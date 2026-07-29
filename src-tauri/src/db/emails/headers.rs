//! Read/write for the `email_headers` table.
//!
//! Headers are written as a side effect of inserting an email and read back
//! explicitly — never hydrated onto `Email` on the read path, because the vast
//! majority of reads (list views, search, threads) have no use for them and
//! would pay a join per row.

use std::collections::HashMap;

use super::*;
use crate::models::headers::RawHeaders;

/// SQLite's parameter ceiling is 32,766; stay well under it per chunk.
const ID_CHUNK: usize = 500;

/// Write one email's headers inside an existing transaction.
///
/// `INSERT OR REPLACE` so a re-sync of the same message refreshes them rather
/// than failing on the primary key.
pub(super) fn insert_email_headers_tx(
    tx: &rusqlite::Transaction<'_>,
    email_id: &str,
    account_id: &str,
    headers: &RawHeaders,
    now: i64,
) -> rusqlite::Result<()> {
    let dkim_domains = if headers.dkim_domains.is_empty() {
        None
    } else {
        Some(headers.dkim_domains.join(","))
    };
    let extra_json = if headers.extra.is_empty() {
        None
    } else {
        serde_json::to_string(&headers.extra).ok()
    };

    tx.execute(
        r#"INSERT OR REPLACE INTO email_headers
           (email_id, account_id, auth_results, authserv_id, received_spf, dkim_domains,
            return_path, reply_to, from_raw, to_raw, list_id, list_unsubscribe,
            list_unsubscribe_post, precedence, x_mailer, content_type,
            received_count, first_received, spam_headers, extra_json, captured_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)"#,
        params![
            email_id,
            account_id,
            headers.auth_results,
            headers.authserv_id,
            headers.received_spf,
            dkim_domains,
            headers.return_path,
            headers.reply_to,
            headers.from_raw,
            headers.to_raw,
            headers.list_id,
            headers.list_unsubscribe,
            headers.list_unsubscribe_post,
            headers.precedence,
            headers.x_mailer,
            headers.content_type,
            headers.received_count as i64,
            headers.first_received,
            headers.spam_headers,
            extra_json,
            now,
        ],
    )?;
    Ok(())
}

fn row_to_headers(row: &rusqlite::Row) -> rusqlite::Result<(String, RawHeaders)> {
    let email_id: String = row.get(0)?;
    let dkim_domains: Option<String> = row.get(4)?;
    let extra_json: Option<String> = row.get(18)?;

    Ok((
        email_id,
        RawHeaders {
            auth_results: row.get(1)?,
            authserv_id: row.get(2)?,
            received_spf: row.get(3)?,
            dkim_domains: dkim_domains
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            return_path: row.get(5)?,
            reply_to: row.get(6)?,
            from_raw: row.get(7)?,
            to_raw: row.get(8)?,
            list_id: row.get(9)?,
            list_unsubscribe: row.get(10)?,
            list_unsubscribe_post: row.get(11)?,
            precedence: row.get(12)?,
            x_mailer: row.get(13)?,
            content_type: row.get(14)?,
            received_count: row.get::<_, i64>(15).unwrap_or(0).max(0) as usize,
            first_received: row.get(16)?,
            spam_headers: row.get(17)?,
            extra: extra_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
        },
    ))
}

impl Database {
    /// Fetch captured headers for a batch of emails.
    ///
    /// Missing entries mean no headers were captured for that message — the
    /// caller must treat that as "unknown", never as "clean".
    pub fn get_email_headers_batch(&self, email_ids: &[String]) -> Result<HashMap<String, RawHeaders>> {
        let mut out: HashMap<String, RawHeaders> = HashMap::new();
        if email_ids.is_empty() {
            return Ok(out);
        }

        let conn = self.reader();
        for chunk in email_ids.chunks(ID_CHUNK) {
            let placeholders = std::iter::repeat_n("?", chunk.len()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT email_id, auth_results, authserv_id, received_spf, dkim_domains,
                        return_path, reply_to, from_raw, to_raw, list_id, list_unsubscribe,
                        list_unsubscribe_post, precedence, x_mailer, content_type,
                        received_count, first_received, spam_headers, extra_json
                 FROM email_headers WHERE email_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let params = rusqlite::params_from_iter(chunk.iter());
            let rows = stmt.query_map(params, row_to_headers)?;
            for row in rows {
                let (id, headers) = row?;
                out.insert(id, headers);
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::header_capture::{capture, parse_header_block};

    fn seeded_db() -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acct-1', 'imap', 'user@example.com', 'User', 0)",
                [],
            )
            .expect("seed account");
        db
    }

    fn email_with(id: &str, headers: Option<RawHeaders>) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acct-1".to_string(),
            thread_id: format!("thread-{id}"),
            message_id: None,
            subject: "Subject".to_string(),
            sender: "Sender".to_string(),
            sender_email: "sender@example.com".to_string(),
            recipients: vec!["user@example.com".to_string()],
            cc: vec![],
            body: "body".to_string(),
            snippet: "body".to_string(),
            timestamp: 1_700_000_000,
            is_read: false,
            is_sent: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "inbox".to_string(),
            headers,
        }
    }

    const RAW: &str = concat!(
        "Authentication-Results: mx.example.com; spf=fail smtp.mailfrom=evil.example;\n",
        "\tdkim=none; dmarc=fail\n",
        "Authentication-Results: forged.example; spf=pass; dmarc=pass\n",
        "From: \"Billing\" <billing@acme-payments.example>\n",
        "Reply-To: ap@mail-secure.example\n",
        "Return-Path: <bounce@mail-secure.example>\n",
        "List-Unsubscribe: <https://x.example/u>\n",
        "DKIM-Signature: v=1; d=first.example; s=a; b=AAA\n",
        "DKIM-Signature: v=1; d=second.example; s=b; b=BBB\n",
        "X-Spam-Flag: YES\n",
        "Auto-Submitted: auto-generated\n",
        "Received: from relay.example by mx.example.com\n",
        "Received: from origin.example by relay.example\n",
    );

    #[test]
    fn captured_headers_survive_a_round_trip_through_the_database() {
        let db = seeded_db();
        let captured = capture(&parse_header_block(RAW));
        db.insert_emails_batch(&[email_with("e1", Some(captured.clone()))])
            .expect("insert");

        let fetched = db.get_email_headers_batch(&["e1".to_string()]).expect("fetch");
        let got = fetched.get("e1").expect("headers stored");
        assert_eq!(got, &captured);
    }

    #[test]
    fn the_forged_authentication_results_never_reaches_storage() {
        // End-to-end version of the capture-layer property: the attacker's
        // pasted "spf=pass" must not be what a later read sees.
        let db = seeded_db();
        db.insert_emails_batch(&[email_with("e1", Some(capture(&parse_header_block(RAW))))])
            .expect("insert");

        let fetched = db.get_email_headers_batch(&["e1".to_string()]).expect("fetch");
        let auth = fetched
            .get("e1")
            .and_then(|h| h.auth_results.as_deref())
            .expect("auth results stored");
        assert!(auth.contains("spf=fail"), "got {auth:?}");
        assert!(!auth.contains("spf=pass"), "forged instance persisted: {auth:?}");
    }

    #[test]
    fn multi_valued_fields_survive_the_join_and_split() {
        let db = seeded_db();
        db.insert_emails_batch(&[email_with("e1", Some(capture(&parse_header_block(RAW))))])
            .expect("insert");

        let fetched = db.get_email_headers_batch(&["e1".to_string()]).expect("fetch");
        let got = fetched.get("e1").expect("headers stored");
        assert_eq!(
            got.dkim_domains,
            vec!["first.example".to_string(), "second.example".to_string()]
        );
        assert_eq!(
            got.extra.get("auto-submitted").map(String::as_str),
            Some("auto-generated")
        );
        assert_eq!(got.received_count, 2);
    }

    #[test]
    fn an_email_without_captured_headers_writes_no_row() {
        // The distinction the detector depends on: absent means "unknown", and
        // a fabricated empty row would read as "checked and clean".
        let db = seeded_db();
        db.insert_emails_batch(&[email_with("e1", None)]).expect("insert");

        let fetched = db.get_email_headers_batch(&["e1".to_string()]).expect("fetch");
        assert!(fetched.is_empty());
    }

    #[test]
    fn re_syncing_a_message_refreshes_its_headers() {
        let db = seeded_db();
        db.insert_emails_batch(&[email_with("e1", Some(capture(&parse_header_block(RAW))))])
            .expect("first insert");

        let updated = capture(&parse_header_block(
            "Authentication-Results: mx.example.com; spf=pass; dmarc=pass\n",
        ));
        db.insert_emails_batch(&[email_with("e1", Some(updated))])
            .expect("second insert");

        let fetched = db.get_email_headers_batch(&["e1".to_string()]).expect("fetch");
        let auth = fetched
            .get("e1")
            .and_then(|h| h.auth_results.as_deref())
            .expect("headers stored");
        assert!(auth.contains("spf=pass"), "stale row survived: {auth:?}");
    }

    #[test]
    fn deleting_an_email_cascades_to_its_headers() {
        let db = seeded_db();
        db.insert_emails_batch(&[email_with("e1", Some(capture(&parse_header_block(RAW))))])
            .expect("insert");
        db.connection()
            .execute("DELETE FROM emails WHERE id = 'e1'", [])
            .expect("delete");

        let fetched = db.get_email_headers_batch(&["e1".to_string()]).expect("fetch");
        assert!(fetched.is_empty(), "orphaned header row left behind");
    }

    #[test]
    fn an_empty_id_list_does_not_query() {
        let db = seeded_db();
        assert!(db.get_email_headers_batch(&[]).expect("fetch").is_empty());
    }
}
