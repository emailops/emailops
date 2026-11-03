use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::error::Result;
use crate::models::{Draft, DraftAttachment, ProviderDraft, SaveDraftRequest};

use super::Database;

/// Explicit column list shared by every draft SELECT so ordinal reads in
/// `row_to_draft` stay in lockstep. `attachments` is filled separately.
const DRAFT_COLUMNS: &str = "id, email_id, account_id, to_addresses_json, cc_addresses_json, \
     subject, body, body_html, ai_generated, status, provider_draft_id, created_at, updated_at";

fn row_to_draft(row: &rusqlite::Row<'_>) -> rusqlite::Result<Draft> {
    let to_json: String = row.get(3)?;
    let to_addresses: Vec<String> = serde_json::from_str(&to_json).unwrap_or_default();
    let cc_json: String = row.get(4)?;
    let cc_addresses: Vec<String> = serde_json::from_str(&cc_json).unwrap_or_default();
    Ok(Draft {
        id: row.get(0)?,
        email_id: row.get(1)?,
        account_id: row.get(2)?,
        to_addresses,
        cc_addresses,
        subject: row.get(5)?,
        body: row.get(6)?,
        body_html: row.get(7)?,
        ai_generated: row.get::<_, i64>(8)? != 0,
        status: row.get(9)?,
        provider_draft_id: row.get(10)?,
        attachments: Vec::new(),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn attachments_for(conn: &Connection, draft_id: &str) -> rusqlite::Result<Vec<DraftAttachment>> {
    let mut stmt = conn.prepare(
        "SELECT id, draft_id, file_path, filename, mime_type
         FROM draft_attachments WHERE draft_id = ?1 ORDER BY filename",
    )?;
    let rows = stmt
        .query_map(params![draft_id], |row| {
            Ok(DraftAttachment {
                id: row.get(0)?,
                draft_id: row.get(1)?,
                file_path: row.get(2)?,
                filename: row.get(3)?,
                mime_type: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Swap a draft's attachment rows for `attachments` and return the file paths
/// it no longer references. The row `id` is (re)generated here so callers don't
/// have to supply one.
fn swap_attachments(
    conn: &Connection,
    draft_id: &str,
    attachments: &[DraftAttachment],
) -> rusqlite::Result<Vec<String>> {
    let dropped = attachments_for(conn, draft_id)?
        .into_iter()
        .map(|att| att.file_path)
        .filter(|path| !attachments.iter().any(|att| att.file_path == *path))
        .collect();
    conn.execute("DELETE FROM draft_attachments WHERE draft_id = ?1", params![draft_id])?;
    for att in attachments {
        conn.execute(
            "INSERT INTO draft_attachments (id, draft_id, file_path, filename, mime_type)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                uuid::Uuid::new_v4().to_string(),
                draft_id,
                att.file_path,
                att.filename,
                att.mime_type,
            ],
        )?;
    }
    Ok(dropped)
}

/// What [`Database::store_pulled_draft`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPulledDraft {
    pub id: String,
    /// False when the row is dirty: the local draft wins and nothing changed.
    pub applied: bool,
    /// Files the draft no longer references.
    pub dropped_files: Vec<String>,
}

/// What [`Database::prune_provider_drafts`] removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedDrafts {
    pub removed: usize,
    /// Files only the removed drafts referenced.
    pub dropped_files: Vec<String>,
}

/// One local draft as the provider sync sees it. Feeds
/// `sync::draft_plan::plan_draft_sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSyncState {
    pub id: String,
    pub provider_draft_id: Option<String>,
    /// The row holds user edits the provider has not received.
    pub dirty: bool,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Write a pulled draft's row, unless it is dirty. Returns its local id and
/// whether it was written.
fn upsert_pulled_row(
    conn: &Connection,
    account_id: &str,
    draft: &ProviderDraft,
    record_token: bool,
) -> Result<(String, bool)> {
    let now = now_secs();
    // Keep the provider's own modification time so a pulled draft doesn't
    // jump to "today" on every sync; fall back to now for providers that
    // don't report one.
    let updated_at = draft.updated_at.unwrap_or(now);
    let to_json = serde_json::to_string(&draft.to_addresses).unwrap_or_else(|_| "[]".to_string());
    let cc_json = serde_json::to_string(&draft.cc_addresses).unwrap_or_else(|_| "[]".to_string());
    let change_token = draft.provider_message_id.as_deref().filter(|_| record_token);

    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM drafts WHERE account_id = ?1 AND provider_draft_id = ?2",
            params![account_id, draft.provider_draft_id],
            |row| row.get(0),
        )
        .optional()?;

    let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let written = conn.execute(
        // A reply draft is linked back to the email it answers by the
        // Message-ID its provider copy carries, so one re-imported from the
        // provider still belongs to its thread. An existing link is kept.
        "INSERT INTO drafts (id, email_id, account_id, to_addresses_json, cc_addresses_json,
                             subject, body, body_html, ai_generated, status,
                             provider_draft_id, provider_message_id, created_at, updated_at)
         VALUES (?1,
                 (SELECT id FROM emails WHERE account_id = ?2
                    AND message_id IN (?12, trim(?12, '<>')) LIMIT 1),
                 ?2, ?3, ?4, ?5, ?6, ?7, 0, 'draft', ?8, ?9,
                 COALESCE((SELECT created_at FROM drafts WHERE id = ?1), ?10), ?11)
         ON CONFLICT(id) DO UPDATE SET
            email_id = COALESCE(drafts.email_id, excluded.email_id),
            to_addresses_json = excluded.to_addresses_json,
            cc_addresses_json = excluded.cc_addresses_json,
            subject = excluded.subject,
            body = excluded.body,
            body_html = excluded.body_html,
            provider_draft_id = excluded.provider_draft_id,
            provider_message_id = excluded.provider_message_id,
            updated_at = excluded.updated_at
         WHERE drafts.dirty = 0",
        params![
            id,
            account_id,
            to_json,
            cc_json,
            draft.subject,
            draft.body,
            draft.body_html,
            draft.provider_draft_id,
            change_token,
            now,
            updated_at,
            draft.in_reply_to,
        ],
    )?;
    Ok((id, written > 0))
}

impl Database {
    pub fn list_drafts(&self, account_id: &str) -> Result<Vec<Draft>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DRAFT_COLUMNS} FROM drafts
             WHERE account_id = ?1 AND status = 'draft'
             ORDER BY updated_at DESC"
        ))?;
        let mut drafts = stmt
            .query_map(params![account_id], row_to_draft)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for draft in &mut drafts {
            draft.attachments = attachments_for(&conn, &draft.id)?;
        }
        Ok(drafts)
    }

    /// Fetch a single draft by id, regardless of `status`. Used by the CLI to
    /// surface a draft the chat assistant just created (linked via the message's
    /// `referenced_draft_ids`). Returns `None` when no such draft exists.
    pub fn get_draft(&self, draft_id: &str) -> Result<Option<Draft>> {
        let conn = self.reader();
        let draft = conn
            .query_row(
                &format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"),
                params![draft_id],
                row_to_draft,
            )
            .optional()?;
        match draft {
            Some(mut d) => {
                d.attachments = attachments_for(&conn, &d.id)?;
                Ok(Some(d))
            }
            None => Ok(None),
        }
    }

    /// Insert or upsert a draft row without touching its `dirty` marker. For
    /// writes that are not the user editing a draft the provider should
    /// receive (the chat draft tool, fixtures); the composer's save goes
    /// through [`Self::save_user_draft`].
    pub fn save_draft(&self, req: &SaveDraftRequest) -> Result<Draft> {
        self.save_draft_row(req, false)
    }

    /// Save a draft the user edited and, in the same statement, mark it dirty:
    /// it now holds text the provider has not received, so the sync must push
    /// it and must never prune or overwrite it until that push succeeds.
    pub fn save_user_draft(&self, req: &SaveDraftRequest) -> Result<Draft> {
        self.save_draft_row(req, true)
    }

    fn save_draft_row(&self, req: &SaveDraftRequest, mark_dirty: bool) -> Result<Draft> {
        let conn = self.connection();
        let now = now_secs();

        let id = req.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let to_json = serde_json::to_string(&req.to_addresses).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(&req.cc_addresses).unwrap_or_else(|_| "[]".to_string());

        conn.execute(
            "INSERT INTO drafts (id, email_id, account_id, to_addresses_json, cc_addresses_json,
                                 subject, body, body_html, ai_generated, status,
                                 provider_draft_id, created_at, updated_at, dirty)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 'draft', ?9,
                     COALESCE((SELECT created_at FROM drafts WHERE id = ?1), ?10), ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                email_id = excluded.email_id,
                to_addresses_json = excluded.to_addresses_json,
                cc_addresses_json = excluded.cc_addresses_json,
                subject = excluded.subject,
                body = excluded.body,
                body_html = excluded.body_html,
                -- keep an existing provider link when the save omits one
                provider_draft_id = COALESCE(excluded.provider_draft_id, drafts.provider_draft_id),
                updated_at = excluded.updated_at,
                -- one more unpushed save; a plain save leaves the count alone
                dirty = drafts.dirty + excluded.dirty",
            params![
                id,
                req.email_id,
                req.account_id,
                to_json,
                cc_json,
                req.subject,
                req.body,
                req.body_html,
                req.provider_draft_id,
                now,
                i64::from(mark_dirty),
            ],
        )?;

        // Read back through the same write connection — avoids holding the write
        // lock while also trying to acquire a read-pool slot (deadlock in test
        // mode where read_conns is empty and reader() falls back to write_conn).
        let mut draft = conn.query_row(
            &format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"),
            params![id],
            row_to_draft,
        )?;
        draft.attachments = attachments_for(&conn, &draft.id)?;
        Ok(draft)
    }

    /// A draft together with its `dirty` revision, read in one go so a push
    /// knows exactly which revision of the content it is sending. `None` when
    /// the draft no longer exists.
    pub fn draft_for_push(&self, draft_id: &str) -> Result<Option<(Draft, i64)>> {
        let conn = self.connection();
        let found = conn
            .query_row(
                &format!("SELECT {DRAFT_COLUMNS}, dirty FROM drafts WHERE id = ?1"),
                params![draft_id],
                |row| Ok((row_to_draft(row)?, row.get::<_, i64>(13)?)),
            )
            .optional()?;
        match found {
            Some((mut draft, dirty)) => {
                draft.attachments = attachments_for(&conn, &draft.id)?;
                Ok(Some((draft, dirty)))
            }
            None => Ok(None),
        }
    }

    /// Record a successful push: link the local draft to `provider_draft_id`
    /// (replacing a stale id when the draft was re-created upstream) and mark
    /// it clean — but only if it is still at `pushed_revision`. A save that
    /// landed while the push was in flight keeps the row dirty, so the next
    /// sync pushes that newer text. The stored change token is dropped: the
    /// push minted a new one upstream that this row has not seen.
    pub fn mark_draft_pushed(&self, draft_id: &str, provider_draft_id: &str, pushed_revision: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE drafts SET provider_draft_id = ?2, provider_message_id = NULL,
                    dirty = CASE WHEN dirty = ?3 THEN 0 ELSE dirty END
             WHERE id = ?1",
            params![draft_id, provider_draft_id, pushed_revision],
        )?;
        Ok(())
    }

    /// Every draft of an account as the provider sync needs to see it.
    pub fn draft_sync_states(&self, account_id: &str) -> Result<Vec<DraftSyncState>> {
        let conn = self.reader();
        let mut stmt =
            conn.prepare("SELECT id, provider_draft_id, dirty FROM drafts WHERE account_id = ?1 ORDER BY id")?;
        let rows = stmt
            .query_map(params![account_id], |row| {
                Ok(DraftSyncState {
                    id: row.get(0)?,
                    provider_draft_id: row.get(1)?,
                    dirty: row.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Replace the attachment set for a draft in one transaction. `attachments`
    /// carry pre-resolved filename/mime. Returns the file paths the draft no
    /// longer references.
    pub fn replace_draft_attachments(&self, draft_id: &str, attachments: &[DraftAttachment]) -> Result<Vec<String>> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let dropped = swap_attachments(&tx, draft_id, attachments)?;
        tx.commit()?;
        Ok(dropped)
    }

    pub fn list_draft_attachments(&self, draft_id: &str) -> Result<Vec<DraftAttachment>> {
        let conn = self.reader();
        Ok(attachments_for(&conn, draft_id)?)
    }

    /// Store a draft pulled from the provider and its attachments, keyed by
    /// `(account_id, provider_draft_id)`. Updates an existing local row in
    /// place (preserving its local id and `created_at`) or inserts a new one.
    ///
    /// A dirty row is left exactly as it is, attachments included: it holds
    /// text the provider has not received, and the local draft wins that
    /// conflict. The guard lives in the statement, and the attachments go in
    /// the same transaction, so a save racing the pull cannot be overwritten.
    ///
    /// `complete` says every file the draft can hold was stored. Otherwise the
    /// change token is not recorded, so the next pull reads the draft again.
    pub fn store_pulled_draft(
        &self,
        account_id: &str,
        draft: &ProviderDraft,
        attachments: &[DraftAttachment],
        complete: bool,
    ) -> Result<StoredPulledDraft> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let (id, applied) = upsert_pulled_row(&tx, account_id, draft, complete)?;
        let dropped_files = if applied {
            swap_attachments(&tx, &id, attachments)?
        } else {
            Vec::new()
        };
        tx.commit()?;
        Ok(StoredPulledDraft {
            id,
            applied,
            dropped_files,
        })
    }

    /// [`Self::store_pulled_draft`] for a draft whose attachments do not matter
    /// to the test.
    #[cfg(test)]
    pub fn upsert_provider_draft(&self, account_id: &str, draft: &ProviderDraft) -> Result<String> {
        let conn = self.connection();
        Ok(upsert_pulled_row(&conn, account_id, draft, true)?.0)
    }

    /// Change tokens for this account's provider-linked drafts, keyed by
    /// provider draft id. Feeds `plan_draft_fetches` so the pull pass can skip
    /// the full content read for drafts that have not changed upstream. Drafts
    /// with no stored token are omitted, which the planner reads as "fetch".
    pub fn provider_draft_change_tokens(&self, account_id: &str) -> Result<HashMap<String, String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT provider_draft_id, provider_message_id FROM drafts
             WHERE account_id = ?1 AND provider_draft_id IS NOT NULL AND provider_message_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![account_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<HashMap<_, _>, _>>()?;
        Ok(rows)
    }

    /// Delete the given local drafts of an account — the ones the sync planned
    /// to prune because their provider copy was sent or deleted elsewhere.
    /// Only clean, provider-linked rows go: a draft that turned dirty since it
    /// was planned holds unpushed text and is kept, and local-only drafts are
    /// never touched.
    pub fn prune_provider_drafts(&self, account_id: &str, draft_ids: &[String]) -> Result<PrunedDrafts> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        let mut dropped_files = Vec::new();
        for draft_id in draft_ids {
            let attachments = attachments_for(&tx, draft_id)?;
            let deleted = tx.execute(
                "DELETE FROM drafts
                 WHERE id = ?1 AND account_id = ?2 AND provider_draft_id IS NOT NULL AND dirty = 0",
                params![draft_id, account_id],
            )?;
            if deleted > 0 {
                removed += deleted;
                dropped_files.extend(attachments.into_iter().map(|att| att.file_path));
            }
        }
        tx.commit()?;
        Ok(PrunedDrafts { removed, dropped_files })
    }

    /// Delete a draft. Returns the file paths its attachments pointed at.
    pub fn delete_draft(&self, draft_id: &str, account_id: &str) -> Result<Vec<String>> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let attachments = attachments_for(&tx, draft_id)?;
        let deleted = tx.execute(
            "DELETE FROM drafts WHERE id = ?1 AND account_id = ?2",
            params![draft_id, account_id],
        )?;
        tx.commit()?;
        Ok(if deleted > 0 {
            attachments.into_iter().map(|att| att.file_path).collect()
        } else {
            Vec::new()
        })
    }

    /// Remember a provider draft whose copy could not be removed.
    pub fn add_draft_tombstone(&self, account_id: &str, provider_draft_id: &str) -> Result<()> {
        self.connection().execute(
            "INSERT OR REPLACE INTO draft_tombstones (account_id, provider_draft_id, deleted_at)
             VALUES (?1, ?2, ?3)",
            params![account_id, provider_draft_id, now_secs()],
        )?;
        Ok(())
    }

    /// The provider draft ids of this account still waiting to be removed.
    pub fn draft_tombstones(&self, account_id: &str) -> Result<HashSet<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare("SELECT provider_draft_id FROM draft_tombstones WHERE account_id = ?1")?;
        let ids = stmt
            .query_map(params![account_id], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?;
        Ok(ids)
    }

    pub fn remove_draft_tombstone(&self, account_id: &str, provider_draft_id: &str) -> Result<()> {
        self.connection().execute(
            "DELETE FROM draft_tombstones WHERE account_id = ?1 AND provider_draft_id = ?2",
            params![account_id, provider_draft_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DraftAttachmentInput;

    fn seed_account(db: &Database, id: &str) {
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, 'gmail', ?2, ?2, 0, 0, 1)",
                params![id, format!("{id}@example.com")],
            )
            .expect("seed account");
    }

    fn save(db: &Database, id: &str) -> Draft {
        seed_account(db, "acct-1");
        db.save_draft(&SaveDraftRequest {
            id: Some(id.to_string()),
            email_id: None,
            account_id: "acct-1".to_string(),
            to_addresses: vec!["alina@example.com".to_string()],
            cc_addresses: vec!["cc@example.com".to_string()],
            subject: "Confirmar reunión".to_string(),
            body: "Hola Alina, confirmo.".to_string(),
            body_html: Some("<p>Hola Alina, confirmo.</p>".to_string()),
            provider_draft_id: None,
            attachments: None,
        })
        .expect("save draft")
    }

    #[test]
    fn get_draft_round_trips_saved_fields() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "draft-1");

        let got = db.get_draft("draft-1").expect("get_draft ok").expect("draft present");
        assert_eq!(got.id, "draft-1");
        assert_eq!(got.account_id, "acct-1");
        assert_eq!(got.to_addresses, vec!["alina@example.com".to_string()]);
        assert_eq!(got.cc_addresses, vec!["cc@example.com".to_string()]);
        assert_eq!(got.subject, "Confirmar reunión");
        assert_eq!(got.body, "Hola Alina, confirmo.");
        assert_eq!(got.body_html.as_deref(), Some("<p>Hola Alina, confirmo.</p>"));
        assert!(got.provider_draft_id.is_none());
    }

    #[test]
    fn get_draft_unknown_id_is_none() {
        let db = Database::new_for_testing().expect("test db");
        assert!(db.get_draft("ghost").expect("get_draft ok").is_none());
    }

    #[test]
    fn mark_draft_pushed_links_and_save_preserves_it() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "draft-1");
        db.mark_draft_pushed("draft-1", "gmail-draft-42", 0).expect("set link");

        let got = db.get_draft("draft-1").expect("ok").expect("present");
        assert_eq!(got.provider_draft_id.as_deref(), Some("gmail-draft-42"));

        // A subsequent auto-save that omits the provider id must not wipe it.
        db.save_draft(&SaveDraftRequest {
            id: Some("draft-1".to_string()),
            email_id: None,
            account_id: "acct-1".to_string(),
            to_addresses: vec!["alina@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Edited".to_string(),
            body: "Edited body".to_string(),
            body_html: None,
            provider_draft_id: None,
            attachments: None,
        })
        .expect("re-save");
        let after = db.get_draft("draft-1").expect("ok").expect("present");
        assert_eq!(after.subject, "Edited");
        assert_eq!(after.provider_draft_id.as_deref(), Some("gmail-draft-42"));
    }

    #[test]
    fn replace_and_list_draft_attachments() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "draft-1");
        let atts = vec![DraftAttachment {
            id: String::new(),
            draft_id: "draft-1".to_string(),
            file_path: "/tmp/report.pdf".to_string(),
            filename: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
        }];
        db.replace_draft_attachments("draft-1", &atts).expect("replace");

        let listed = db.list_draft_attachments("draft-1").expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].filename, "report.pdf");
        assert_eq!(listed[0].mime_type, "application/pdf");
        assert!(!listed[0].id.is_empty(), "id generated on insert");

        // get_draft surfaces the attachments too.
        let got = db.get_draft("draft-1").expect("ok").expect("present");
        assert_eq!(got.attachments.len(), 1);

        // Replace is a full swap, not an append.
        db.replace_draft_attachments("draft-1", &[]).expect("clear");
        assert!(db.list_draft_attachments("draft-1").expect("list").is_empty());
    }

    #[test]
    fn deleting_draft_cascades_attachments() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "draft-1");
        db.replace_draft_attachments(
            "draft-1",
            &[DraftAttachment {
                id: String::new(),
                draft_id: "draft-1".to_string(),
                file_path: "/tmp/a.txt".to_string(),
                filename: "a.txt".to_string(),
                mime_type: "text/plain".to_string(),
            }],
        )
        .expect("replace");
        db.delete_draft("draft-1", "acct-1").expect("delete");
        assert!(db.list_draft_attachments("draft-1").expect("list").is_empty());
    }

    fn provider_draft(id: &str, subject: &str) -> ProviderDraft {
        ProviderDraft {
            provider_draft_id: id.to_string(),
            to_addresses: vec!["dest@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: subject.to_string(),
            body: "body".to_string(),
            body_html: None,
            updated_at: None,
            provider_message_id: None,
            in_reply_to: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn upsert_provider_draft_persists_the_change_token() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");

        let mut pd = provider_draft("p-1", "Hello");
        pd.provider_message_id = Some("msg-1".to_string());
        db.upsert_provider_draft("acct-1", &pd).expect("insert");

        let tokens = db.provider_draft_change_tokens("acct-1").expect("tokens");
        assert_eq!(tokens.get("p-1").map(String::as_str), Some("msg-1"));
    }

    #[test]
    fn re_upserting_a_draft_moves_its_change_token() {
        // A draft edited upstream must not keep answering with the stale token,
        // or the next pull would skip the content it just fetched.
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");

        let mut pd = provider_draft("p-1", "First");
        pd.provider_message_id = Some("msg-1".to_string());
        db.upsert_provider_draft("acct-1", &pd).expect("insert");

        pd.subject = "Second".to_string();
        pd.provider_message_id = Some("msg-2".to_string());
        db.upsert_provider_draft("acct-1", &pd).expect("update");

        let tokens = db.provider_draft_change_tokens("acct-1").expect("tokens");
        assert_eq!(tokens.get("p-1").map(String::as_str), Some("msg-2"));
    }

    #[test]
    fn drafts_without_a_change_token_are_omitted() {
        // Rows predating the migration have NULL here; the planner must see
        // them as "unknown" so their content is fetched once and backfilled.
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        db.upsert_provider_draft("acct-1", &provider_draft("p-1", "No token"))
            .expect("insert");

        let tokens = db.provider_draft_change_tokens("acct-1").expect("tokens");
        assert!(tokens.is_empty(), "NULL token must not look like a match");
    }

    #[test]
    fn change_tokens_are_scoped_to_one_account() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        seed_account(&db, "acct-2");

        let mut pd = provider_draft("p-1", "Mine");
        pd.provider_message_id = Some("msg-1".to_string());
        db.upsert_provider_draft("acct-1", &pd).expect("insert");

        let tokens = db.provider_draft_change_tokens("acct-2").expect("tokens");
        assert!(tokens.is_empty(), "must not leak tokens across accounts");
    }

    #[test]
    fn upsert_provider_draft_inserts_then_updates_in_place() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");

        let id1 = db
            .upsert_provider_draft("acct-1", &provider_draft("p-1", "First"))
            .expect("insert");
        let id2 = db
            .upsert_provider_draft("acct-1", &provider_draft("p-1", "Updated"))
            .expect("update");
        assert_eq!(id1, id2, "same provider id updates the same local row");

        let drafts = db.list_drafts("acct-1").expect("list");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].subject, "Updated");
        assert_eq!(drafts[0].provider_draft_id.as_deref(), Some("p-1"));
    }

    #[test]
    fn upsert_provider_draft_keeps_the_providers_own_timestamp() {
        // Regression: every sync re-stamped pulled drafts with now(), so the
        // Drafts list showed today's date for every draft no matter how old.
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");

        let mut pd = provider_draft("p-1", "Written last year");
        pd.updated_at = Some(1_700_000_000);
        db.upsert_provider_draft("acct-1", &pd).expect("insert");

        let drafts = db.list_drafts("acct-1").expect("list");
        assert_eq!(
            drafts[0].updated_at, 1_700_000_000,
            "provider timestamp wins over now()"
        );
    }

    #[test]
    fn upsert_provider_draft_without_timestamp_falls_back_to_now() {
        // Providers that don't report a draft date still need a sane value.
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");

        let before = crate::services::clock::now_secs();
        db.upsert_provider_draft("acct-1", &provider_draft("p-1", "No date"))
            .expect("insert");

        let drafts = db.list_drafts("acct-1").expect("list");
        assert!(
            drafts[0].updated_at >= before,
            "missing provider timestamp falls back to now"
        );
    }

    fn sync_state(db: &Database, id: &str) -> DraftSyncState {
        db.draft_sync_states("acct-1")
            .expect("states")
            .into_iter()
            .find(|s| s.id == id)
            .expect("draft listed")
    }

    fn user_save(db: &Database, id: &str, body: &str) {
        seed_account(db, "acct-1");
        db.save_user_draft(&SaveDraftRequest {
            id: Some(id.to_string()),
            email_id: None,
            account_id: "acct-1".to_string(),
            to_addresses: vec!["dest@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Subject".to_string(),
            body: body.to_string(),
            body_html: None,
            provider_draft_id: None,
            attachments: None,
        })
        .expect("user save");
    }

    #[test]
    fn a_user_save_marks_the_draft_dirty_and_a_plain_save_does_not() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "plain");
        user_save(&db, "edited", "v1");

        assert!(!sync_state(&db, "plain").dirty);
        assert!(sync_state(&db, "edited").dirty);

        // A plain save over a dirty draft must not launder it clean.
        db.save_draft(&SaveDraftRequest {
            id: Some("edited".to_string()),
            email_id: None,
            account_id: "acct-1".to_string(),
            to_addresses: Vec::new(),
            cc_addresses: Vec::new(),
            subject: "Subject".to_string(),
            body: "v2".to_string(),
            body_html: None,
            provider_draft_id: None,
            attachments: None,
        })
        .expect("plain save");
        assert!(sync_state(&db, "edited").dirty);
    }

    #[test]
    fn mark_draft_pushed_cleans_the_revision_it_pushed() {
        let db = Database::new_for_testing().expect("test db");
        user_save(&db, "d-1", "v1");
        let (draft, revision) = db.draft_for_push("d-1").expect("ok").expect("present");
        assert_eq!(draft.body, "v1");

        db.mark_draft_pushed("d-1", "p-1", revision).expect("mark");

        let state = sync_state(&db, "d-1");
        assert!(!state.dirty);
        assert_eq!(state.provider_draft_id.as_deref(), Some("p-1"));
    }

    #[test]
    fn a_save_during_a_push_keeps_the_draft_dirty() {
        // The push sent v1; the user saved v2 before it returned. Clearing the
        // marker now would strand v2 on this device.
        let db = Database::new_for_testing().expect("test db");
        user_save(&db, "d-1", "v1");
        let (_, pushed_revision) = db.draft_for_push("d-1").expect("ok").expect("present");
        user_save(&db, "d-1", "v2");

        db.mark_draft_pushed("d-1", "p-1", pushed_revision).expect("mark");

        let state = sync_state(&db, "d-1");
        assert!(state.dirty, "the newer save still has to be pushed");
        assert_eq!(state.provider_draft_id.as_deref(), Some("p-1"), "the link is kept");
    }

    #[test]
    fn mark_draft_pushed_drops_the_stale_change_token() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        let mut pd = provider_draft("p-1", "Pulled");
        pd.provider_message_id = Some("msg-1".to_string());
        let id = db.upsert_provider_draft("acct-1", &pd).expect("insert");

        db.mark_draft_pushed(&id, "p-1", 0).expect("mark");

        let tokens = db.provider_draft_change_tokens("acct-1").expect("tokens");
        assert!(tokens.is_empty(), "the push minted a token this row has not seen");
    }

    #[test]
    fn draft_for_push_unknown_id_is_none() {
        let db = Database::new_for_testing().expect("test db");
        assert!(db.draft_for_push("ghost").expect("ok").is_none());
    }

    #[test]
    fn upsert_provider_draft_leaves_a_dirty_draft_untouched() {
        let db = Database::new_for_testing().expect("test db");
        user_save(&db, "d-1", "written here");
        let (_, revision) = db.draft_for_push("d-1").expect("ok").expect("present");
        db.mark_draft_pushed("d-1", "p-1", revision).expect("link");
        user_save(&db, "d-1", "edited here");

        let id = db
            .upsert_provider_draft("acct-1", &provider_draft("p-1", "Edited elsewhere"))
            .expect("upsert");

        assert_eq!(id, "d-1");
        let draft = db.get_draft("d-1").expect("ok").expect("present");
        assert_eq!(draft.body, "edited here");
        assert_eq!(draft.subject, "Subject");
        assert!(sync_state(&db, "d-1").dirty);
    }

    fn pulled_attachment(file_path: &str) -> DraftAttachment {
        DraftAttachment {
            id: String::new(),
            draft_id: String::new(),
            file_path: file_path.to_string(),
            filename: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
        }
    }

    #[test]
    fn store_pulled_draft_swaps_the_attachments_and_names_the_files_it_dropped() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        let pd = provider_draft("p-1", "Pulled");
        let first = db
            .store_pulled_draft(
                "acct-1",
                &pd,
                &[pulled_attachment("kept.pdf"), pulled_attachment("old.pdf")],
                true,
            )
            .expect("first");
        assert!(first.applied);

        let second = db
            .store_pulled_draft(
                "acct-1",
                &pd,
                &[pulled_attachment("kept.pdf"), pulled_attachment("new.pdf")],
                true,
            )
            .expect("second");

        assert_eq!(second.id, first.id);
        assert_eq!(second.dropped_files, vec!["old.pdf".to_string()]);
        let paths: Vec<String> = db
            .list_draft_attachments(&first.id)
            .expect("list")
            .into_iter()
            .map(|att| att.file_path)
            .collect();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"kept.pdf".to_string()) && paths.contains(&"new.pdf".to_string()));
    }

    #[test]
    fn store_pulled_draft_leaves_a_dirty_drafts_attachments_alone() {
        let db = Database::new_for_testing().expect("test db");
        user_save(&db, "d-1", "written here");
        let (_, revision) = db.draft_for_push("d-1").expect("ok").expect("present");
        db.mark_draft_pushed("d-1", "p-1", revision).expect("link");
        user_save(&db, "d-1", "edited here");
        db.replace_draft_attachments("d-1", &[pulled_attachment("mine.pdf")])
            .expect("attach");

        let stored = db
            .store_pulled_draft(
                "acct-1",
                &provider_draft("p-1", "Edited elsewhere"),
                &[pulled_attachment("theirs.pdf")],
                true,
            )
            .expect("store");

        assert!(!stored.applied, "the local draft wins");
        assert!(stored.dropped_files.is_empty());
        let attachments = db.list_draft_attachments("d-1").expect("list");
        assert_eq!(attachments[0].file_path, "mine.pdf");
    }

    #[test]
    fn an_incomplete_pull_records_no_change_token() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        let mut pd = provider_draft("p-1", "Pulled");
        pd.provider_message_id = Some("msg-1".to_string());

        db.store_pulled_draft("acct-1", &pd, &[], false).expect("store");

        assert!(db.provider_draft_change_tokens("acct-1").expect("tokens").is_empty());
    }

    #[test]
    fn deleting_a_draft_names_the_files_it_held() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "draft-1");
        db.replace_draft_attachments("draft-1", &[pulled_attachment("a.pdf")])
            .expect("attach");

        assert!(
            db.delete_draft("draft-1", "someone-else").expect("delete").is_empty(),
            "another account's delete removes nothing, so it drops nothing"
        );
        assert_eq!(
            db.delete_draft("draft-1", "acct-1").expect("delete"),
            vec!["a.pdf".to_string()]
        );
    }

    #[test]
    fn prune_names_only_the_files_of_the_drafts_it_removed() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        let gone = db
            .store_pulled_draft(
                "acct-1",
                &provider_draft("p-1", "gone"),
                &[pulled_attachment("gone.pdf")],
                true,
            )
            .expect("gone");
        // Turned dirty after the sync planned to prune it: kept, file included.
        user_save(&db, "dirty-1", "unpushed");
        db.mark_draft_pushed("dirty-1", "p-2", 0).expect("link");
        db.replace_draft_attachments("dirty-1", &[pulled_attachment("kept.pdf")])
            .expect("attach");

        let pruned = db
            .prune_provider_drafts("acct-1", &[gone.id, "dirty-1".to_string()])
            .expect("prune");

        assert_eq!(pruned.removed, 1);
        assert_eq!(pruned.dropped_files, vec!["gone.pdf".to_string()]);
    }

    #[test]
    fn prune_removes_only_the_clean_linked_drafts_it_is_given() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        let keep = db
            .upsert_provider_draft("acct-1", &provider_draft("p-1", "keep"))
            .expect("p1");
        let gone = db
            .upsert_provider_draft("acct-1", &provider_draft("p-2", "gone"))
            .expect("p2");
        // Linked but edited since: must survive even when asked to go.
        user_save(&db, "dirty-1", "unpushed");
        db.mark_draft_pushed("dirty-1", "p-3", 0).expect("link");
        // A local-only draft must survive pruning.
        save(&db, "local-1");

        let removed = db
            .prune_provider_drafts("acct-1", &[gone.clone(), "dirty-1".to_string(), "local-1".to_string()])
            .expect("prune");
        assert_eq!(removed.removed, 1);

        let ids: Vec<String> = db
            .list_drafts("acct-1")
            .expect("list")
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert!(ids.contains(&keep));
        assert!(ids.contains(&"dirty-1".to_string()));
        assert!(ids.contains(&"local-1".to_string()));
        assert!(!ids.contains(&gone));
    }

    #[test]
    fn prune_is_scoped_to_the_account() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        seed_account(&db, "acct-2");
        let id = db
            .upsert_provider_draft("acct-1", &provider_draft("p-1", "mine"))
            .expect("p1");

        let removed = db.prune_provider_drafts("acct-2", &[id]).expect("prune");

        assert_eq!(removed.removed, 0);
        assert_eq!(db.list_drafts("acct-1").expect("list").len(), 1);
    }

    #[test]
    fn save_draft_accepts_input_attachments_type_compiles() {
        // Guards the model surface the service layer relies on.
        let input = DraftAttachmentInput {
            file_path: "/tmp/x".to_string(),
            filename: None,
            mime_type: None,
        };
        assert_eq!(input.file_path, "/tmp/x");
    }
}
