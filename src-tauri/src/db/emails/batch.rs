use super::*;

impl Database {
    /// Insert or refresh multiple emails in a single transaction.
    ///
    /// This is **the** ingest path — `insert_email` delegates here — so every
    /// stored message gets the same treatment: the row, its body, its captured
    /// headers and its FTS entry. One transaction rather than N is also
    /// markedly faster on a sync chunk.
    ///
    /// An existing id is updated in place, never replaced; see the comment on
    /// the statement for why that distinction matters.
    pub fn insert_emails_batch(&self, emails: &[Email]) -> Result<()> {
        if emails.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp();
        for email in emails {
            let recipients_json = serde_json::to_string(&email.recipients)?;
            let cc_json = serde_json::to_string(&email.cc)?;
            let sender_domain = extract_sender_domain(&email.sender_email);
            let mailbox = normalize_mailbox(&email.mailbox);
            // The FTS table is not a child of `emails` (no FK), so its stale row
            // is cleared by hand.
            tx.execute("DELETE FROM emails_fts WHERE email_id = ?1", params![email.id])?;
            // UPSERT, deliberately **not** `INSERT OR REPLACE`.
            //
            // REPLACE resolves a conflict by deleting the existing row first,
            // and with `PRAGMA foreign_keys = ON` (set on every connection here)
            // that delete fires every `ON DELETE CASCADE` hanging off
            // `emails(id)`: `email_tags`, `email_junk`, `lens_rows`,
            // `lens_exclusions`, `chat_message_sources`, `embedding_chunks`,
            // `email_headers`, `attachments`, `email_attachment_meta`,
            // `email_extraction_status`. Re-downloading one message therefore
            // wiped its classification tags, its junk verdict including a
            // permanent user `not_junk` override, hand-edited lens rows, and the
            // citations tying it to past chat answers. (`AFTER DELETE` triggers
            // do *not* fire on a REPLACE conflict, which is why the FTS row
            // above always needed deleting by hand — the same asymmetry hid the
            // cascade.)
            //
            // `is_deleted` and `pending_sync` are deliberately absent from both
            // lists: REPLACE reset them to their defaults, so a re-ingested row
            // un-deleted itself. Leaving them out of the UPDATE preserves them.
            // `created_at` is likewise only set on insert — it records when this
            // mailbox first stored the message, not when it was last refreshed.
            tx.execute(
                r#"INSERT INTO emails
                   (id, account_id, thread_id, message_id, subject, sender, sender_email,
                    sender_domain, recipients_json, cc_json, snippet, timestamp, is_read, triage_status, category, mailbox, is_sent, created_at,
                    references_header)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
                   ON CONFLICT(id) DO UPDATE SET
                     account_id = excluded.account_id,
                     thread_id = excluded.thread_id,
                     message_id = excluded.message_id,
                     subject = excluded.subject,
                     sender = excluded.sender,
                     sender_email = excluded.sender_email,
                     sender_domain = excluded.sender_domain,
                     recipients_json = excluded.recipients_json,
                     cc_json = excluded.cc_json,
                     snippet = excluded.snippet,
                     timestamp = excluded.timestamp,
                     is_read = excluded.is_read,
                     triage_status = excluded.triage_status,
                     category = excluded.category,
                     mailbox = excluded.mailbox,
                     is_sent = excluded.is_sent,
                     references_header = excluded.references_header"#,
                params![
                    email.id,
                    email.account_id,
                    email.thread_id,
                    email.message_id,
                    email.subject,
                    email.sender,
                    email.sender_email,
                    sender_domain,
                    recipients_json,
                    cc_json,
                    email.snippet,
                    email.timestamp,
                    email.is_read as i32,
                    email.triage_status,
                    email.category,
                    mailbox,
                    is_sent_flag(email, mailbox) as i32,
                    now,
                    email.references,
                ],
            )?;
            tx.execute(
                "INSERT INTO email_bodies (email_id, body) VALUES (?1, ?2)
                 ON CONFLICT(email_id) DO UPDATE SET body = excluded.body",
                params![email.id, email.body],
            )?;
            // Captured RFC 5322 headers, when the provider supplied them. No row
            // at all when it didn't: the junk detector distinguishes "no
            // evidence" from "checked and clean", so a fabricated empty row
            // would be a lie.
            if let Some(headers) = &email.headers {
                super::headers::insert_email_headers_tx(&tx, &email.id, &email.account_id, headers, now)?;
            }
            // Manual FTS insert with stripped HTML (triggers removed)
            let body_text = strip_html_for_fts(&email.body);
            tx.execute(
                "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, ?3, ?4)",
                params![email.id, email.subject, email.sender, body_text],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::db::Database;

    /// `INSERT OR REPLACE` resolves a conflict by *deleting* the existing row
    /// and inserting a new one. With `PRAGMA foreign_keys = ON` — which this app
    /// sets on every connection — that delete fires every `ON DELETE CASCADE`
    /// hanging off `emails(id)`, while `AFTER DELETE` triggers do NOT fire
    /// (recursive_triggers is off). The FTS row was hand-deleted here for
    /// exactly that reason; the cascade side was never handled.
    ///
    /// Ordinary sync filters out ids it already holds, so this never bit there.
    /// The user-facing re-download hits it by construction: `redownload_email`
    /// re-inserts a row that exists.
    ///
    /// What the user loses on a re-download: classification tags, the junk
    /// verdict *including a permanent `not_junk` override*, lens rows with
    /// hand-edited values, the chat citations linking the mail to past answers,
    /// and its embeddings.
    #[test]
    fn re_ingesting_an_email_keeps_its_tags() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.test");
        db.insert_emails_batch(&[email_fixture("e1", "acc1", "<p>original</p>")])
            .unwrap();
        db.upsert_email_tag("e1", "intent", "invoice", Some(0.9)).unwrap();

        db.insert_emails_batch(&[email_fixture("e1", "acc1", "<p>re-downloaded</p>")])
            .unwrap();

        let tags = db.get_email_tags("e1").unwrap();
        assert_eq!(
            tags.iter().map(|t| t.tag_value.as_str()).collect::<Vec<_>>(),
            vec!["invoice"],
            "re-ingesting must not cascade-delete the email's tags"
        );
    }

    /// `is_deleted` is not in the insert's column list, so a REPLACE reset it to
    /// its default of 0 — resurrecting a message the user had deleted.
    #[test]
    fn re_ingesting_a_deleted_email_leaves_it_deleted() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.test");
        db.insert_emails_batch(&[email_fixture("e1", "acc1", "<p>original</p>")])
            .unwrap();
        db.delete_email("e1").unwrap();

        db.insert_emails_batch(&[email_fixture("e1", "acc1", "<p>re-downloaded</p>")])
            .unwrap();

        // `get_email` deliberately returns soft-deleted rows, so assert on the
        // flag itself rather than on the row's absence.
        let is_deleted: i64 = db
            .reader()
            .query_row("SELECT is_deleted FROM emails WHERE id = 'e1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(is_deleted, 1, "a re-ingested row must not un-delete itself");
    }

    /// `insert_email` (the single-row path used by the failed-download retry and
    /// by `redownload_email`) wrote only `emails` + `email_bodies`. For a
    /// retried message that insert is the *first* one, so the mail never got an
    /// FTS row: invisible to keyword search and to the chat's `search_emails`
    /// tool, permanently — `populate_fts_if_empty` only runs when the whole
    /// index is empty.
    #[test]
    fn single_row_insert_makes_the_email_searchable() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.test");

        db.insert_email(&email_fixture("e1", "acc1", "<p>reconciliation spreadsheet</p>"))
            .unwrap();

        let hits: i64 = db
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM emails_fts WHERE emails_fts MATCH 'reconciliation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "a singly-inserted email must be findable by keyword");
    }

    /// Re-ingesting must not leave the *old* body in the index either.
    #[test]
    fn re_ingesting_replaces_the_search_index_entry() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.test");
        db.insert_email(&email_fixture("e1", "acc1", "<p>original wording</p>"))
            .unwrap();

        db.insert_email(&email_fixture("e1", "acc1", "<p>revised wording</p>"))
            .unwrap();

        let stale: i64 = db
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM emails_fts WHERE emails_fts MATCH 'original'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0, "the superseded body must leave the index");
    }
}
