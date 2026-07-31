//! Ambient per-user context.
//!
//! ## The problem
//!
//! `services::events`, `services::logger` and `services::keychain` each resolve their
//! backend from a **process-global** registry (`LazyLock<RwLock<Arc<dyn …>>>`). That is
//! exactly right for a desktop app — one process, one user — and exactly wrong for a
//! server, where a chat token streamed by user A must not land in user B's browser.
//!
//! ## Why not thread a `&UserCtx` through every call?
//!
//! Because `events::emit` / `logger::log` / `keychain::current` are called from several
//! hundred places across `services/`, `sync/` and `db/`, most of them deep inside
//! functions whose signatures would all have to change. That refactor is desirable in
//! the long run but it is not what makes a server possible, and doing it first would
//! mean a many-thousand-line diff before anything runs.
//!
//! ## What this does instead
//!
//! A `tokio::task_local!` holds the current `UserCtx`. The three seam modules gain a
//! two-line change: consult the task-local first, fall back to the process global.
//!
//! * **Desktop / CLI / existing tests** never install a context, always hit the
//!   fallback, and behave exactly as before. That is what keeps the desktop shipping
//!   while this work lands.
//! * **A server** wraps every request in [`scope`], so the same call sites resolve that
//!   request's user.
//!
//! ## The failure mode, and how it is handled
//!
//! Task-locals do not survive a bare `tokio::spawn` — the spawned task starts with an
//! empty task-local set and would silently fall through to the process global. In a
//! multi-user process that is a data leak, not a glitch.
//!
//! Two mitigations, and neither is optional:
//!
//! 1. [`spawn`] and [`spawn_blocking`] in `runtime::spawn` capture the current context
//!    and re-enter it inside the new task. Use them instead of the tokio originals.
//! 2. A server installs *poisoned* backends into the process globals (see
//!    [`install_server_fallback_guards`]), so any code that does reach the fallback
//!    fails loudly instead of writing into whichever user happened to be there.
//!
//! Mitigation 2 is the important one: it converts "silently serves the wrong user"
//! into "obvious, reported error".

use std::future::Future;
use std::sync::Arc;

use crate::runtime::core::AppCore;
use crate::services::events::EventSink;
use crate::services::keychain::Keychain;
use crate::services::logger::Logger;

/// Identifies the signed-in user a task is running on behalf of.
///
/// Opaque on purpose: the desktop has no user id, the server uses a UUID string, and
/// nothing in `services/` should care which.
pub type UserId = String;

/// Everything a request needs in order to act as one specific user.
pub struct UserCtx {
    pub user_id: UserId,
    /// That user's mailbox: database handle, data dir, task queues.
    pub core: Arc<AppCore>,
    /// Where this user's UI events go (one browser session, or a fan-out to several).
    pub sink: Arc<dyn EventSink>,
    /// Where this user's `app-log` lines go.
    pub logger: Arc<dyn Logger>,
    /// This user's secrets (OAuth tokens, IMAP passwords).
    pub keychain: Arc<dyn Keychain>,
}

impl UserCtx {
    pub fn new(
        user_id: impl Into<UserId>,
        core: Arc<AppCore>,
        sink: Arc<dyn EventSink>,
        logger: Arc<dyn Logger>,
        keychain: Arc<dyn Keychain>,
    ) -> Arc<Self> {
        Arc::new(Self {
            user_id: user_id.into(),
            core,
            sink,
            logger,
            keychain,
        })
    }
}

tokio::task_local! {
    static CTX: Arc<UserCtx>;
}

/// The context for the running task, if one was installed.
///
/// `None` on the desktop, in the CLI, and in every test that has not opted in — which
/// is what makes the fallback path in the seam modules the normal path there.
pub fn try_current() -> Option<Arc<UserCtx>> {
    CTX.try_with(Arc::clone).ok()
}

/// Run `f` with `ctx` installed as the ambient context.
///
/// Everything awaited inside — including nested calls into `services/` — resolves this
/// user's sink, logger, keychain and database.
pub async fn scope<F>(ctx: Arc<UserCtx>, f: F) -> F::Output
where
    F: Future,
{
    CTX.scope(ctx, f).await
}

/// Convenience: the current user's `AppCore`, if a context is installed.
pub fn try_core() -> Option<Arc<AppCore>> {
    try_current().map(|c| c.core.clone())
}

/// Convenience: the current user's id, if a context is installed.
pub fn try_user_id() -> Option<UserId> {
    try_current().map(|c| c.user_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::services::events::VecEventSink;
    use crate::services::keychain::InMemoryKeychain;
    use crate::services::logger::VecLogger;

    fn ctx_for(user: &str) -> Arc<UserCtx> {
        #[allow(clippy::expect_used)] // test-only: an in-memory DB cannot fail to open
        let db = Arc::new(Database::new_for_testing().expect("in-memory db"));
        UserCtx::new(
            user,
            Arc::new(AppCore::for_testing(db)),
            Arc::new(VecEventSink::new()),
            Arc::new(VecLogger::new()),
            Arc::new(InMemoryKeychain::new()),
        )
    }

    #[tokio::test]
    async fn no_context_by_default() {
        assert!(try_current().is_none(), "desktop/CLI/tests must see no ambient context");
    }

    #[tokio::test]
    async fn scope_installs_the_context() {
        scope(ctx_for("alice"), async {
            assert_eq!(try_user_id().as_deref(), Some("alice"));
        })
        .await;
    }

    #[tokio::test]
    async fn context_is_removed_after_the_scope_ends() {
        scope(ctx_for("alice"), async {}).await;
        assert!(try_current().is_none());
    }

    #[tokio::test]
    async fn concurrent_scopes_do_not_bleed_into_each_other() {
        // The whole point of the design: two users in one process, one runtime.
        let a = scope(ctx_for("alice"), async {
            tokio::task::yield_now().await;
            try_user_id()
        });
        let b = scope(ctx_for("bob"), async {
            tokio::task::yield_now().await;
            try_user_id()
        });
        let (a, b) = tokio::join!(a, b);
        assert_eq!(a.as_deref(), Some("alice"));
        assert_eq!(b.as_deref(), Some("bob"));
    }

    #[tokio::test]
    async fn nested_scopes_shadow_the_outer_one() {
        scope(ctx_for("alice"), async {
            scope(ctx_for("bob"), async {
                assert_eq!(try_user_id().as_deref(), Some("bob"));
            })
            .await;
            assert_eq!(try_user_id().as_deref(), Some("alice"));
        })
        .await;
    }

    #[tokio::test]
    async fn each_context_carries_its_own_core() {
        let alice = ctx_for("alice");
        let bob = ctx_for("bob");
        assert!(
            !Arc::ptr_eq(&alice.core, &bob.core),
            "two users must not share a mailbox handle"
        );
    }
}
