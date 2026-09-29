//! Junk detection: spam, phishing/BEC and graymail.
//!
//! Local-flag-only by design — this module never moves a message on the server.
//! It computes a score plus reason codes, persists them locally, and the UI
//! deprioritizes accordingly. See `docs/DECISIONS.md`.
//!
//! Layout follows the pure-planner / thin-executor split:
//!   `verdict`   — the pure planner (`judge`) and the domain types
//!   `auth`      — pure: interpreting `Authentication-Results`
//!   `lookalike` — pure: typosquat / cousin-domain / homoglyph detection
//!   `content`   — pure: link, attachment and text signals
//!   `signals`   — executor: materializes `JunkSignals` from SQLite
//!   (this file) — executor: batching, persistence, feedback, backfill
//!
//! The measurement gate lives in `evals::junk` and runs as `make eval-junk`.
//! Every behaviour change here is a diff on that report.

#[cfg(test)]
mod architecture_tests;
pub mod auth;
pub mod config;
pub mod content;
pub mod golden;
pub mod lookalike;
pub mod model;
pub mod signals;
pub mod tokens;
pub mod verdict;

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::services::clock::now_secs;
use crate::services::junk::signals::AccountContext;
use crate::services::junk::verdict::{judge, JunkAxis, JunkKind, Weights};

/// Messages scored per `score_new_emails` call.
///
/// Scoring is pure computation — no model, no network — so this can be
/// generous; the bound exists to keep one sync's follow-up work finite.
const SCORE_BATCH: usize = 500;

/// Bumped whenever the deterministic weights change, so a later pass can tell
/// which rows predate the change and re-score selectively.
pub const DETERMINISTIC_MODEL_VERSION: i64 = 1;

/// Is the phishing axis allowed to surface to the user?
///
/// **Default: off.** The other two axes are measured — precision 1.000 over
/// dozens of hand-labelled messages — but the phishing axis has almost no
/// ground truth behind it, and it is the one that renders a red "this may be
/// impersonating someone" banner. An unvalidated accusation of fraud on a
/// legitimate message costs more trust than every correct graymail call earns.
///
/// The gate lives here, in the executor, and NOT inside `judge()`. The planner
/// keeps scoring all three axes so the eval harness can go on measuring
/// phishing against the golden set — which is the only way it will ever earn
/// being switched on. Suppressing it in the planner would hide the very number
/// needed to validate it.
pub fn is_phishing_enabled(db: &Arc<Database>) -> bool {
    config::get_config(db).phishing_enabled
}

/// Is junk detection switched on for this install?
pub fn is_enabled(db: &Arc<Database>) -> bool {
    config::get_config(db).enabled
}

/// Score every message in the account that has no verdict yet.
///
/// Returns how many were scored. Called from the post-sync follow-up queue.
pub async fn score_new_emails(db: &Arc<Database>, account_id: &str) -> Result<usize> {
    if !is_enabled(db) {
        return Ok(0);
    }
    // Same age window as the other AI pipelines (Settings → AI limits).
    let min_timestamp = db.ai_processing_min_timestamp(account_id, now_secs())?.unwrap_or(0);

    let ids = db.get_unscored_junk_email_ids(account_id, SCORE_BATCH, min_timestamp)?;
    if ids.is_empty() {
        return Ok(0);
    }
    score_ids(db, account_id, &ids).await
}

/// Score one message by id, whatever its current state.
pub async fn score_email_by_id(db: &Arc<Database>, account_id: &str, email_id: &str) -> Result<usize> {
    score_ids(db, account_id, std::slice::from_ref(&email_id.to_string())).await
}

/// Upper bound on one backfill run, so a huge mailbox cannot occupy the queue
/// indefinitely. The pass is resumable: `get_unscored_junk_email_ids` returns
/// what is still missing, newest first, so the next run picks up where this one
/// stopped without a stored cursor.
const BACKFILL_CEILING: usize = 50_000;

/// Score already-synced mail that predates the feature.
///
/// Newest-first, because recent mail is what the user is actually looking at.
/// Idempotent and interruption-safe — nothing is lost if it stops halfway.
pub async fn backfill_account(db: &Arc<Database>, account_id: &str) -> Result<usize> {
    if !is_enabled(db) {
        return Ok(0);
    }
    let mut total = 0usize;
    while total < BACKFILL_CEILING {
        let ids = db.get_unscored_junk_email_ids(account_id, SCORE_BATCH, 0)?;
        if ids.is_empty() {
            break;
        }
        let scored = score_ids(db, account_id, &ids).await?;
        if scored == 0 {
            // Every id in the batch failed to materialize (deleted mid-run).
            // Stop rather than spin on the same rows forever.
            break;
        }
        total += scored;
        // Yield between batches so a long backfill cannot starve the queue.
        tokio::task::yield_now().await;
    }
    Ok(total)
}

/// Score a specific set of ids against an already-loaded account context.
///
/// Used by the sync loop, which scores each chunk **before** telling the UI the
/// chunk landed. Junk scoring is deterministic — no model load, no network, a
/// handful of indexed reads per message — so unlike classification (an LLM call
/// per email) there is no reason to defer it. Deferring it is what made a
/// message appear in the inbox and then visibly change state a moment later.
///
/// The caller owns the `AccountContext` so a multi-chunk sync builds the contact
/// reference set once instead of once per twenty messages.
pub async fn score_ids_with_context(
    db: &Arc<Database>,
    account_id: &str,
    ctx: &AccountContext,
    ids: &[String],
) -> Result<usize> {
    score_with(db, account_id, ctx, ids).await
}

async fn score_ids(db: &Arc<Database>, account_id: &str, ids: &[String]) -> Result<usize> {
    // The account-wide contact reference set is identical for every message in
    // the batch, so it is loaded once rather than per email.
    let ctx = AccountContext::load(db, account_id)?;
    score_with(db, account_id, &ctx, ids).await
}

/// Blank the phishing axis without disturbing the other two.
///
/// Used when the axis is switched off: the verdict is still computed and still
/// measurable by the eval, it simply never reaches the user.
fn suppress_phishing(verdict: &mut verdict::JunkVerdict) {
    verdict.phishing = verdict::AxisScore::clean();
    verdict.reasons.retain(|r| r.axis != JunkAxis::Phishing);
    if verdict.primary == JunkKind::Phishing {
        // Fall back to whatever the remaining axes still claim.
        verdict.primary = if verdict.spam.band.is_flagged() {
            JunkKind::Spam
        } else if verdict.graymail.band.is_flagged() {
            JunkKind::Graymail
        } else {
            JunkKind::Legit
        };
    }
}

async fn score_with(db: &Arc<Database>, account_id: &str, ctx: &AccountContext, ids: &[String]) -> Result<usize> {
    let weights = Weights::default();
    let phishing_enabled = is_phishing_enabled(db);
    let now = now_secs();
    let mut scored = 0usize;

    for id in ids {
        let Some(signal_set) = signals::materialize(db, ctx, id)? else {
            continue;
        };
        let mut verdict = judge(&signal_set, &weights);
        if !phishing_enabled {
            suppress_phishing(&mut verdict);
        }
        db.upsert_junk_verdict(id, account_id, &verdict, DETERMINISTIC_MODEL_VERSION, now)?;

        // Mirror a flagged verdict into `email_tags` so the existing tag chips,
        // sidebar counts and smart-filter aggregation pick junk up with no
        // further plumbing. Cleared when a re-score decides the message is fine,
        // otherwise a stale chip would outlive the verdict behind it.
        let flagged = verdict.primary != JunkKind::Legit
            && [JunkAxis::Phishing, JunkAxis::Spam, JunkAxis::Graymail]
                .iter()
                .any(|a| verdict.axis(*a).band.is_flagged());
        if flagged {
            let value = junk_tag_value(verdict.primary);
            if let Err(e) = db.upsert_email_tag(
                id,
                "junk",
                value,
                Some(f64::from(verdict.axis(JunkAxis::Phishing).score)),
            ) {
                log_tag_error("write", id, &e);
            }
            // Push the chip to any open list immediately. Without this the
            // inbox keeps whatever tags it cached when the row first rendered,
            // and a message scored during an in-progress sync shows no badge
            // until the user navigates away and back.
            crate::services::events::emit("email-junk-scored", serde_json::json!({ "emailId": id, "kind": value }));
        } else if let Err(e) = db.delete_email_tag(id, "junk") {
            log_tag_error("clear", id, &e);
        }
        scored += 1;
    }

    Ok(scored)
}

/// Value of the `junk` chip for a verdict kind. `Legit` never gets a chip;
/// callers only reach here for a flagged kind.
fn junk_tag_value(kind: JunkKind) -> &'static str {
    match kind {
        JunkKind::Phishing => "phishing",
        JunkKind::Graymail => "graymail",
        JunkKind::Spam | JunkKind::Legit => "spam",
    }
}

/// A chip write failing leaves the verdict itself stored, so the operation
/// goes on — but the mismatch must be visible in the logs.
fn log_tag_error(action: &str, email_id: &str, e: &crate::models::error::AppError) {
    crate::services::logger::log(
        "warn",
        "system",
        format!("junk: failed to {action} chip for {email_id}: {e}"),
    );
}

/// Labelled rows sampled per class when training.
const TRAIN_SAMPLE_CAP: usize = 10_000;

/// Retrain both statistical models for an account.
///
/// A full retrain, not an incremental update: one pass over subject + snippet +
/// sender for ≤20k rows takes seconds, it is reproducible, and it removes any
/// possibility of the counters drifting out of step with the mailbox.
///
/// Returns `(axis, positives, negatives)` per trained axis. An axis that cannot
/// reach the sample floor is trained anyway — the model simply refuses to vote
/// until it can, which keeps the decision in one place (`NaiveBayes::is_usable`).
pub async fn train_models(db: &Arc<Database>, account_id: &str) -> Result<Vec<(&'static str, u32, u32)>> {
    // Hand labels from the private golden set are the highest-quality training
    // data available — a human looked at each one. They are also the only labels
    // that can teach the difference between junk and legitimate cold outreach,
    // which is structurally invisible to the deterministic layer: both are
    // authenticated first-contact mail with no bulk markers and no bad links.
    let golden_labels = golden::load(&golden::default_path()).unwrap_or_default();

    let mut out = Vec::new();
    for axis in model::ModelAxis::ALL {
        let mut rows = db.get_junk_training_rows(account_id, axis, TRAIN_SAMPLE_CAP)?;
        rows.extend(db.golden_training_rows(account_id, axis, &golden_labels)?);
        let samples: Vec<model::Sample> = rows
            .iter()
            .map(|row| model::Sample {
                features: tokens::features(&tokens::FeatureInput {
                    subject: &row.subject,
                    snippet: &row.snippet,
                    sender_email: &row.sender_email,
                    x_mailer: row.x_mailer.as_deref(),
                }),
                positive: row.positive,
                weight: row.weight,
            })
            .collect();

        let trained = model::train(&samples);
        db.save_junk_model(account_id, axis, &trained, now_secs())?;
        out.push((axis.as_str(), trained.n_pos, trained.n_neg));

        // Yield between axes: each pass walks thousands of rows.
        tokio::task::yield_now().await;
    }
    Ok(out)
}

/// Record the user's correction.
///
/// Deliberately does NOT re-score: the stored override outranks whatever
/// `judge()` would say next, so re-running it would change nothing the user can
/// see and would cost a full signal materialization on a button press. The
/// correction reaches the model on the next `train_models` pass instead, where
/// it carries `FEEDBACK_WEIGHT`.
pub async fn set_feedback(db: &Arc<Database>, account_id: &str, email_id: &str, is_junk: bool) -> Result<()> {
    let verdict = if is_junk { "junk" } else { "not_junk" };
    db.set_junk_override(email_id, account_id, Some(verdict), now_secs())?;
    if is_junk {
        // Show the chip straight away, keeping the scored kind when there is
        // one; a message the scorer thought legit is shown as spam.
        let kind = db
            .get_junk_verdicts_batch(std::slice::from_ref(&email_id.to_string()))?
            .remove(email_id)
            .map(|v| v.primary_kind)
            .filter(|k| *k != JunkKind::Legit)
            .unwrap_or(JunkKind::Spam);
        if let Err(e) = db.upsert_email_tag(email_id, "junk", junk_tag_value(kind), None) {
            log_tag_error("write", email_id, &e);
        }
    } else if let Err(e) = db.delete_email_tag(email_id, "junk") {
        // Drop the chip straight away; the stored override keeps the message
        // clear on every future re-score.
        log_tag_error("clear", email_id, &e);
    }
    Ok(())
}

#[cfg(test)]
mod suppression_tests {
    use super::*;
    use crate::services::junk::verdict::{AxisScore, Band, JunkVerdict, Reason, ReasonCode};

    fn flagged(axis: JunkAxis, primary: JunkKind) -> JunkVerdict {
        let scored = AxisScore {
            score: 0.9,
            band: Band::Junk,
        };
        let mut v = JunkVerdict {
            primary,
            reasons: vec![Reason {
                code: ReasonCode::DmarcFail,
                axis,
                weight: 0.5,
                detail: None,
            }],
            ..JunkVerdict::clean()
        };
        match axis {
            JunkAxis::Phishing => v.phishing = scored,
            JunkAxis::Spam => v.spam = scored,
            JunkAxis::Graymail => v.graymail = scored,
        }
        v
    }

    #[test]
    fn suppressing_phishing_clears_the_axis_and_its_reasons() {
        let mut v = flagged(JunkAxis::Phishing, JunkKind::Phishing);
        suppress_phishing(&mut v);
        assert_eq!(v.phishing.band, Band::Clean);
        assert_eq!(v.primary, JunkKind::Legit);
        assert!(v.reason_codes_for(JunkAxis::Phishing).is_empty());
    }

    #[test]
    fn suppressing_phishing_leaves_the_other_axes_alone() {
        // Switching off an unvalidated axis must not quietly switch off the two
        // that are measured.
        let mut v = flagged(JunkAxis::Spam, JunkKind::Spam);
        v.phishing = AxisScore {
            score: 0.8,
            band: Band::Junk,
        };
        suppress_phishing(&mut v);
        assert_eq!(v.spam.band, Band::Junk);
        assert_eq!(v.primary, JunkKind::Spam, "the spam verdict still stands");
        assert!(!v.reason_codes_for(JunkAxis::Spam).is_empty());
    }

    #[test]
    fn a_message_that_was_only_phishing_falls_back_to_the_next_claim() {
        let mut v = flagged(JunkAxis::Phishing, JunkKind::Phishing);
        v.graymail = AxisScore {
            score: 0.7,
            band: Band::Junk,
        };
        suppress_phishing(&mut v);
        assert_eq!(v.primary, JunkKind::Graymail);
    }
}

#[cfg(test)]
mod feedback_tests {
    use super::*;

    fn seed_email(db: &Database) {
        let conn = db.connection();
        conn.execute(
            "INSERT INTO accounts (id, provider, email, name, created_at)
             VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
             (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
              recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
             VALUES ('e1','acct','t1','Offer','Promo','promo@example.net','example.net',
                     '[]','[]','snip',100,0,'primary',0)",
            [],
        )
        .unwrap();
    }

    fn junk_chip(db: &Database) -> Option<String> {
        db.get_email_tags("e1")
            .unwrap()
            .into_iter()
            .find(|t| t.tag_type == "junk")
            .map(|t| t.tag_value)
    }

    /// Post-sync scoring honours the same AI-processing age window as the
    /// other AI pipelines (it used to read a preference nothing writes).
    #[tokio::test]
    async fn scoring_new_emails_skips_mail_older_than_the_ai_window() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_email(&db); // timestamp 100 — decades old
        db.connection()
            .execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES ('e2','acct','t2','Hello','Friend','friend@example.net','example.net',
                         '[]','[]','snip',?1,0,'primary',0)",
                rusqlite::params![now_secs() - 60],
            )
            .unwrap();
        db.set_preference("junk_enabled", "true").unwrap();
        db.set_preference("ai_max_email_count", "0").unwrap();
        db.set_preference("ai_max_email_age_days", "30").unwrap();

        assert_eq!(score_new_emails(&db, "acct").await.unwrap(), 1);
    }

    /// Marking a message as junk must show the junk chip right away, as
    /// "not junk" removes it right away.
    #[tokio::test]
    async fn junk_feedback_writes_the_chip_and_not_junk_removes_it() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_email(&db);

        set_feedback(&db, "acct", "e1", true).await.unwrap();
        assert_eq!(junk_chip(&db).as_deref(), Some("spam"));

        set_feedback(&db, "acct", "e1", false).await.unwrap();
        assert_eq!(junk_chip(&db), None);
    }
}
