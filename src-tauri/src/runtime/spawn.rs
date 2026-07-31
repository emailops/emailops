//! Context-preserving replacements for `tokio::spawn` / `spawn_blocking`.
//!
//! A task-local does **not** cross a spawn boundary: `tokio::spawn(async { … })` starts
//! with an empty task-local set, so `runtime::ctx::try_current()` inside it returns
//! `None` and the seam modules fall through to the process-global backend. On the
//! desktop that is harmless. In a server it means a background job emits into whatever
//! sink was installed globally — i.e. potentially another user's browser.
//!
//! These wrappers capture the ambient [`UserCtx`] at spawn time and re-enter it inside
//! the new task. When no context is installed (desktop, CLI, tests) they degrade to a
//! plain `tokio::spawn`, so there is no behavioural change on those paths.
//!
//! **Use these instead of the tokio originals anywhere under `services/`.**

use std::future::Future;

use crate::runtime::ctx;

/// `tokio::spawn`, preserving the ambient user context.
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match ctx::try_current() {
        Some(cx) => tokio::spawn(async move { ctx::scope(cx, future).await }),
        None => tokio::spawn(future),
    }
}

/// `tokio::task::spawn_blocking`, preserving the ambient user context.
///
/// The closure runs on a blocking thread, where async task-locals are not available,
/// so the context is re-entered around it via a current-thread block-on only when one
/// is present. Blocking work that needs the context should therefore keep its use of
/// it short.
pub fn spawn_blocking<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match ctx::try_current() {
        Some(cx) => tokio::task::spawn_blocking(move || {
            // Re-establish the task-local for the duration of the blocking closure so
            // nested `events::emit` / `logger::log` calls resolve the right user.
            let handle = tokio::runtime::Handle::try_current();
            match handle {
                Ok(h) => h.block_on(ctx::scope(cx, async move { f() })),
                // No reactor (shouldn't happen from spawn_blocking, but never panic
                // on a background path): run without the context rather than abort.
                Err(_) => f(),
            }
        }),
        None => tokio::task::spawn_blocking(f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::runtime::core::AppCore;
    use crate::runtime::ctx::UserCtx;
    use crate::services::events::VecEventSink;
    use crate::services::keychain::InMemoryKeychain;
    use crate::services::logger::VecLogger;
    use std::sync::Arc;

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
    async fn spawn_propagates_the_context() {
        let seen = ctx::scope(ctx_for("alice"), async {
            spawn(async { ctx::try_user_id() }).await.unwrap_or_default()
        })
        .await;
        assert_eq!(seen.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn raw_tokio_spawn_would_lose_it() {
        // Documents precisely why the wrapper exists — if this ever starts returning
        // Some, tokio changed its semantics and the wrappers can be reconsidered.
        let seen = ctx::scope(ctx_for("alice"), async {
            tokio::spawn(async { ctx::try_user_id() }).await.unwrap_or_default()
        })
        .await;
        assert_eq!(seen, None);
    }

    #[tokio::test]
    async fn spawn_without_a_context_still_works() {
        let seen = spawn(async { ctx::try_user_id() }).await.unwrap_or_default();
        assert_eq!(seen, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_blocking_propagates_the_context() {
        let seen = ctx::scope(ctx_for("bob"), async {
            spawn_blocking(ctx::try_user_id).await.unwrap_or_default()
        })
        .await;
        assert_eq!(seen.as_deref(), Some("bob"));
    }

    #[tokio::test]
    async fn concurrent_spawns_keep_their_own_user() {
        let a = ctx::scope(ctx_for("alice"), async {
            spawn(async {
                tokio::task::yield_now().await;
                ctx::try_user_id()
            })
            .await
            .unwrap_or_default()
        });
        let b = ctx::scope(ctx_for("bob"), async {
            spawn(async {
                tokio::task::yield_now().await;
                ctx::try_user_id()
            })
            .await
            .unwrap_or_default()
        });
        let (a, b) = tokio::join!(a, b);
        assert_eq!(a.as_deref(), Some("alice"));
        assert_eq!(b.as_deref(), Some("bob"));
    }
}
