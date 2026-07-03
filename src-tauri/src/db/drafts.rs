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

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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

    pub fn save_draft(&self, req: &SaveDraftRequest) -> Result<Draft> {
        let conn = self.connection();
        let now = now_secs();

        let id = req.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let to_json = serde_json::to_string(&req.to_addresses).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(&req.cc_addresses).unwrap_or_else(|_| "[]".to_string());

        conn.execute(
            "INSERT INTO drafts (id, email_id, account_id, to_addresses_json, cc_addresses_json,
                                 subject, body, body_html, ai_generated, status,
                                 provider_draft_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 'draft', ?9,
                     COALESCE((SELECT created_at FROM drafts WHERE id = ?1), ?10), ?10)
             ON CONFLICT(id) DO UPDATE SET
                email_id = excluded.email_id,
                to_addresses_json = excluded.to_addresses_json,
                cc_addresses_json = excluded.cc_addresses_json,
                subject = excluded.subject,
                body = excluded.body,
                body_html = excluded.body_html,
                -- keep an existing provider link when the save omits one
                provider_draft_id = COALESCE(excluded.provider_draft_id, drafts.provider_draft_id),
                updated_at = excluded.updated_at",
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

    /// Record the provider-side draft id for a local draft after a push.
    pub fn set_provider_draft_id(&self, draft_id: &str, provider_draft_id: Option<&str>) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE drafts SET provider_draft_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![draft_id, provider_draft_id, now_secs()],
        )?;
        Ok(())
    }

    /// Replace the attachment set for a draft (delete-all then insert) in one
    /// transaction. `attachments` carry pre-resolved filename/mime; the row `id`
    /// is (re)generated here so callers don't have to.
    pub fn replace_draft_attachments(&self, draft_id: &str, attachments: &[DraftAttachment]) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM draft_attachments WHERE draft_id = ?1", params![draft_id])?;
        for att in attachments {
            tx.execute(
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
        tx.commit()?;
        Ok(())
    }

    pub fn list_draft_attachments(&self, draft_id: &str) -> Result<Vec<DraftAttachment>> {
        let conn = self.reader();
        Ok(attachments_for(&conn, draft_id)?)
    }

    /// Upsert a draft pulled from the provider, keyed by `(account_id,
    /// provider_draft_id)`. Updates an existing local row in place (preserving
    /// its local id and `created_at`) or inserts a new one. Returns the local id.
    pub fn upsert_provider_draft(&self, account_id: &str, draft: &ProviderDraft) -> Result<String> {
        let conn = self.connection();
        let now = now_secs();
        let to_json = serde_json::to_string(&draft.to_addresses).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(&draft.cc_addresses).unwrap_or_else(|_| "[]".to_string());

        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM drafts WHERE account_id = ?1 AND provider_draft_id = ?2",
                params![account_id, draft.provider_draft_id],
                |row| row.get(0),
            )
            .optional()?;

        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        conn.execute(
            "INSERT INTO drafts (id, email_id, account_id, to_addresses_json, cc_addresses_json,
                                 subject, body, body_html, ai_generated, status,
                                 provider_draft_id, created_at, updated_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'draft', ?8,
                     COALESCE((SELECT created_at FROM drafts WHERE id = ?1), ?9), ?9)
             ON CONFLICT(id) DO UPDATE SET
                to_addresses_json = excluded.to_addresses_json,
                cc_addresses_json = excluded.cc_addresses_json,
                subject = excluded.subject,
                body = excluded.body,
                body_html = excluded.body_html,
                provider_draft_id = excluded.provider_draft_id,
                updated_at = excluded.updated_at",
            params![
                id,
                account_id,
                to_json,
                cc_json,
                draft.subject,
                draft.body,
                draft.body_html,
                draft.provider_draft_id,
                now,
            ],
        )?;
        Ok(id)
    }

    /// Delete provider-linked local drafts for an account whose provider id is
    /// no longer present upstream (sent or deleted elsewhere). Local-only drafts
    /// (`provider_draft_id IS NULL`) are never touched.
    pub fn prune_provider_drafts(&self, account_id: &str, keep_provider_ids: &[String]) -> Result<usize> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        {
            let mut stmt = tx.prepare(
                "SELECT id, provider_draft_id FROM drafts
                 WHERE account_id = ?1 AND provider_draft_id IS NOT NULL",
            )?;
            let rows = stmt
                .query_map(params![account_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (local_id, provider_id) in rows {
                if !keep_provider_ids.iter().any(|k| k == &provider_id) {
                    tx.execute("DELETE FROM drafts WHERE id = ?1", params![local_id])?;
                    removed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn delete_draft(&self, draft_id: &str, account_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM drafts WHERE id = ?1 AND account_id = ?2",
            params![draft_id, account_id],
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
    fn set_provider_draft_id_links_and_save_preserves_it() {
        let db = Database::new_for_testing().expect("test db");
        save(&db, "draft-1");
        db.set_provider_draft_id("draft-1", Some("gmail-draft-42"))
            .expect("set link");

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
        }
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
    fn prune_removes_absent_provider_drafts_only() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        db.upsert_provider_draft("acct-1", &provider_draft("p-1", "keep"))
            .expect("p1");
        db.upsert_provider_draft("acct-1", &provider_draft("p-2", "gone"))
            .expect("p2");
        // A local-only draft must survive pruning.
        save(&db, "local-1");

        let removed = db.prune_provider_drafts("acct-1", &["p-1".to_string()]).expect("prune");
        assert_eq!(removed, 1);

        let drafts = db.list_drafts("acct-1").expect("list");
        let subjects: Vec<_> = drafts.iter().map(|d| d.subject.as_str()).collect();
        assert!(subjects.contains(&"keep"));
        assert!(subjects.contains(&"Confirmar reunión")); // the local-only draft
        assert!(!subjects.contains(&"gone"));
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
