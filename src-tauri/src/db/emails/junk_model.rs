//! Model persistence and the labelled rows the statistical layer trains on.

use rusqlite::OptionalExtension;

use super::*;
use crate::services::junk::model::{from_blob, to_blob, ModelAxis, NaiveBayes};

/// One labelled message, projected to exactly what the tokenizer consumes.
///
/// Subject + snippet + sender only: the snippet is already on the `emails` row,
/// so a full retrain over tens of thousands of messages never loads a single
/// body. Scoring must use the same projection or the features would not line up.
#[derive(Debug, Clone)]
pub struct TrainingRow {
    pub subject: String,
    pub snippet: String,
    pub sender_email: String,
    pub x_mailer: Option<String>,
    pub positive: bool,
    pub weight: u32,
}

/// The user's own corrections count for far more than a label inferred from the
/// provider's folders.
const FEEDBACK_WEIGHT: u32 = 5;

impl Database {
    pub fn save_junk_model(&self, account_id: &str, axis: ModelAxis, model: &NaiveBayes, now: i64) -> Result<i64> {
        let conn = self.connection();
        let next_version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM junk_model WHERE account_id = ?1 AND axis = ?2",
                params![account_id, axis.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(1);

        conn.execute(
            "INSERT OR REPLACE INTO junk_model (account_id, axis, version, n_pos, n_neg, counts_blob, trained_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                account_id,
                axis.as_str(),
                next_version,
                model.n_pos,
                model.n_neg,
                to_blob(model),
                now,
            ],
        )?;
        Ok(next_version)
    }

    /// Load a trained model. `None` when never trained, or when the stored blob
    /// no longer matches the current bucket count.
    pub fn load_junk_model(&self, account_id: &str, axis: ModelAxis) -> Result<Option<(NaiveBayes, i64)>> {
        let conn = self.reader();
        let row = conn
            .query_row(
                "SELECT n_pos, n_neg, counts_blob, version FROM junk_model WHERE account_id = ?1 AND axis = ?2",
                params![account_id, axis.as_str()],
                |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;

        let Some((n_pos, n_neg, blob, version)) = row else {
            return Ok(None);
        };
        Ok(from_blob(&blob, n_pos, n_neg).map(|m| (m, version)))
    }

    /// Labelled rows for one axis.
    ///
    /// Every label here is derived from data the user already produced — no
    /// cold-start labelling session. The `weight` column is what keeps an
    /// explicit correction from being outvoted by a pile of inferred labels.
    pub fn get_junk_training_rows(&self, account_id: &str, axis: ModelAxis, limit: usize) -> Result<Vec<TrainingRow>> {
        let conn = self.reader();
        let cap = limit as i64;

        // Positives and negatives are gathered separately so one class cannot
        // crowd the other out of a shared LIMIT.
        let (positive_sql, negative_sql) = match axis {
            ModelAxis::Spam => (
                // The provider already sorted these. Free, and labelled by a
                // scanner with far more context than we have locally.
                "SELECT e.subject, e.snippet, e.sender_email, h.x_mailer
                 FROM emails e LEFT JOIN email_headers h ON h.email_id = e.id
                 WHERE e.account_id = ?1 AND e.mailbox = 'spam' AND e.is_deleted = 0
                 ORDER BY e.timestamp DESC LIMIT ?2",
                // Ordinary inbox mail. A little of it is spam the server missed;
                // Naive Bayes tolerates that level of contamination, and the
                // alternative — only mail in replied-to threads — yields far too
                // few negatives on a young account.
                "SELECT e.subject, e.snippet, e.sender_email, h.x_mailer
                 FROM emails e LEFT JOIN email_headers h ON h.email_id = e.id
                 WHERE e.account_id = ?1 AND e.mailbox = 'inbox' AND e.is_sent = 0 AND e.is_deleted = 0
                 ORDER BY e.timestamp DESC LIMIT ?2",
            ),
            ModelAxis::Graymail => (
                // Bulk markers present and the user has never replied: the
                // working definition of mail they do not want.
                "SELECT e.subject, e.snippet, e.sender_email, h.x_mailer
                 FROM emails e JOIN email_headers h ON h.email_id = e.id
                 WHERE e.account_id = ?1 AND e.is_sent = 0 AND e.is_deleted = 0
                   AND (h.list_id IS NOT NULL OR h.list_unsubscribe IS NOT NULL)
                   AND NOT EXISTS (
                     SELECT 1 FROM emails s
                     WHERE s.account_id = e.account_id AND s.thread_id = e.thread_id AND s.is_sent = 1
                   )
                 ORDER BY e.timestamp DESC LIMIT ?2",
                // Mail with no bulk markers at all, or that the user answered.
                "SELECT e.subject, e.snippet, e.sender_email, h.x_mailer
                 FROM emails e LEFT JOIN email_headers h ON h.email_id = e.id
                 WHERE e.account_id = ?1 AND e.is_sent = 0 AND e.is_deleted = 0
                   AND (
                     (h.list_id IS NULL AND h.list_unsubscribe IS NULL)
                     OR EXISTS (
                       SELECT 1 FROM emails s
                       WHERE s.account_id = e.account_id AND s.thread_id = e.thread_id AND s.is_sent = 1
                     )
                   )
                 ORDER BY e.timestamp DESC LIMIT ?2",
            ),
        };

        let mut rows: Vec<TrainingRow> = Vec::new();
        for (sql, positive) in [(positive_sql, true), (negative_sql, false)] {
            let mut stmt = conn.prepare(sql)?;
            let iter = stmt.query_map(params![account_id, cap], |r| {
                Ok(TrainingRow {
                    subject: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    snippet: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    sender_email: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    x_mailer: r.get(3)?,
                    positive,
                    weight: 1,
                })
            })?;
            for row in iter {
                rows.push(row?);
            }
        }

        // Explicit corrections, last so they are never truncated away.
        let mut stmt = conn.prepare(
            "SELECT e.subject, e.snippet, e.sender_email, h.x_mailer, j.user_override, j.primary_kind
             FROM email_junk j
             JOIN emails e ON e.id = j.email_id
             LEFT JOIN email_headers h ON h.email_id = e.id
             WHERE j.account_id = ?1 AND j.user_override IS NOT NULL",
        )?;
        let iter = stmt.query_map(params![account_id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        for row in iter {
            let (subject, snippet, sender_email, x_mailer, override_value, primary_kind) = row?;
            // "Not junk" is a statement about the message as a whole, so it is a
            // negative for every trainable axis. "Junk" only speaks for the axis
            // that flagged it — calling a newsletter spam would poison the spam
            // model with ordinary marketing copy.
            let positive = match override_value.as_str() {
                "not_junk" => false,
                "junk" if primary_kind == axis.as_str() => true,
                _ => continue,
            };
            rows.push(TrainingRow {
                subject,
                snippet,
                sender_email,
                x_mailer,
                positive,
                weight: FEEDBACK_WEIGHT,
            });
        }

        Ok(rows)
    }
}

impl Database {
    /// Training rows for hand-labelled messages from the private golden set.
    ///
    /// Weighted like a user override: a human looked at each of these, which
    /// makes them worth far more than a label inferred from a folder. They are
    /// also the only source that can separate unwanted cold outreach from a
    /// welcome cold intro — the deterministic layer sees the same structure in
    /// both, so that judgement can only come from the user's own decisions.
    pub fn golden_training_rows(
        &self,
        account_id: &str,
        axis: ModelAxis,
        labels: &[crate::services::junk::golden::GoldenEntry],
    ) -> Result<Vec<TrainingRow>> {
        use crate::services::junk::golden::GoldenLabel;

        let relevant: Vec<(&str, bool)> = labels
            .iter()
            .filter(|e| e.account_id == account_id)
            .filter_map(|e| {
                // A `legit` label is a negative for every axis. A junk label is
                // only a positive for its own axis: calling a newsletter spam
                // would teach the spam model that ordinary marketing copy is
                // fraud.
                let positive = match (e.label, axis) {
                    (GoldenLabel::Legit, _) => false,
                    (GoldenLabel::Spam, ModelAxis::Spam) => true,
                    (GoldenLabel::Graymail, ModelAxis::Graymail) => true,
                    // Phishing trains no model by design; it is still a negative
                    // for neither axis, so it is skipped entirely.
                    _ => return None,
                };
                Some((e.email_id.as_str(), positive))
            })
            .collect();

        if relevant.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.reader();
        let mut out = Vec::new();
        for (email_id, positive) in relevant {
            let row = conn
                .query_row(
                    "SELECT e.subject, e.snippet, e.sender_email, h.x_mailer
                     FROM emails e LEFT JOIN email_headers h ON h.email_id = e.id
                     WHERE e.id = ?1",
                    params![email_id],
                    |r| {
                        Ok(TrainingRow {
                            subject: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            snippet: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            sender_email: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            x_mailer: r.get(3)?,
                            positive,
                            weight: FEEDBACK_WEIGHT,
                        })
                    },
                )
                .optional()?;
            if let Some(row) = row {
                out.push(row);
            }
        }
        Ok(out)
    }
}

impl Database {
    pub fn junk_model_trained_at(&self, account_id: &str, axis: ModelAxis) -> Result<Option<i64>> {
        let conn = self.reader();
        Ok(conn
            .query_row(
                "SELECT trained_at FROM junk_model WHERE account_id = ?1 AND axis = ?2",
                params![account_id, axis.as_str()],
                |r| r.get::<_, i64>(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::junk::model::NaiveBayes;

    fn seeded() -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('a1', 'imap', 'u@example.com', 'U', 0)",
                [],
            )
            .expect("account");
        db
    }

    fn insert(db: &Database, id: &str, mailbox: &str, is_sent: bool, thread: &str) {
        db.insert_emails_batch(&[Email {
            id: id.to_string(),
            account_id: "a1".into(),
            thread_id: thread.into(),
            message_id: None,
            subject: format!("Subject {id}"),
            sender: "S".into(),
            sender_email: format!("{id}@other.example"),
            recipients: vec![],
            cc: vec![],
            body: "body".into(),
            snippet: "snippet text here".into(),
            timestamp: 1_700_000_000,
            is_read: false,
            is_sent,
            triage_status: None,
            category: "primary".into(),
            mailbox: mailbox.into(),
            headers: None,
        }])
        .expect("insert");
    }

    #[test]
    fn a_model_round_trips_and_versions_increment() {
        let db = seeded();
        let model = NaiveBayes {
            n_pos: 50,
            n_neg: 60,
            ..NaiveBayes::default()
        };

        let v1 = db.save_junk_model("a1", ModelAxis::Spam, &model, 100).expect("save");
        assert_eq!(v1, 1);
        let v2 = db.save_junk_model("a1", ModelAxis::Spam, &model, 200).expect("resave");
        assert_eq!(v2, 2, "each retrain gets a new version so re-scores can be targeted");

        let (loaded, version) = db
            .load_junk_model("a1", ModelAxis::Spam)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.n_pos, 50);
        assert_eq!(version, 2);
    }

    #[test]
    fn an_untrained_axis_loads_as_none() {
        let db = seeded();
        assert!(db.load_junk_model("a1", ModelAxis::Graymail).expect("load").is_none());
    }

    #[test]
    fn the_spam_folder_supplies_positives_and_the_inbox_negatives() {
        let db = seeded();
        insert(&db, "s1", "spam", false, "t1");
        insert(&db, "i1", "inbox", false, "t2");

        let rows = db.get_junk_training_rows("a1", ModelAxis::Spam, 100).expect("rows");
        assert_eq!(rows.iter().filter(|r| r.positive).count(), 1);
        assert_eq!(rows.iter().filter(|r| !r.positive).count(), 1);
    }

    #[test]
    fn a_user_correction_carries_more_weight_than_an_inferred_label() {
        let db = seeded();
        insert(&db, "i1", "inbox", false, "t1");
        db.set_junk_override("i1", "a1", Some("not_junk"), 100)
            .expect("override");

        let rows = db.get_junk_training_rows("a1", ModelAxis::Spam, 100).expect("rows");
        let feedback = rows.iter().find(|r| r.weight > 1).expect("feedback row present");
        assert!(!feedback.positive);
        assert_eq!(feedback.weight, FEEDBACK_WEIGHT);
    }

    #[test]
    fn a_junk_correction_only_trains_the_axis_that_flagged_it() {
        // Marking a newsletter as junk must not teach the SPAM model that
        // ordinary marketing copy is spam — the user was agreeing about
        // graymail, not about fraud.
        let db = seeded();
        insert(&db, "i1", "inbox", false, "t1");
        db.upsert_junk_verdict(
            "i1",
            "a1",
            &crate::services::junk::verdict::JunkVerdict {
                primary: crate::services::junk::verdict::JunkKind::Graymail,
                ..crate::services::junk::verdict::JunkVerdict::clean()
            },
            1,
            100,
        )
        .expect("verdict");
        db.set_junk_override("i1", "a1", Some("junk"), 200).expect("override");

        let spam_rows = db.get_junk_training_rows("a1", ModelAxis::Spam, 100).expect("rows");
        assert!(
            !spam_rows.iter().any(|r| r.positive && r.weight == FEEDBACK_WEIGHT),
            "a graymail correction must not become a spam positive"
        );

        let gray_rows = db.get_junk_training_rows("a1", ModelAxis::Graymail, 100).expect("rows");
        assert!(gray_rows.iter().any(|r| r.positive && r.weight == FEEDBACK_WEIGHT));
    }

    #[test]
    fn training_rows_carry_the_projection_the_tokenizer_expects() {
        // Subject + snippet + sender, and no body: a full retrain must never
        // load message bodies.
        let db = seeded();
        insert(&db, "i1", "inbox", false, "t1");
        let rows = db.get_junk_training_rows("a1", ModelAxis::Spam, 10).expect("rows");
        let row = rows.first().expect("one row");
        assert_eq!(row.subject, "Subject i1");
        assert_eq!(row.snippet, "snippet text here");
        assert_eq!(row.sender_email, "i1@other.example");
    }
}
