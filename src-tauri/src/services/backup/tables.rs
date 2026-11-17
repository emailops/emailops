//! Which tables the local data copy holds.
//!
//! Every table of the migrated schema is listed exactly once: in [`BACKED_UP`]
//! when its rows exist nowhere else (the user typed, chose or confirmed them,
//! or an AI pass that is expensive to repeat produced them), or in
//! [`NOT_BACKED_UP`] with the reason it can be left out. The shadow tables a
//! virtual table keeps for itself are not listed; the virtual table is.
//!
//! Two checks keep the lists honest: `scripts/check_backup_tables.py` (the
//! `backup-tables` gate, run by the pre-commit hook) reads the migrations, and
//! the tests below and in `local.rs` read the migrated schema and make a copy.
//! When either fails after a migration, the `backup-tables` agent
//! (`.claude/agents/backup-tables.md`) classifies the table and updates these
//! lists.

/// A table the local data copy holds, with the rows it leaves out.
#[derive(Debug, Clone, Copy)]
pub struct BackedUp {
    pub table: &'static str,
    /// A SQL condition the copied rows must meet, when only some of them exist
    /// nowhere else. Applied when writing a copy and again when restoring one.
    pub only: Option<&'static str>,
}

/// Why a table stays out of the local data copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Downloaded from the mail or calendar provider; a sync brings it back.
    FromServer,
    /// Derived from other rows on this machine: an index, a cache, a score.
    Recomputed,
    /// Credentials, which belong in the OS keychain and never in a copy.
    Secret,
    /// Bookkeeping of the database itself.
    Internal,
}

const fn all(table: &'static str) -> BackedUp {
    BackedUp { table, only: None }
}

pub const BACKED_UP: &[BackedUp] = &[
    // Accounts and their configuration. Passwords and tokens are in the
    // keychain, not in these rows.
    all("accounts"),
    all("imap_account_settings"),
    all("account_signatures"),
    all("ai_config"),
    all("calendar_feeds"),
    // Every setting except the plain-text API keys of debug builds and the
    // main password's failed-attempt counters.
    BackedUp {
        table: "user_preferences",
        only: Some(
            "key NOT IN ('openrouter_api_key_dev', 'openai_compatible_api_key_dev', \
             'security.main_password_failed_attempts', 'security.main_password_last_failed_at')",
        ),
    },
    // Rules and choices the user made.
    all("classification_rules"),
    all("attachment_rules"),
    all("attachment_rule_suggestions"),
    all("smart_filter_prefs"),
    all("blocked_senders"),
    all("trusted_senders"),
    all("sender_unsubscribes"),
    all("junk_model"),
    // Only the verdicts the user gave; the detector's own scores are recomputed.
    BackedUp {
        table: "email_junk",
        only: Some("user_override IS NOT NULL"),
    },
    // What the AI built and the user curated.
    all("lenses"),
    all("lens_rows"),
    all("lens_exclusions"),
    all("memory_facts"),
    all("pending_tasks"),
    all("interaction_events"),
    all("chat_conversations"),
    all("chat_messages"),
    all("chat_message_sources"),
    // Mail waiting to go out, snoozed threads and this month's AI spend.
    all("outbox"),
    all("thread_snoozes"),
    all("ai_usage"),
    // Shared documents: one never shared, and edits not yet mailed, exist only
    // here.
    all("shared_docs"),
    all("shared_doc_folders"),
    all("shared_doc_participants"),
    all("shared_doc_versions"),
    all("shared_doc_messages"),
    all("shared_doc_tombstones"),
];

pub const NOT_BACKED_UP: &[(&str, Reason)] = &[
    ("emails", Reason::FromServer),
    ("email_bodies", Reason::FromServer),
    ("email_headers", Reason::FromServer),
    ("email_attachment_meta", Reason::FromServer),
    ("folders", Reason::FromServer),
    ("folder_uid_validity", Reason::FromServer),
    ("sync_state", Reason::FromServer),
    ("sync_failed_emails", Reason::FromServer),
    ("calendar_events", Reason::FromServer),
    ("calendar_sync_state", Reason::FromServer),
    // The provider's calendar list; which calendars are shown is the one
    // local column, and a restore leaves them all visible.
    ("calendars", Reason::FromServer),
    // Drafts are pushed to the provider's Drafts folder; their attachments
    // are files outside the database.
    ("drafts", Reason::FromServer),
    ("draft_attachments", Reason::FromServer),
    ("draft_tombstones", Reason::FromServer),
    ("email_tags", Reason::Recomputed),
    ("email_extraction_status", Reason::Recomputed),
    ("embedding_chunks", Reason::Recomputed),
    ("memory_fact_chunks", Reason::Recomputed),
    ("tag_priority", Reason::Recomputed),
    ("thread_states", Reason::Recomputed),
    ("smart_filter_suggestions", Reason::Recomputed),
    // Files the attachment rules saved; the rules are backed up.
    ("attachments", Reason::Recomputed),
    ("lens_runs", Reason::Recomputed),
    ("help_doc_chunks", Reason::Recomputed),
    ("emails_fts", Reason::Recomputed),
    ("memory_facts_fts", Reason::Recomputed),
    ("shared_docs_fts", Reason::Recomputed),
    ("help_docs_fts", Reason::Recomputed),
    ("vec_emails", Reason::Recomputed),
    ("vec_memory_facts", Reason::Recomputed),
    ("vec_help_docs", Reason::Recomputed),
    ("dev_tokens", Reason::Secret),
    ("dev_imap_creds", Reason::Secret),
    ("refinery_schema_history", Reason::Internal),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::collections::BTreeSet;

    /// Tables of the migrated schema, without SQLite's own and without the
    /// shadow tables a virtual table creates for itself.
    fn schema_tables(db: &Database) -> BTreeSet<String> {
        let conn = db.reader();
        let mut stmt = conn
            .prepare(
                "SELECT name, COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .expect("prepare");
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<std::result::Result<_, _>>()
            .expect("rows");
        let virtual_tables: Vec<&str> = rows
            .iter()
            .filter(|(_, sql)| sql.to_ascii_uppercase().starts_with("CREATE VIRTUAL TABLE"))
            .map(|(name, _)| name.as_str())
            .collect();
        rows.iter()
            .map(|(name, _)| name)
            .filter(|name| {
                !virtual_tables
                    .iter()
                    .any(|v| name.as_str() != *v && name.starts_with(&format!("{v}_")))
            })
            .cloned()
            .collect()
    }

    #[test]
    fn every_table_of_the_schema_is_listed_once() {
        let db = Database::new_for_testing().expect("test db");
        let schema = schema_tables(&db);
        let listed: Vec<&str> = BACKED_UP
            .iter()
            .map(|b| b.table)
            .chain(NOT_BACKED_UP.iter().map(|(t, _)| *t))
            .collect();
        let listed_set: BTreeSet<String> = listed.iter().map(|t| (*t).to_string()).collect();

        let unlisted: Vec<&String> = schema.difference(&listed_set).collect();
        let gone: Vec<&String> = listed_set.difference(&schema).collect();
        assert!(
            unlisted.is_empty() && gone.is_empty(),
            "services/backup/tables.rs is out of date with the migrations. \
             Not listed: {unlisted:?}. Listed but no longer in the schema: {gone:?}. \
             Run the backup-tables agent (.claude/agents/backup-tables.md) to classify them."
        );
        assert_eq!(listed.len(), listed_set.len(), "a table is listed twice");
    }

    #[test]
    fn the_preferences_filter_names_every_secret_preference() {
        let filter = BACKED_UP
            .iter()
            .find(|b| b.table == "user_preferences")
            .and_then(|b| b.only)
            .expect("user_preferences is backed up with a filter");
        for key in [
            crate::services::ai::OPENROUTER_DEV_KEY_PREF,
            crate::services::ai_server::DEV_KEY_PREF,
            crate::services::password::FAILED_ATTEMPTS_KEY,
            crate::services::password::LAST_FAILED_AT_KEY,
        ] {
            assert!(filter.contains(&format!("'{key}'")), "{key} would be copied");
        }
    }
}
