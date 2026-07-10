mod compose;
mod drafts;
mod events;
mod html_sanitizer;
mod provider;
mod redownload;
mod send;
mod sync;

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Draft, Email, SaveDraftRequest};

pub use compose::{
    compose_draft, delete_draft, plan_compose, pull_provider_drafts, send_draft, ComposeInput, ComposePlan,
};
pub use drafts::{generate_draft, generate_new_draft, DraftResult, DraftSource};
pub use events::SyncProgress;
pub use html_sanitizer::sanitize_outgoing_html;
pub use provider::build_provider;
pub use redownload::{redownload_email, redownload_empty_emails};
pub use send::{send_new_email, send_new_email_with_provider, send_reply, send_reply_with_provider};
pub use sync::{resync_mailbox_full, sync_account, sync_account_with_provider};

/// List emails for one account, or — when `account_id` is `None` — merged
/// across all enabled accounts (the unified "All accounts" inbox).
pub fn get_emails(
    db: &Arc<Database>,
    account_id: Option<&str>,
    limit: i32,
    offset: i32,
    mailbox: Option<&str>,
    category: Option<&str>,
) -> Result<Vec<Email>> {
    let scope = match account_id {
        Some(id) => crate::db::AccountScope::Account(id),
        None => crate::db::AccountScope::AllEnabled,
    };
    db.get_emails(scope, limit, offset, None, mailbox, category)
}

pub fn get_thread(db: &Arc<Database>, account_id: &str, thread_id: &str) -> Result<Vec<Email>> {
    db.get_thread(account_id, thread_id)
}

pub fn mark_as_read(db: &Arc<Database>, email_id: &str) -> Result<()> {
    db.mark_as_read(email_id)
}

/// Fetch the full body of one email by id. Backs the chat `get_email_body`
/// tool and the redownload flow.
pub fn get_email_body(db: &Arc<Database>, email_id: &str) -> Result<String> {
    db.get_email_body(email_id)
}

/// List the user's saved drafts for an account, newest first. Backs the
/// chat `list_drafts` tool and the existing `list_drafts` command.
pub fn list_drafts(db: &Arc<Database>, account_id: &str) -> Result<Vec<Draft>> {
    db.list_drafts(account_id)
}

/// Insert or upsert a draft row. Backs the chat draft-generation tool
/// (which saves the generated body) and the composer's save action.
pub fn save_draft(db: &Arc<Database>, req: &SaveDraftRequest) -> Result<Draft> {
    db.save_draft(req)
}

/// Low-level mailbox search with explicit filters. Distinct from the
/// higher-level `services::search::search_emails` (which does pattern
/// parsing, AI query parsing, RAG hybrid retrieval, etc.) — this one is the
/// raw FTS+filter path the chat `search_emails` tool needs. Keeps the SQL
/// in `db::emails::search`.
#[allow(clippy::too_many_arguments)]
pub fn search_emails_filtered(
    db: &Arc<Database>,
    account_id: &str,
    query: &str,
    categories: Option<&[String]>,
    from_filter: Option<&str>,
    to_filter: Option<&str>,
    subject_filter: Option<&str>,
    after_timestamp: Option<i64>,
    before_timestamp: Option<i64>,
    tag_filters: Option<&[String]>,
    limit: i32,
    // `true` returns oldest-first — needed to answer "first / primer correo".
    // Default callers pass `false` (newest-first, the historical behaviour).
    ascending: bool,
) -> Result<Vec<Email>> {
    db.search_emails_ordered(
        account_id,
        query,
        categories,
        from_filter,
        to_filter,
        subject_filter,
        after_timestamp,
        before_timestamp,
        tag_filters,
        limit,
        ascending,
    )
}
