//! Synthetic eval for the AI draft generator: how a draft ends.
//!
//! `draft_eval` scores drafts against the user's real replies, so it needs a
//! private mailbox. This harness needs none: each case in
//! `src-tauri/evals/drafts/cases.yaml` describes a synthetic account (with or
//! without a signature), a thread to answer or a new message to write, and
//! what the end of the draft must look like. It seeds an in-memory DB and runs
//! the real `services::emails::{generate_draft, generate_new_draft}`, so the
//! prompt the app builds is the one under test. Scoring is heuristic (no judge).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::db::Database;
use crate::models::{Account, Email};
use crate::services::emails::{generate_draft, generate_new_draft};

use super::json_report::{EvidenceCheck, ItemEvidence, ItemResult, JsonRunReport};
use super::{EvalError, EvalResult};

const EVAL_ACCOUNT_ID: &str = "eval-draft-acct";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftCaseKind {
    Reply,
    New,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseAccount {
    pub email: String,
    /// Display name; its first word is the name a sign-off would carry.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseSignature {
    pub html: String,
    #[serde(default = "yes")]
    pub use_for_new: bool,
    #[serde(default = "yes")]
    pub use_for_replies: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseMessage {
    pub from_name: String,
    pub from_email: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DraftCase {
    pub id: String,
    pub kind: DraftCaseKind,
    pub account: CaseAccount,
    /// The account's saved signature; absent = none.
    #[serde(default)]
    pub signature: Option<CaseSignature>,
    /// Reply: the thread, oldest first; the last message is answered.
    #[serde(default)]
    pub thread: Vec<CaseMessage>,
    /// New: recipients and subject.
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    /// Whether the end of the draft may carry the sender's name (or a
    /// "[Your name]" placeholder). `false`: the app adds the signature, so a
    /// name there signs the message twice. Absent: report only.
    #[serde(default)]
    pub expect_signed: Option<bool>,
}

pub struct DraftCaseEvalConfig {
    pub model: String,
    pub provider_name: String,
    pub cases_path: PathBuf,
    pub out_dir: PathBuf,
    pub case_filter: Option<String>,
    /// Drafts per case: generation samples at temperature 0.7, so one run
    /// says little about a rate.
    pub repeat: usize,
}

pub fn load_cases(path: &Path) -> EvalResult<Vec<DraftCase>> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&raw)?)
}

/// How a draft ends. Pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEnding {
    /// The sender's first name appears in the last lines.
    pub name_in_tail: bool,
    /// A "[Your name]"-style placeholder appears in the last lines.
    pub placeholder_in_tail: bool,
}

impl DraftEnding {
    /// The draft carries its own sign-off block (a name or a stand-in for one).
    pub fn signed(&self) -> bool {
        self.name_in_tail || self.placeholder_in_tail
    }
}

/// Lines of the end of the draft looked at for a sign-off.
const TAIL_LINES: usize = 3;

const NAME_PLACEHOLDERS: [&str; 6] = ["[your name", "[name]", "[tu nombre", "[nombre", "[sender", "[firma"];

/// Inspect the last non-empty lines of `draft` for `first_name` (as a whole
/// word, any case) or a name placeholder. Pure.
pub fn draft_ending(draft: &str, first_name: &str) -> DraftEnding {
    let tail: Vec<String> = draft
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .take(TAIL_LINES)
        .map(str::to_lowercase)
        .collect();
    let name = first_name.trim().to_lowercase();
    let name_in_tail = !name.is_empty()
        && tail
            .iter()
            .any(|line| line.split(|c: char| !c.is_alphanumeric()).any(|word| word == name));
    // The thread prompt asks for "[placeholder]" where a fact is missing; a
    // line holding only that after the closing stands in for the name.
    let placeholder_in_tail = tail
        .iter()
        .any(|line| NAME_PLACEHOLDERS.iter().any(|p| line.contains(p)) || line == "[placeholder]");
    DraftEnding {
        name_in_tail,
        placeholder_in_tail,
    }
}

fn first_name(full: &str) -> &str {
    full.split_whitespace().next().unwrap_or("")
}

fn seed_case(db: &Database, case: &DraftCase) -> EvalResult<Option<String>> {
    let conn = db.connection();
    conn.execute("DELETE FROM emails WHERE account_id = ?1", [EVAL_ACCOUNT_ID])?;
    conn.execute(
        "DELETE FROM account_signatures WHERE account_id = ?1",
        [EVAL_ACCOUNT_ID],
    )?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", [EVAL_ACCOUNT_ID])?;
    drop(conn);
    db.insert_account(&Account {
        id: EVAL_ACCOUNT_ID.to_string(),
        provider: "gmail".to_string(),
        email: case.account.email.clone(),
        name: case.account.name.clone(),
        created_at: 0,
        sort_order: 0,
        enabled: true,
        sync_from_timestamp: None,
    })?;
    if let Some(sig) = &case.signature {
        db.upsert_account_signature(EVAL_ACCOUNT_ID, &sig.html, sig.use_for_new, sig.use_for_replies, 0)?;
    }
    let thread_id = format!("thread-{}", case.id);
    let mut last_id = None;
    for (i, m) in case.thread.iter().enumerate() {
        let id = format!("{}-{}", case.id, i + 1);
        let is_sent = m.from_email.eq_ignore_ascii_case(&case.account.email);
        db.insert_email(&Email {
            id: id.clone(),
            account_id: EVAL_ACCOUNT_ID.to_string(),
            thread_id: thread_id.clone(),
            message_id: None,
            references: None,
            subject: m.subject.clone(),
            sender: m.from_name.clone(),
            sender_email: m.from_email.clone(),
            recipients: vec![case.account.email.clone()],
            cc: vec![],
            body: m.body.clone(),
            snippet: m.body.chars().take(200).collect(),
            timestamp: 1_700_000_000 + i as i64 * 3600,
            is_read: true,
            is_sent,
            is_starred: false,
            headers: None,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: if is_sent { "sent" } else { "inbox" }.to_string(),
        })?;
        last_id = Some(id);
    }
    Ok(last_id)
}

async fn draft_once(db: &Arc<Database>, case: &DraftCase, reply_to: Option<&str>) -> EvalResult<String> {
    let instructions = case.instructions.as_deref();
    let result = match case.kind {
        DraftCaseKind::Reply => {
            let id = reply_to.ok_or_else(|| EvalError::Config(format!("case {}: reply needs a thread", case.id)))?;
            generate_draft(db, id, instructions).await?
        }
        DraftCaseKind::New => {
            let subject = case
                .subject
                .as_deref()
                .ok_or_else(|| EvalError::Config(format!("case {}: new needs a subject", case.id)))?;
            generate_new_draft(db, EVAL_ACCOUNT_ID, &case.to, subject, instructions).await?
        }
    };
    Ok(result.body)
}

async fn run_case(db: &Arc<Database>, case: &DraftCase, repeat: usize) -> EvalResult<ItemResult> {
    let reply_to = seed_case(db, case)?;
    let name = first_name(&case.account.name);
    let mut checks = Vec::new();
    let mut outputs = Vec::new();
    let mut signed_count = 0usize;
    for i in 0..repeat.max(1) {
        let body = draft_once(db, case, reply_to.as_deref()).await?;
        let ending = draft_ending(&body, name);
        if ending.signed() {
            signed_count += 1;
        }
        let passed = case.expect_signed.is_none_or(|want| want == ending.signed());
        checks.push(EvidenceCheck {
            name: format!("run {} signed", i + 1),
            expected: case
                .expect_signed
                .map_or_else(|| "(report only)".to_string(), |b| b.to_string()),
            actual: format!(
                "{} (name_in_tail={}, placeholder={})",
                ending.signed(),
                ending.name_in_tail,
                ending.placeholder_in_tail
            ),
            passed,
        });
        outputs.push(format!("── run {} ──\n{}", i + 1, body));
    }
    let runs = checks.len();
    let passing = checks.iter().filter(|c| c.passed).count();
    Ok(ItemResult {
        id: case.id.clone(),
        passed: passing == runs,
        score: Some(passing as f64 / runs as f64),
        detail: format!(
            "signed {signed_count}/{runs} (expect_signed={})",
            case.expect_signed
                .map_or_else(|| "report-only".to_string(), |b| b.to_string())
        ),
        evidence: Some(ItemEvidence {
            input: format!(
                "kind={:?} signature={} instructions={:?}",
                case.kind,
                case.signature.is_some(),
                case.instructions
            ),
            output: outputs.join("\n\n"),
            checks,
        }),
    })
}

fn app_data_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("EMAILOPS_DATA_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library").join("Application Support").join("com.emailops.app"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_dir().map(|d| d.join("com.emailops.app"))
    }
}

/// Run the synthetic draft eval. Returns the path of the JSON report.
pub async fn run(cfg: DraftCaseEvalConfig) -> EvalResult<PathBuf> {
    let cases = load_cases(&cfg.cases_path)?;
    let cases: Vec<_> = match &cfg.case_filter {
        Some(filter) => cases.into_iter().filter(|c| c.id.contains(filter.as_str())).collect(),
        None => cases,
    };
    if cases.is_empty() {
        return Err(EvalError::Config(format!(
            "no cases matched (filter: {:?}) in {}",
            cfg.case_filter,
            cfg.cases_path.display()
        )));
    }

    // In-memory DB: nothing of the user's mailbox is read or written. The
    // llamacpp backend finds GGUFs through `app_data_dir` (read-only).
    let db = Arc::new(Database::new_for_testing()?);
    if let Some(dir) = app_data_dir() {
        db.set_preference("app_data_dir", &dir.to_string_lossy())?;
    }
    db.set_preference("ai_provider", &cfg.provider_name)?;
    db.set_preference("ai_model", &cfg.model)?;
    super::shared::preflight_models(&db, [cfg.model.as_str()])?;

    let mut report = JsonRunReport::new("draft_case_eval", cfg.model.clone());
    for case in &cases {
        let item = match run_case(&db, case, cfg.repeat).await {
            Ok(item) => item,
            Err(e) => ItemResult {
                id: case.id.clone(),
                passed: false,
                score: Some(0.0),
                detail: format!("ERROR: {e}"),
                evidence: None,
            },
        };
        eprintln!(
            "[draft-case-eval] {} {} — {}",
            if item.passed { "PASS" } else { "FAIL" },
            item.id,
            item.detail
        );
        report.push(item);
    }
    eprintln!(
        "[draft-case-eval] {}/{} passed ({:.0}%)",
        report.succeeded,
        report.total,
        report.pass_rate() * 100.0
    );
    report.write(&cfg.out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ending_with_the_name_is_signed() {
        let e = draft_ending("Hi Ana,\n\nThursday works.\n\nBest regards,\nUlises", "Ulises");
        assert!(e.name_in_tail && e.signed());
    }

    #[test]
    fn ending_with_a_closing_only_is_not_signed() {
        let e = draft_ending("Hi Ana,\n\nThursday works.\n\nBest regards,", "Ulises");
        assert!(!e.signed());
    }

    #[test]
    fn the_name_early_in_the_body_does_not_count() {
        let body = "Hi Ana,\n\nUlises here, about Thursday.\n\nIt works for me.\nSee you then.\nThanks,";
        assert!(!draft_ending(body, "Ulises").signed());
    }

    #[test]
    fn a_name_placeholder_counts_as_signed() {
        let e = draft_ending("Thursday works.\n\nSaludos,\n[Tu nombre]", "Ulises");
        assert!(e.placeholder_in_tail && e.signed());
    }

    #[test]
    fn a_lone_placeholder_line_after_the_closing_counts_as_signed() {
        assert!(draft_ending("See you then,\n\n[placeholder]", "Ulises").signed());
        assert!(!draft_ending("Price: [placeholder] per hour.\nBest regards,", "Ulises").signed());
    }

    #[test]
    fn the_name_must_be_a_whole_word() {
        assert!(!draft_ending("Thanks,\nAnalytics team", "Ana").signed());
        assert!(draft_ending("Thanks,\n— ana", "Ana").signed());
    }

    #[test]
    fn ships_with_valid_cases_file() {
        let cases = load_cases(Path::new("evals/drafts/cases.yaml")).expect("cases file must parse");
        assert!(cases.len() >= 4, "expected a meaningful case set, got {}", cases.len());
        for c in &cases {
            match c.kind {
                DraftCaseKind::Reply => assert!(!c.thread.is_empty(), "{} needs a thread", c.id),
                DraftCaseKind::New => assert!(c.subject.is_some() && !c.to.is_empty(), "{} needs to+subject", c.id),
            }
        }
    }
}
