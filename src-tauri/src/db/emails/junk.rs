//! Read/write for the `email_junk` table, plus the history queries the
//! detector's signal materialization depends on.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::services::junk::verdict::{Band, JunkKind, JunkVerdict, Method, Reason};

const ID_CHUNK: usize = 500;

/// A stored verdict, plus the user's override if they have corrected it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredJunkVerdict {
    pub email_id: String,
    pub spam_score: f32,
    pub phish_score: f32,
    pub gray_score: f32,
    pub band: Band,
    pub primary_kind: JunkKind,
    pub reasons: Vec<Reason>,
    pub method: Method,
    pub model_version: i64,
    pub scored_at: i64,
    pub user_override: Option<String>,
}

impl StoredJunkVerdict {
    /// Should the UI show a junk badge?
    ///
    /// A `not_junk` override always wins, whatever the scores say.
    pub fn is_flagged(&self) -> bool {
        if self.user_override.as_deref() == Some("not_junk") {
            return false;
        }
        self.user_override.as_deref() == Some("junk") || self.band.is_flagged()
    }
}

fn band_str(band: Band) -> &'static str {
    match band {
        Band::Clean => "clean",
        Band::Unknown => "unknown",
        Band::Uncertain => "uncertain",
        Band::Junk => "junk",
    }
}

fn kind_str(kind: JunkKind) -> &'static str {
    match kind {
        JunkKind::Legit => "legit",
        JunkKind::Spam => "spam",
        JunkKind::Phishing => "phishing",
        JunkKind::Graymail => "graymail",
    }
}

fn method_str(method: Method) -> &'static str {
    match method {
        Method::Deterministic => "deterministic",
        Method::Statistical => "statistical",
        Method::Llm => "llm",
    }
}

fn parse_band(s: &str) -> Band {
    match s {
        "junk" => Band::Junk,
        "uncertain" => Band::Uncertain,
        "unknown" => Band::Unknown,
        _ => Band::Clean,
    }
}

fn parse_kind(s: &str) -> JunkKind {
    match s {
        "phishing" => JunkKind::Phishing,
        "spam" => JunkKind::Spam,
        "graymail" => JunkKind::Graymail,
        _ => JunkKind::Legit,
    }
}

fn parse_method(s: &str) -> Method {
    match s {
        "llm" => Method::Llm,
        "statistical" => Method::Statistical,
        _ => Method::Deterministic,
    }
}

/// The worst band across the three axes — what the list view filters on.
fn overall_band(verdict: &JunkVerdict) -> Band {
    [verdict.phishing.band, verdict.spam.band, verdict.graymail.band]
        .into_iter()
        .max()
        .unwrap_or(Band::Clean)
}

impl Database {
    /// Store a verdict, preserving any user override already recorded.
    ///
    /// The override is deliberately not overwritten: a re-score (new model
    /// version, backfill, changed weights) must never undo the user's
    /// correction.
    pub fn upsert_junk_verdict(
        &self,
        email_id: &str,
        account_id: &str,
        verdict: &JunkVerdict,
        model_version: i64,
        now: i64,
    ) -> Result<()> {
        let reasons_json = serde_json::to_string(&verdict.reasons)?;
        let conn = self.connection();
        conn.execute(
            r#"INSERT INTO email_junk
               (email_id, account_id, spam_score, phish_score, gray_score, band, primary_kind,
                reasons_json, method, model_version, scored_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
               ON CONFLICT(email_id) DO UPDATE SET
                 spam_score = excluded.spam_score,
                 phish_score = excluded.phish_score,
                 gray_score = excluded.gray_score,
                 band = excluded.band,
                 primary_kind = excluded.primary_kind,
                 reasons_json = excluded.reasons_json,
                 method = excluded.method,
                 model_version = excluded.model_version,
                 scored_at = excluded.scored_at"#,
            params![
                email_id,
                account_id,
                verdict.spam.score,
                verdict.phishing.score,
                verdict.graymail.score,
                band_str(overall_band(verdict)),
                kind_str(verdict.primary),
                reasons_json,
                method_str(verdict.method),
                model_version,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_junk_verdicts_batch(&self, email_ids: &[String]) -> Result<HashMap<String, StoredJunkVerdict>> {
        let mut out = HashMap::new();
        if email_ids.is_empty() {
            return Ok(out);
        }
        let conn = self.reader();
        for chunk in email_ids.chunks(ID_CHUNK) {
            let placeholders = std::iter::repeat_n("?", chunk.len()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT email_id, spam_score, phish_score, gray_score, band, primary_kind,
                        reasons_json, method, model_version, scored_at, user_override
                 FROM email_junk WHERE email_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                let reasons_json: String = row.get(6)?;
                Ok(StoredJunkVerdict {
                    email_id: row.get(0)?,
                    spam_score: row.get(1)?,
                    phish_score: row.get(2)?,
                    gray_score: row.get(3)?,
                    band: parse_band(&row.get::<_, String>(4)?),
                    primary_kind: parse_kind(&row.get::<_, String>(5)?),
                    reasons: serde_json::from_str(&reasons_json).unwrap_or_default(),
                    method: parse_method(&row.get::<_, String>(7)?),
                    model_version: row.get(8)?,
                    scored_at: row.get(9)?,
                    user_override: row.get(10)?,
                })
            })?;
            for row in rows {
                let v = row?;
                out.insert(v.email_id.clone(), v);
            }
        }
        Ok(out)
    }

    /// Record the user's correction.
    ///
    /// Creates a placeholder row when the message has not been scored yet, so a
    /// `not_junk` decision is durable even if scoring runs afterwards.
    pub fn set_junk_override(&self, email_id: &str, account_id: &str, verdict: Option<&str>, now: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            r#"INSERT INTO email_junk
               (email_id, account_id, spam_score, phish_score, gray_score, band, primary_kind,
                reasons_json, method, model_version, scored_at, user_override, overridden_at)
               VALUES (?1, ?2, 0, 0, 0, 'clean', 'legit', '[]', 'deterministic', 0, ?4, ?3, ?4)
               ON CONFLICT(email_id) DO UPDATE SET
                 user_override = ?3,
                 overridden_at = ?4"#,
            params![email_id, account_id, verdict, now],
        )?;
        Ok(())
    }

    /// Email ids in this account that have no junk verdict yet.
    pub fn get_unscored_junk_email_ids(
        &self,
        account_id: &str,
        limit: usize,
        min_timestamp: i64,
    ) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT e.id FROM emails e
             LEFT JOIN email_junk j ON j.email_id = e.id
             WHERE e.account_id = ?1 AND e.is_deleted = 0 AND e.timestamp >= ?2 AND j.email_id IS NULL
             ORDER BY e.timestamp DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, min_timestamp, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ── History the detector reasons about ────────────────────────────────

    /// Registrable-ish domains and display names the user actually corresponds
    /// with: senders in threads where the user has sent something.
    ///
    /// This is the reference set lookalike and impersonation detection measure
    /// against, so it must reflect real correspondence rather than everything
    /// that ever landed in the inbox — otherwise a spammer who mailed twice
    /// becomes a "known contact" and immunises their own lookalikes.
    pub fn get_known_contacts(&self, account_id: &str, limit: usize) -> Result<(Vec<String>, Vec<String>)> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT e.sender_domain, e.sender
             FROM emails e
             WHERE e.account_id = ?1 AND e.is_sent = 0 AND e.sender_domain IS NOT NULL
               AND EXISTS (
                 SELECT 1 FROM emails s
                 WHERE s.account_id = e.account_id AND s.thread_id = e.thread_id AND s.is_sent = 1
               )
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
        })?;

        let mut domains: HashSet<String> = HashSet::new();
        let mut names: HashSet<String> = HashSet::new();
        for row in rows {
            let (domain, name) = row?;
            if let Some(d) = domain.filter(|d| !d.trim().is_empty()) {
                domains.insert(d.to_lowercase());
            }
            if let Some(n) = name.filter(|n| !n.trim().is_empty() && !n.contains('@')) {
                names.insert(n.trim().to_string());
            }
        }
        Ok((domains.into_iter().collect(), names.into_iter().collect()))
    }

    /// Has the user ever sent a message in a thread involving this address?
    pub fn is_sender_engaged(&self, account_id: &str, sender_email: &str) -> Result<bool> {
        let conn = self.reader();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM emails e
             WHERE e.account_id = ?1 AND e.sender_email = ?2 COLLATE NOCASE
               AND EXISTS (
                 SELECT 1 FROM emails s
                 WHERE s.account_id = e.account_id AND s.thread_id = e.thread_id AND s.is_sent = 1
               )
             LIMIT 1",
            params![account_id, sender_email],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Does this thread contain a message the user sent?
    pub fn thread_has_own_message(&self, account_id: &str, thread_id: &str) -> Result<bool> {
        let conn = self.reader();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM emails WHERE account_id = ?1 AND thread_id = ?2 AND is_sent = 1 LIMIT 1",
            params![account_id, thread_id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }
}

impl Database {
    /// How many messages this exact sender address has ever sent to the account.
    ///
    /// Feeds the graymail axis: `List-Unsubscribe` marks an ESP, not an unwanted
    /// sender, so recurrence is what separates a newsletter from a one-off
    /// invitation or job offer that happens to be sent through one.
    pub fn count_messages_from_sender(&self, account_id: &str, sender_email: &str) -> Result<usize> {
        let conn = self.reader();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM emails
             WHERE account_id = ?1 AND sender_email = ?2 COLLATE NOCASE AND is_deleted = 0",
            params![account_id, sender_email],
            |r| r.get(0),
        )?;
        Ok(count.max(0) as usize)
    }
}

/// Counts behind the settings status block.
///
/// A named struct rather than the seven-wide tuple this used to return: the
/// three flagged counts and the two override counts are all `i64` and all
/// plausible in any position, so a caller transposing two of them would compile
/// cleanly and quietly report the wrong numbers to the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JunkCounts {
    pub scored: i64,
    pub unscored: i64,
    pub phishing: i64,
    pub spam: i64,
    pub graymail: i64,
    pub marked_junk: i64,
    pub marked_not_junk: i64,
}

impl Database {
    /// Counts for the settings status block.
    pub fn junk_stats_counts(&self, account_id: &str) -> Result<JunkCounts> {
        let conn = self.reader();
        let one = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, params![account_id], |r| r.get::<_, i64>(0))?) };
        // Flagged counts exclude a `not_junk` override: the status block must
        // never report a badge the user already dismissed.
        let by_kind = |kind: &str| -> Result<i64> {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM email_junk
                 WHERE account_id = ?1 AND band = 'junk' AND primary_kind = ?2
                   AND (user_override IS NULL OR user_override <> 'not_junk')",
                params![account_id, kind],
                |r| r.get::<_, i64>(0),
            )?)
        };
        let overrides = |value: &str| -> Result<i64> {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM email_junk WHERE account_id = ?1 AND user_override = ?2",
                params![account_id, value],
                |r| r.get::<_, i64>(0),
            )?)
        };
        Ok(JunkCounts {
            scored: one("SELECT COUNT(*) FROM email_junk WHERE account_id = ?1")?,
            unscored: one(
                "SELECT COUNT(*) FROM emails e LEFT JOIN email_junk j ON j.email_id = e.id
                 WHERE e.account_id = ?1 AND e.is_deleted = 0 AND j.email_id IS NULL",
            )?,
            phishing: by_kind("phishing")?,
            spam: by_kind("spam")?,
            graymail: by_kind("graymail")?,
            marked_junk: overrides("junk")?,
            marked_not_junk: overrides("not_junk")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::junk::verdict::{AxisScore, JunkVerdict};

    fn db_with_email(id: &str) -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acct-1', 'imap', 'user@example.com', 'User', 0)",
                [],
            )
            .expect("seed account");
        db.insert_emails_batch(&[Email {
            id: id.to_string(),
            account_id: "acct-1".to_string(),
            thread_id: format!("t-{id}"),
            message_id: None,
            subject: "S".into(),
            sender: "Sender".into(),
            sender_email: "s@other.example".into(),
            recipients: vec![],
            cc: vec![],
            body: "b".into(),
            snippet: "b".into(),
            timestamp: 1_700_000_000,
            is_read: false,
            is_sent: false,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            headers: None,
        }])
        .expect("insert email");
        db
    }

    fn flagged_verdict() -> JunkVerdict {
        JunkVerdict {
            phishing: AxisScore {
                score: 0.9,
                band: Band::Junk,
            },
            primary: JunkKind::Phishing,
            ..JunkVerdict::clean()
        }
    }

    #[test]
    fn a_verdict_round_trips() {
        let db = db_with_email("e1");
        db.upsert_junk_verdict("e1", "acct-1", &flagged_verdict(), 0, 100)
            .expect("store");

        let got = db.get_junk_verdicts_batch(&["e1".to_string()]).expect("fetch");
        let v = got.get("e1").expect("stored");
        assert_eq!(v.band, Band::Junk);
        assert_eq!(v.primary_kind, JunkKind::Phishing);
        assert!(v.is_flagged());
    }

    #[test]
    fn a_not_junk_override_survives_a_rescore() {
        // The single most important durability property in the feature: a
        // re-score must never undo the user's correction.
        let db = db_with_email("e1");
        db.upsert_junk_verdict("e1", "acct-1", &flagged_verdict(), 0, 100)
            .expect("store");
        db.set_junk_override("e1", "acct-1", Some("not_junk"), 200)
            .expect("override");

        db.upsert_junk_verdict("e1", "acct-1", &flagged_verdict(), 7, 300)
            .expect("rescore");

        let got = db.get_junk_verdicts_batch(&["e1".to_string()]).expect("fetch");
        let v = got.get("e1").expect("stored");
        assert_eq!(v.user_override.as_deref(), Some("not_junk"));
        assert!(!v.is_flagged(), "override must win over the fresh verdict");
        assert_eq!(v.model_version, 7, "the re-score itself still landed");
    }

    #[test]
    fn an_override_can_be_recorded_before_the_message_is_ever_scored() {
        let db = db_with_email("e1");
        db.set_junk_override("e1", "acct-1", Some("not_junk"), 100)
            .expect("override");
        let got = db.get_junk_verdicts_batch(&["e1".to_string()]).expect("fetch");
        assert_eq!(
            got.get("e1").and_then(|v| v.user_override.clone()).as_deref(),
            Some("not_junk")
        );
    }

    #[test]
    fn unscored_ids_exclude_already_scored_messages() {
        let db = db_with_email("e1");
        assert_eq!(
            db.get_unscored_junk_email_ids("acct-1", 10, 0).expect("query"),
            vec!["e1"]
        );

        db.upsert_junk_verdict("e1", "acct-1", &JunkVerdict::clean(), 0, 100)
            .expect("store");
        assert!(db
            .get_unscored_junk_email_ids("acct-1", 10, 0)
            .expect("query")
            .is_empty());
    }

    #[test]
    fn a_sender_is_only_known_once_the_user_has_replied_in_the_thread() {
        // Inbound mail alone must not make a sender "known": otherwise a
        // spammer who mails twice becomes a trusted reference domain and
        // immunises their own lookalikes.
        let db = db_with_email("e1");
        let (domains, _) = db.get_known_contacts("acct-1", 100).expect("query");
        assert!(domains.is_empty(), "inbound-only sender must not count");
        assert!(!db.is_sender_engaged("acct-1", "s@other.example").expect("query"));

        db.connection()
            .execute(
                "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                                     recipients_json, cc_json, snippet, timestamp, is_read, is_sent, category, mailbox, created_at)
                 VALUES ('sent-1','acct-1','t-e1','Re: S','User','user@example.com','example.com','[]','[]','',1,1,1,'primary','sent',0)",
                [],
            )
            .expect("seed reply");

        let (domains, _) = db.get_known_contacts("acct-1", 100).expect("query");
        assert_eq!(domains, vec!["other.example".to_string()]);
        assert!(db.is_sender_engaged("acct-1", "s@other.example").expect("query"));
        assert!(db.thread_has_own_message("acct-1", "t-e1").expect("query"));
    }

    #[test]
    fn deleting_an_email_cascades_to_its_verdict() {
        let db = db_with_email("e1");
        db.upsert_junk_verdict("e1", "acct-1", &flagged_verdict(), 0, 100)
            .expect("store");
        db.connection()
            .execute("DELETE FROM emails WHERE id = 'e1'", [])
            .expect("delete");
        assert!(db
            .get_junk_verdicts_batch(&["e1".to_string()])
            .expect("fetch")
            .is_empty());
    }
}
