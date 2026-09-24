mod compose;
mod drafts;
mod events;
mod folders;
mod html_sanitizer;
mod mailbox_state;
mod optimistic;
mod provider;
mod reconcile;
mod redownload;
mod send;
mod sync;

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Draft, Email, SaveDraftRequest};

pub use compose::{
    compose_draft, delete_draft, plan_compose, pull_provider_drafts, refresh_provider_drafts, send_draft, ComposeInput,
    ComposePlan,
};
pub use drafts::{generate_draft, generate_new_draft, DraftResult, DraftSource};
pub use events::SyncProgress;
pub use folders::{create_folder, delete_folder, move_email, rename_folder};
pub use html_sanitizer::sanitize_outgoing_html;
pub use mailbox_state::{delete_email, delete_email_with_provider, mark_as_read, mark_as_read_with_provider};
pub use provider::build_provider;
pub use redownload::{redownload_email, redownload_empty_emails};
pub use send::{send_new_email, send_new_email_with_provider, send_reply, send_reply_with_provider};
pub use sync::{
    request_extra_mailbox_backfill_reset, request_sync_abort, resync_mailbox_full, sync_account,
    sync_account_with_contention, sync_account_with_provider, SyncContention,
};

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

/// List an account's custom folders for the sidebar. Well-known role folders
/// (Sent/Spam/Trash) are excluded — they already have dedicated views.
pub fn get_folders(db: &Arc<Database>, account_id: &str) -> Result<Vec<crate::models::Folder>> {
    db.list_folders(account_id, Some(crate::models::FolderRole::Custom))
}

pub fn get_thread(db: &Arc<Database>, account_id: &str, thread_id: &str) -> Result<Vec<Email>> {
    db.get_thread(account_id, thread_id)
}

/// Message count per `(account_id, thread_id)`. Backs the `messages=N` field
/// on chat search rows.
pub fn thread_sizes(
    db: &Arc<Database>,
    threads: &[(&str, &str)],
) -> Result<std::collections::HashMap<(String, String), i64>> {
    db.thread_sizes(threads)
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

/// Most addresses a person's name resolves to. A short name can sit inside
/// others' ("Ana" in "Mariana"): the most frequent senders come first.
const MAX_PARTICIPANT_ADDRESSES: usize = 5;

/// What "emails with X" searches for: X itself, plus — when X is a name —
/// the addresses X has written from, so mail the user sent to those
/// addresses (which rarely carries the name) is found too. An address is
/// searched as it is. Lowercased, without repeats. Pure.
pub fn participant_terms(with: &str, addresses: Vec<String>) -> Vec<String> {
    let with = with.trim().to_lowercase();
    if with.is_empty() {
        return Vec::new();
    }
    if with.contains('@') {
        return vec![with];
    }
    let mut terms = vec![with];
    for address in addresses {
        let address = address.trim().to_lowercase();
        if !address.is_empty() && !terms.contains(&address) {
            terms.push(address);
        }
    }
    terms
}

/// [`participant_terms`] for `with` in this account's mailbox. A failed
/// lookup is logged and falls back to the name alone.
pub fn resolve_participant(db: &Database, account_id: &str, with: &str) -> Vec<String> {
    let addresses = if with.contains('@') {
        Vec::new()
    } else {
        db.sender_addresses_matching(account_id, with, MAX_PARTICIPANT_ADDRESSES)
            .unwrap_or_else(|e| {
                crate::services::logger::log(
                    "warn",
                    "chat",
                    format!("resolving the addresses of a participant failed: {e}"),
                );
                Vec::new()
            })
    };
    participant_terms(with, addresses)
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
    tag_filters: Option<&[crate::db::emails::search::TagQuery]>,
    limit: i32,
    // `true` returns oldest-first — needed to answer "first / primer correo".
    // Default callers pass `false` (newest-first, the historical behaviour).
    ascending: bool,
    // `true` keeps only mail the user has not read (filtered in SQL).
    unread_only: bool,
    // "Emails exchanged with X": the person's name plus the addresses it
    // resolves to; see `Database::search_emails_ordered`.
    participants: Option<&[String]>,
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
        // Chat never wants spam or phishing in its results.
        true,
        unread_only,
        participants,
    )
}

#[cfg(test)]
mod participant_tests {
    use super::participant_terms;

    #[test]
    fn a_name_is_searched_with_the_addresses_it_writes_from() {
        assert_eq!(
            participant_terms("Ana", vec!["ar@client.example".into(), "ana@home.example".into()]),
            vec!["ana", "ar@client.example", "ana@home.example"]
        );
    }

    #[test]
    fn an_address_is_searched_as_it_is() {
        assert_eq!(
            participant_terms(" AR@Client.example ", vec!["other@x.example".into()]),
            vec!["ar@client.example"]
        );
    }
}
