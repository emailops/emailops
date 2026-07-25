//! Eval harness for AI email translation (language detection + translation).
//!
//! Fully synthetic and heuristic: cases live in `src-tauri/evals/translation/`
//! (no personal-mailbox content), scoring is deterministic (exact ISO-code
//! match for detection; keyword presence/absence for translation) — no LLM
//! judge. Runs against an in-memory DB seeded with the synthetic case text, so
//! the real `services::translation` executors are exercised end-to-end minus
//! only the Tauri command shell.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::db::Database;
use crate::services::ai::AiService;
use crate::services::translation;

use super::json_report::{ItemResult, JsonRunReport};
use super::{EvalError, EvalResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseKind {
    Detect,
    Translate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranslationCase {
    pub id: String,
    pub kind: CaseKind,
    /// Synthetic email/draft text (may include simple HTML for detect cases).
    pub text: String,
    /// Detect: expected ISO 639-1 code.
    #[serde(default)]
    pub expected_language: Option<String>,
    /// Translate: target language (ISO code or English name).
    #[serde(default)]
    pub target: Option<String>,
    /// Translate: strings that MUST appear in the output (names, URLs, numbers).
    #[serde(default)]
    pub expect_contains: Vec<String>,
    /// Translate: strings that must NOT appear (source-language phrases).
    #[serde(default)]
    pub expect_not_contains: Vec<String>,
}

pub struct TranslationEvalConfig {
    pub model: String,
    pub provider_name: String,
    pub cases_path: PathBuf,
    pub out_dir: PathBuf,
    /// Run only the case with this id.
    pub case_filter: Option<String>,
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

pub fn load_cases(path: &Path) -> EvalResult<Vec<TranslationCase>> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&raw)?)
}

/// Seed a synthetic email into the (in-memory) eval DB so the real
/// email-scoped executor path runs unchanged.
fn seed_email(db: &Database, id: &str, body: &str) -> EvalResult<()> {
    db.connection().execute(
        "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
         VALUES ('eval-acct', 'gmail', 'eval@translation.test', 'Eval', 0)",
        [],
    )?;
    let email = crate::models::Email {
        id: id.to_string(),
        account_id: "eval-acct".to_string(),
        thread_id: format!("thread-{id}"),
        message_id: None,
        subject: "Translation eval".to_string(),
        sender: "Eval Sender".to_string(),
        sender_email: "sender@translation.test".to_string(),
        recipients: vec!["eval@translation.test".to_string()],
        cc: vec![],
        body: body.to_string(),
        snippet: String::new(),
        timestamp: 1_700_000_000,
        is_read: true,
        triage_status: None,
        category: "primary".to_string(),
        mailbox: "inbox".to_string(),
    };
    db.insert_email(&email)?;
    Ok(())
}

async fn run_case(ai: &AiService, db: &Arc<Database>, case: &TranslationCase) -> EvalResult<ItemResult> {
    match case.kind {
        CaseKind::Detect => {
            let expected = case
                .expected_language
                .as_deref()
                .ok_or_else(|| EvalError::Config(format!("case {}: detect needs expected_language", case.id)))?;
            let email_id = format!("eval-{}", case.id);
            seed_email(db, &email_id, &case.text)?;
            let result = translation::detect_email_language_with(ai, db, &email_id).await?;
            let passed = result.language == expected;
            Ok(ItemResult {
                id: case.id.clone(),
                passed,
                score: Some(if passed { 1.0 } else { 0.0 }),
                detail: format!("detected={} expected={}", result.language, expected),
            })
        }
        CaseKind::Translate => {
            let target = case
                .target
                .as_deref()
                .ok_or_else(|| EvalError::Config(format!("case {}: translate needs target", case.id)))?;
            let result = translation::translate_text_with(ai, db, &case.text, target).await?;
            let mut failures: Vec<String> = Vec::new();
            for needle in &case.expect_contains {
                if !result.text.contains(needle.as_str()) {
                    failures.push(format!("missing {needle:?}"));
                }
            }
            let lower = result.text.to_lowercase();
            for needle in &case.expect_not_contains {
                if lower.contains(&needle.to_lowercase()) {
                    failures.push(format!("still contains {needle:?}"));
                }
            }
            let checks = case.expect_contains.len() + case.expect_not_contains.len();
            let passed = failures.is_empty();
            Ok(ItemResult {
                id: case.id.clone(),
                passed,
                score: Some(if checks == 0 {
                    1.0
                } else {
                    (checks - failures.len()) as f64 / checks as f64
                }),
                detail: if passed {
                    format!(
                        "ok ({} chars, target={})",
                        result.text.chars().count(),
                        result.target_language
                    )
                } else {
                    failures.join("; ")
                },
            })
        }
    }
}

/// Run the translation eval. Returns the path of the JSON report.
pub async fn run(cfg: TranslationEvalConfig) -> EvalResult<PathBuf> {
    let cases = load_cases(&cfg.cases_path)?;
    let cases: Vec<_> = match &cfg.case_filter {
        Some(id) => cases.into_iter().filter(|c| &c.id == id).collect(),
        None => cases,
    };
    if cases.is_empty() {
        return Err(EvalError::Config(format!(
            "no cases matched (filter: {:?}) in {}",
            cfg.case_filter,
            cfg.cases_path.display()
        )));
    }

    let db = Arc::new(Database::new_for_testing()?);
    // The llamacpp backend resolves GGUF paths via the `app_data_dir` pref,
    // which a fresh in-memory DB doesn't have — point it at the real app data
    // dir (or `EMAILOPS_DATA_DIR`) so the bundled models are found.
    if let Some(dir) = app_data_dir() {
        db.set_preference("app_data_dir", &dir.to_string_lossy())?;
    }
    let provider = AiService::build_provider(&db, &cfg.provider_name, &cfg.model)?;
    let ai = AiService::with_provider(db.clone(), provider);

    let mut report = JsonRunReport::new("translation_eval", cfg.model.clone());
    for case in &cases {
        let item = match run_case(&ai, &db, case).await {
            Ok(item) => item,
            Err(e) => ItemResult {
                id: case.id.clone(),
                passed: false,
                score: Some(0.0),
                detail: format!("ERROR: {e}"),
            },
        };
        eprintln!(
            "[translation-eval] {} {} — {}",
            if item.passed { "PASS" } else { "FAIL" },
            item.id,
            item.detail
        );
        report.push(item);
    }

    eprintln!(
        "[translation-eval] {}/{} passed ({:.0}%)",
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
    fn ships_with_valid_cases_file() {
        let cases = load_cases(Path::new("evals/translation/cases.yaml")).expect("cases file must parse");
        assert!(cases.len() >= 8, "expected a meaningful case set, got {}", cases.len());
        for c in &cases {
            match c.kind {
                CaseKind::Detect => assert!(c.expected_language.is_some(), "{} needs expected_language", c.id),
                CaseKind::Translate => {
                    assert!(c.target.is_some(), "{} needs target", c.id);
                    assert!(
                        !c.expect_contains.is_empty() || !c.expect_not_contains.is_empty(),
                        "{} needs at least one assertion",
                        c.id
                    );
                }
            }
        }
    }
}
