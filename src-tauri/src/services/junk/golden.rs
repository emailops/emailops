//! The private golden set: human ground truth over the user's own mailbox.
//!
//! # Why this exists
//!
//! The synthetic corpus (`src-tauri/evals/junk/cases/`) stops regressions. It
//! cannot *find* anything: in one session the real mailbox surfaced six defects
//! the synthetic cases were structurally incapable of catching — a header whose
//! presence was read as a verdict, a display-name convention mistaken for an
//! address, an ESP bounce domain read as impersonation, English-only lexicons on
//! a Spanish mailbox, an empty message body, and newsletter preheaders scored as
//! concealed filler. Real data finds bugs; the corpus keeps them fixed.
//!
//! # Privacy
//!
//! The file stores **pointers only** — `email_id`, `account_id`, a label, and a
//! source. No subject, no address, no body ever leaves SQLite. The directory is
//! gitignored (`private-evals/*`), and nothing here is safe to commit even so.
//!
//! # The circularity trap
//!
//! A label derived from a signal the detector already reads measures **nothing**:
//! it grades the detector against its own input. That rules out most of the
//! tempting "free" labels:
//!
//! | Candidate label source          | Usable? | Why |
//! |---------------------------------|---------|-----|
//! | `mailbox = 'spam'`              | **yes** | The provider moved it. Nothing in `judge()` reads `mailbox`. |
//! | The user's own override         | **yes** | Human judgement, the strongest label there is. |
//! | Hand-labelled by the user       | **yes** | |
//! | `X-Spam-Status: Yes`            | no      | `server_spam_flag` reads exactly this header. |
//! | `List-Id` present, never replied| no      | The graymail axis is *defined* by those signals. |
//! | Threads the user replied to     | no      | Feeds the engagement suppressors directly. |
//!
//! Bootstrapping therefore seeds only from the two independent sources and
//! leaves the rest to be labelled by hand. That produces a smaller starting set
//! than the circular version — and a meaningful one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::services::clock::now_secs;

/// Ground truth for one message. Deliberately a single label rather than three
/// axes: a human looking at a message decides what it *is*, and forcing three
/// independent judgements per message makes hand-labelling unbearable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldenLabel {
    Legit,
    Spam,
    Phishing,
    Graymail,
}

impl GoldenLabel {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "legit" | "ok" | "good" => Some(GoldenLabel::Legit),
            "spam" | "junk" => Some(GoldenLabel::Spam),
            "phishing" | "phish" => Some(GoldenLabel::Phishing),
            "graymail" | "gray" | "grey" | "bulk" => Some(GoldenLabel::Graymail),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoldenLabel::Legit => "legit",
            GoldenLabel::Spam => "spam",
            GoldenLabel::Phishing => "phishing",
            GoldenLabel::Graymail => "graymail",
        }
    }

    /// Should the detector have flagged this message at all?
    pub fn is_junk(self) -> bool {
        !matches!(self, GoldenLabel::Legit)
    }
}

/// Where a label came from. Recorded so a measurement can be restricted to
/// labels that are genuinely independent of the detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// The provider filed it under Spam. Independent: `judge()` never reads
    /// `emails.mailbox`.
    ProviderFolder,
    /// The user pressed "not junk" or "confirm junk" in the app.
    UserOverride,
    /// Reviewed by hand.
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenEntry {
    pub email_id: String,
    pub account_id: String,
    pub label: GoldenLabel,
    pub source: LabelSource,
    pub labeled_at: i64,
}

/// Default location. Gitignored via `private-evals/*`.
pub fn default_path() -> PathBuf {
    PathBuf::from("private-evals/junk/labels.jsonl")
}

/// Read the label file. A missing file is an empty set, not an error.
pub fn load(path: &Path) -> Result<Vec<GoldenEntry>> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (n, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match serde_json::from_str::<GoldenEntry>(line) {
            Ok(entry) => out.push(entry),
            // One malformed line must not discard a hand-built label set.
            Err(e) => {
                crate::services::logger::log("warn", "system", format!("golden set: skipping line {} ({e})", n + 1))
            }
        }
    }
    Ok(out)
}

/// Write the whole set, newest label per email id winning.
pub fn save(path: &Path, entries: &[GoldenEntry]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::IoError(e.to_string()))?;
    }
    let mut body = String::new();
    body.push_str("# EmailOps junk golden set — POINTERS ONLY, never content.\n");
    body.push_str("# Gitignored. See services::junk::golden for the circularity rules.\n");
    for entry in entries {
        body.push_str(&serde_json::to_string(entry).map_err(|e| AppError::InvalidInput(e.to_string()))?);
        body.push('\n');
    }
    std::fs::write(path, body).map_err(|e| AppError::IoError(e.to_string()))?;
    Ok(())
}

/// Merge new entries into an existing set. A later label replaces an earlier one
/// for the same message, and a manual label is never overwritten by a bootstrap.
pub fn merge(existing: Vec<GoldenEntry>, incoming: Vec<GoldenEntry>) -> Vec<GoldenEntry> {
    let mut by_id: BTreeMap<String, GoldenEntry> = BTreeMap::new();
    for entry in existing {
        by_id.insert(entry.email_id.clone(), entry);
    }
    for entry in incoming {
        let keep_existing = by_id
            .get(&entry.email_id)
            .is_some_and(|prev| prev.source == LabelSource::Manual && entry.source != LabelSource::Manual);
        if !keep_existing {
            by_id.insert(entry.email_id.clone(), entry);
        }
    }
    by_id.into_values().collect()
}

/// Seed labels from the two sources that are independent of the detector.
///
/// Everything else — server spam headers, list markers, reply history — is an
/// input to `judge()`, so a label derived from it would grade the detector
/// against its own reasoning. See the module docs.
pub fn bootstrap(db: &Arc<Database>, account_id: &str, limit: usize) -> Result<Vec<GoldenEntry>> {
    let now = now_secs();
    let mut out = Vec::new();

    {
        let conn = db.reader();
        // The provider's own filing decision.
        let mut stmt = conn.prepare(
            "SELECT id FROM emails
             WHERE account_id = ?1 AND mailbox = 'spam' AND is_deleted = 0
             ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![account_id, limit as i64], |r| r.get::<_, String>(0))?;
        for row in rows {
            out.push(GoldenEntry {
                email_id: row?,
                account_id: account_id.to_string(),
                label: GoldenLabel::Spam,
                source: LabelSource::ProviderFolder,
                labeled_at: now,
            });
        }

        // The user's own corrections outrank everything.
        let mut stmt = conn.prepare(
            "SELECT email_id, user_override FROM email_junk
             WHERE account_id = ?1 AND user_override IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![account_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (email_id, override_value) = row?;
            let label = match override_value.as_str() {
                "not_junk" => GoldenLabel::Legit,
                "junk" => GoldenLabel::Spam,
                _ => continue,
            };
            out.push(GoldenEntry {
                email_id,
                account_id: account_id.to_string(),
                label,
                source: LabelSource::UserOverride,
                labeled_at: now,
            });
        }
    }

    Ok(out)
}

/// Messages with no label yet — the review queue.
///
/// `random` spreads the sample across the whole mailbox instead of taking the
/// newest first. That matters more than it looks: a mailbox going through a spam
/// wave has a recent window that is almost entirely junk, so newest-first
/// review produces a label set with hundreds of positives and a handful of
/// negatives — and `legit_fp_rate`, the only number that decides whether the
/// feature is usable, is computed *from the negatives*. A biased sample makes it
/// unmeasurable no matter how many labels are added.
pub fn unlabelled(
    db: &Arc<Database>,
    account_id: &str,
    known: &[GoldenEntry],
    limit: usize,
    random: bool,
) -> Result<Vec<String>> {
    let labelled: std::collections::HashSet<&str> = known.iter().map(|e| e.email_id.as_str()).collect();
    let conn = db.reader();
    let order = if random { "RANDOM()" } else { "timestamp DESC" };
    let sql = format!(
        "SELECT id FROM emails
         WHERE account_id = ?1 AND is_deleted = 0 AND mailbox IN ('inbox', 'spam')
         ORDER BY {order} LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![account_id, (limit * 4) as i64], |r| {
        r.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        let id = row?;
        if !labelled.contains(id.as_str()) {
            out.push(id);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

/// Confusion counts against the golden set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoldenReport {
    pub total: usize,
    /// Labelled junk and flagged.
    pub true_pos: usize,
    /// Labelled legit but flagged — the number that decides whether the feature
    /// is usable.
    pub false_pos: usize,
    pub true_neg: usize,
    pub false_neg: usize,
    /// Per-source counts, so a reader can see how much of the verdict rests on
    /// independent labels versus the user's own corrections.
    pub by_source: BTreeMap<String, usize>,
    /// Ids the detector flagged that the golden set calls legitimate. The list
    /// to actually go and look at.
    pub false_positive_ids: Vec<String>,
    pub false_negative_ids: Vec<String>,
}

impl GoldenReport {
    pub fn precision(&self) -> Option<f64> {
        let flagged = self.true_pos + self.false_pos;
        (flagged > 0).then(|| self.true_pos as f64 / flagged as f64)
    }

    pub fn recall(&self) -> Option<f64> {
        let positives = self.true_pos + self.false_neg;
        (positives > 0).then(|| self.true_pos as f64 / positives as f64)
    }

    /// Of the messages the human called legitimate, what fraction did the
    /// detector badge? The headline number.
    pub fn legit_fp_rate(&self) -> Option<f64> {
        let negatives = self.false_pos + self.true_neg;
        (negatives > 0).then(|| self.false_pos as f64 / negatives as f64)
    }
}

/// Score the stored verdicts against the golden set.
///
/// Reads persisted verdicts rather than re-running `judge()`, so what is
/// measured is exactly what the user saw.
pub fn measure(db: &Arc<Database>, entries: &[GoldenEntry]) -> Result<GoldenReport> {
    let mut report = GoldenReport::default();
    if entries.is_empty() {
        return Ok(report);
    }

    let ids: Vec<String> = entries.iter().map(|e| e.email_id.clone()).collect();
    let verdicts = db.get_junk_verdicts_batch(&ids)?;

    for entry in entries {
        // A message with no verdict has not been scored; counting it as "clean"
        // would credit the detector for silence it never produced.
        let Some(verdict) = verdicts.get(&entry.email_id) else {
            continue;
        };
        report.total += 1;
        *report
            .by_source
            .entry(format!("{:?}", entry.source).to_lowercase())
            .or_insert(0) += 1;

        match (entry.label.is_junk(), verdict.is_flagged()) {
            (true, true) => report.true_pos += 1,
            (false, true) => {
                report.false_pos += 1;
                report.false_positive_ids.push(entry.email_id.clone());
            }
            (false, false) => report.true_neg += 1,
            (true, false) => {
                report.false_neg += 1;
                report.false_negative_ids.push(entry.email_id.clone());
            }
        }
    }

    Ok(report)
}

/// Turn labelled messages into a standalone corpus in the same YAML shape as
/// `src-tauri/evals/junk/cases/`.
///
/// # Why this exists
///
/// `labels.jsonl` stores pointers, which makes it privacy-cheap but fragile: an
/// IMAP message id is `{account}::{uid}`, and UIDs are **not stable** — a
/// `UIDVALIDITY` change on the server invalidates every one of them at once, as
/// does rebuilding the local database. A label set that silently dangles is not
/// a label set.
///
/// Exporting the message alongside its label fixes that and buys something else:
/// the result is a corpus, so the existing `junk_eval` harness — confusion
/// matrix, per-axis rates, threshold sweep, CI gates — runs over real mail with
/// no new machinery.
///
/// # This file contains real mail
///
/// Unlike `labels.jsonl`, the export carries subjects, addresses, headers and
/// bodies. It lives under the gitignored `private-evals/` tree and must never be
/// committed, pasted into an issue, or shared. Nothing derived from it belongs
/// in `src-tauri/evals/junk/cases/`, which is the public corpus — paraphrase
/// into synthetic equivalents instead.
pub const EXPORT_WARNING: &str = "\
# ⚠️  REAL MAIL — NEVER COMMIT, PASTE OR SHARE THIS FILE.
#
# Exported from the local mailbox so the golden set survives UID changes and
# database rebuilds. Contains real subjects, addresses, headers and bodies.
# `private-evals/` is gitignored; keep it that way.
#
# To contribute a case upstream, paraphrase it into a synthetic equivalent under
# src-tauri/evals/junk/cases/ that preserves the technical shape and drops every
# identifying detail.
";

/// Rebuild an RFC 5322 header block from the captured subset.
///
/// Round-trips correctly because `sync::header_capture::capture` only ever reads
/// these fields — re-parsing the reconstruction yields the same `RawHeaders`.
fn header_block(headers: Option<&crate::models::headers::RawHeaders>, subject: &str, from: &str) -> String {
    let mut out = String::new();
    let Some(h) = headers else {
        return out;
    };
    let mut push = |name: &str, value: Option<&str>| {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            out.push_str(&format!("{name}: {}\n", v.replace('\n', " ")));
        }
    };
    push("From", h.from_raw.as_deref().or(Some(from)));
    push("Subject", Some(subject));
    push("Reply-To", h.reply_to.as_deref());
    push("Return-Path", h.return_path.as_deref());
    push("To", h.to_raw.as_deref());
    push("Authentication-Results", h.auth_results.as_deref());
    push("Received-SPF", h.received_spf.as_deref());
    push("List-Id", h.list_id.as_deref());
    push("List-Unsubscribe", h.list_unsubscribe.as_deref());
    push("List-Unsubscribe-Post", h.list_unsubscribe_post.as_deref());
    push("Precedence", h.precedence.as_deref());
    push("X-Mailer", h.x_mailer.as_deref());
    push("Content-Type", h.content_type.as_deref());
    for domain in &h.dkim_domains {
        out.push_str(&format!("DKIM-Signature: v=1; d={domain}; s=x; b=x\n"));
    }
    if let Some(spam) = &h.spam_headers {
        for line in spam.lines() {
            if let Some((name, value)) = line.split_once(':') {
                out.push_str(&format!("{}: {}\n", name.trim(), value.trim()));
            }
        }
    }
    // Received order matters (bottom-most is the origin hop), and only the count
    // plus the origin were captured — reconstruct that shape.
    for i in 0..h.received_count {
        if i + 1 == h.received_count {
            if let Some(first) = &h.first_received {
                out.push_str(&format!("Received: {}\n", first.replace('\n', " ")));
                continue;
            }
        }
        out.push_str("Received: from relay.example by mx.example\n");
    }
    out
}

/// Per-axis expectation for one label. A junk label takes a position only on its
/// own axis; claiming the other two are clean would assert more than the human
/// actually decided.
fn expectation_yaml(label: GoldenLabel) -> String {
    match label {
        GoldenLabel::Legit => "    phishing: clean\n    spam: clean\n    graymail: clean\n".into(),
        GoldenLabel::Spam => "    spam: junk\n".into(),
        GoldenLabel::Phishing => "    phishing: junk\n".into(),
        GoldenLabel::Graymail => "    graymail: junk\n".into(),
    }
}

fn yaml_block(value: &str, indent: &str) -> String {
    value
        .lines()
        .map(|l| format!("{indent}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Export every labelled message that still exists locally.
///
/// Returns the number of cases written. Messages whose row has gone are skipped
/// — and reported, because a shrinking export is exactly the dangling-pointer
/// problem this exists to solve.
pub fn export_cases(db: &Arc<Database>, entries: &[GoldenEntry], out_path: &Path) -> Result<(usize, usize)> {
    let mut yaml = String::from(EXPORT_WARNING);
    let mut written = 0usize;
    let mut missing = 0usize;

    // Grouped per account so the contact reference set is built once.
    let mut by_account: BTreeMap<String, Vec<&GoldenEntry>> = BTreeMap::new();
    for entry in entries {
        by_account.entry(entry.account_id.clone()).or_default().push(entry);
    }

    for (account_id, account_entries) in by_account {
        let Ok(ctx) = crate::services::junk::signals::AccountContext::load(db, &account_id) else {
            continue;
        };
        for entry in account_entries {
            let Some(email) = db.get_email_by_id(&entry.email_id)? else {
                missing += 1;
                continue;
            };
            let headers = db
                .get_email_headers_batch(std::slice::from_ref(&email.id))?
                .remove(&email.id);
            let body = db.get_email_body(&email.id).unwrap_or_default();
            let attachments: Vec<String> = db
                .get_email_attachment_metas(&email.id)
                .unwrap_or_default()
                .into_iter()
                .map(|a| a.filename)
                .collect();

            let block = header_block(headers.as_ref(), &email.subject, &email.sender_email);
            // A stable, mailbox-independent id: the label survives a UID change.
            let case_id = format!("real-{}-{:08x}", entry.label.as_str(), fnv(&email.id));

            yaml.push_str(&format!("\n- id: {case_id}\n  tier: private\n"));
            yaml.push_str("  raw_headers: |\n");
            yaml.push_str(&yaml_block(&block, "    "));
            yaml.push('\n');
            yaml.push_str("  body: |\n");
            // Bodies can be enormous; the content layer only reads the first
            // couple of thousand characters anyway.
            let trimmed: String = body.chars().take(4_000).collect();
            yaml.push_str(&yaml_block(&trimmed, "    "));
            yaml.push('\n');

            if !ctx.known_contact_domains.is_empty() {
                yaml.push_str(&format!(
                    "  known_contact_domains: {}\n",
                    serde_json::to_string(&ctx.known_contact_domains).unwrap_or_else(|_| "[]".into())
                ));
            }
            if !ctx.known_contact_names.is_empty() {
                yaml.push_str(&format!(
                    "  known_contact_names: {}\n",
                    serde_json::to_string(&ctx.known_contact_names).unwrap_or_else(|_| "[]".into())
                ));
            }
            if let Some(authserv) = &ctx.trusted_authserv {
                yaml.push_str(&format!("  trusted_authserv: {authserv}\n"));
            } else {
                yaml.push_str("  trusted_authserv: null\n");
            }
            if !attachments.is_empty() {
                yaml.push_str(&format!(
                    "  attachment_names: {}\n",
                    serde_json::to_string(&attachments).unwrap_or_else(|_| "[]".into())
                ));
            }
            yaml.push_str(&format!("  provider_category: {}\n", email.category));
            yaml.push_str(&format!(
                "  sender_engaged: {}\n",
                db.is_sender_engaged(&account_id, &email.sender_email).unwrap_or(false)
            ));
            yaml.push_str(&format!(
                "  own_thread: {}\n",
                db.thread_has_own_message(&account_id, &email.thread_id)
                    .unwrap_or(false)
            ));
            // Every signal the planner reads has to travel with the case. This
            // one was missed on the first pass and the exported corpus reported
            // seven graymail false positives the live detector did not have:
            // the cases fell back to the default and re-fired a rule that
            // depends on it. An export that omits an input is measuring a
            // different detector than the one that runs.
            yaml.push_str(&format!(
                "  sender_message_count: {}\n",
                db.count_messages_from_sender(&account_id, &email.sender_email)
                    .unwrap_or(0)
            ));
            yaml.push_str("  expect:\n");
            yaml.push_str(&expectation_yaml(entry.label));
            written += 1;
        }
    }

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::IoError(e.to_string()))?;
    }
    std::fs::write(out_path, yaml).map_err(|e| AppError::IoError(e.to_string()))?;
    Ok((written, missing))
}

fn fnv(s: &str) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    (hash & 0xffff_ffff) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(id: &str, label: GoldenLabel, source: LabelSource) -> GoldenEntry {
        GoldenEntry {
            email_id: id.into(),
            account_id: "a1".into(),
            label,
            source,
            labeled_at: 100,
        }
    }

    #[test]
    fn a_missing_label_file_is_an_empty_set_not_an_error() {
        let dir = TempDir::new().expect("tmp");
        assert!(load(&dir.path().join("nope.jsonl")).expect("load").is_empty());
    }

    #[test]
    fn labels_round_trip_through_the_file() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("labels.jsonl");
        let entries = vec![
            entry("e1", GoldenLabel::Spam, LabelSource::ProviderFolder),
            entry("e2", GoldenLabel::Legit, LabelSource::Manual),
        ];
        save(&path, &entries).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].label, GoldenLabel::Spam);
    }

    #[test]
    fn the_file_never_contains_message_content() {
        // The whole privacy contract. If this ever fails, the golden set has
        // started carrying the user's mail.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("labels.jsonl");
        save(&path, &[entry("e1", GoldenLabel::Spam, LabelSource::ProviderFolder)]).expect("save");
        let raw = std::fs::read_to_string(&path).expect("read");
        for field in ["subject", "sender", "body", "snippet", "@"] {
            assert!(!raw.contains(field), "golden set leaked {field:?}: {raw}");
        }
    }

    #[test]
    fn a_malformed_line_does_not_discard_the_rest() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("labels.jsonl");
        std::fs::write(&path, "{ not json\n{\"email_id\":\"e1\",\"account_id\":\"a1\",\"label\":\"spam\",\"source\":\"manual\",\"labeled_at\":1}\n")
            .expect("write");
        assert_eq!(load(&path).expect("load").len(), 1);
    }

    #[test]
    fn a_hand_label_survives_a_later_bootstrap() {
        // Bootstrapping is a convenience; a human decision outranks it and must
        // never be silently overwritten by a re-run.
        let existing = vec![entry("e1", GoldenLabel::Legit, LabelSource::Manual)];
        let incoming = vec![entry("e1", GoldenLabel::Spam, LabelSource::ProviderFolder)];
        let merged = merge(existing, incoming);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].label, GoldenLabel::Legit);
        assert_eq!(merged[0].source, LabelSource::Manual);
    }

    #[test]
    fn a_hand_label_replaces_an_earlier_hand_label() {
        let merged = merge(
            vec![entry("e1", GoldenLabel::Legit, LabelSource::Manual)],
            vec![entry("e1", GoldenLabel::Phishing, LabelSource::Manual)],
        );
        assert_eq!(merged[0].label, GoldenLabel::Phishing);
    }

    #[test]
    fn every_non_legit_label_counts_as_junk() {
        assert!(GoldenLabel::Spam.is_junk());
        assert!(GoldenLabel::Phishing.is_junk());
        assert!(GoldenLabel::Graymail.is_junk());
        assert!(!GoldenLabel::Legit.is_junk());
    }

    #[test]
    fn report_rates_are_undefined_rather_than_zero_when_there_is_nothing_to_divide() {
        let empty = GoldenReport::default();
        assert_eq!(empty.precision(), None);
        assert_eq!(empty.recall(), None);
        assert_eq!(empty.legit_fp_rate(), None);
    }

    #[test]
    fn the_false_positive_list_is_the_actionable_output() {
        let report = GoldenReport {
            total: 3,
            true_pos: 1,
            false_pos: 1,
            true_neg: 1,
            false_negative_ids: vec![],
            false_positive_ids: vec!["e2".into()],
            ..GoldenReport::default()
        };
        let fp = report.legit_fp_rate().expect("defined");
        assert!((fp - 0.5).abs() < 1e-9);
        assert_eq!(report.false_positive_ids, vec!["e2".to_string()]);
    }
}

// ── Self-contained export ────────────────────────────────────────────────────
