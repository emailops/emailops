use rusqlite::{params, OptionalExtension};

use crate::models::error::Result;
use crate::models::{Draft, SaveDraftRequest};

use super::Database;

fn row_to_draft(row: &rusqlite::Row<'_>) -> rusqlite::Result<Draft> {
    let to_json: String = row.get(3)?;
    let to_addresses: Vec<String> = serde_json::from_str(&to_json).unwrap_or_default();
    Ok(Draft {
        id: row.get(0)?,
        email_id: row.get(1)?,
        account_id: row.get(2)?,
        to_addresses,
        subject: row.get(4)?,
        body: row.get(5)?,
        ai_generated: row.get::<_, i64>(6)? != 0,
        status: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

impl Database {
    pub fn list_drafts(&self, account_id: &str) -> Result<Vec<Draft>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, email_id, account_id, to_addresses_json, subject, body,
                    ai_generated, status, created_at, updated_at
             FROM drafts
             WHERE account_id = ?1 AND status = 'draft'
             ORDER BY updated_at DESC",
        )?;
        let drafts = stmt
            .query_map(params![account_id], row_to_draft)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(drafts)
    }

    /// Fetch a single draft by id, regardless of `status`. Used by the CLI to
    /// surface a draft the chat assistant just created (linked via the message's
    /// `referenced_draft_ids`). Returns `None` when no such draft exists.
    pub fn get_draft(&self, draft_id: &str) -> Result<Option<Draft>> {
        let conn = self.reader();
        let draft = conn
            .query_row(
                "SELECT id, email_id, account_id, to_addresses_json, subject, body,
                        ai_generated, status, created_at, updated_at
                 FROM drafts WHERE id = ?1",
                params![draft_id],
                row_to_draft,
            )
            .optional()?;
        Ok(draft)
    }

    pub fn save_draft(&self, req: &SaveDraftRequest) -> Result<Draft> {
        let conn = self.connection();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let id = req.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let to_json = serde_json::to_string(&req.to_addresses).unwrap_or_default();

        conn.execute(
            "INSERT INTO drafts (id, email_id, account_id, to_addresses_json, subject, body,
                                 ai_generated, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 'draft',
                     COALESCE((SELECT created_at FROM drafts WHERE id = ?1), ?7), ?7)
             ON CONFLICT(id) DO UPDATE SET
                email_id = excluded.email_id,
                to_addresses_json = excluded.to_addresses_json,
                subject = excluded.subject,
                body = excluded.body,
                updated_at = excluded.updated_at",
            params![id, req.email_id, req.account_id, to_json, req.subject, req.body, now],
        )?;

        // Read back through the same write connection — avoids holding the write
        // lock while also trying to acquire a read-pool slot (deadlock in test
        // mode where read_conns is empty and reader() falls back to write_conn).
        let draft = conn.query_row(
            "SELECT id, email_id, account_id, to_addresses_json, subject, body,
                    ai_generated, status, created_at, updated_at
             FROM drafts WHERE id = ?1",
            params![id],
            row_to_draft,
        )?;
        Ok(draft)
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

    fn seed_account(db: &Database, id: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
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
            subject: "Confirmar reunión".to_string(),
            body: "Hola Alina, confirmo.".to_string(),
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
        assert_eq!(got.subject, "Confirmar reunión");
        assert_eq!(got.body, "Hola Alina, confirmo.");
    }

    #[test]
    fn get_draft_unknown_id_is_none() {
        let db = Database::new_for_testing().expect("test db");
        assert!(db.get_draft("ghost").expect("get_draft ok").is_none());
    }
}
