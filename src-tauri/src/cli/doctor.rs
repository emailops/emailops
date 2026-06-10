//! `emailops-cli doctor` — a fast, read-only environment readiness check.
//!
//! It answers "is this CLI pointed at a usable EmailOps install?" without
//! loading any AI model or touching the network: it reports the data dir, the
//! DB email/account counts, and the configured AI provider/model from
//! preferences. An agent runs `doctor --json` before driving real commands to
//! confirm the environment is wired up.

use std::path::Path;

use serde::Serialize;

use crate::db::Database;
use crate::models::error::Result;

use super::output;
use super::OutputMode;

/// Structured readiness report. `ok` is true when the install is usable for the
/// common commands: a reachable DB and at least one enabled account.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub ok: bool,
    pub data_dir: String,
    pub email_count: i64,
    pub accounts_total: usize,
    pub accounts_enabled: usize,
    pub ai_enabled: bool,
    pub provider: String,
    pub model: String,
    pub embedding_model: String,
    /// Non-fatal issues an agent or user may want to fix (e.g. no enabled
    /// account, AI disabled).
    pub warnings: Vec<String>,
}

/// Build the report from the DB. Pure aside from read-only DB queries — no model
/// load, no network — so it stays fast and safe to run against a live install.
pub fn build_report(db: &Database, data_dir: &Path, model: &str) -> Result<DoctorReport> {
    let email_count: i64 = db.reader().query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0))?;

    let accounts = db.list_accounts()?;
    let accounts_total = accounts.len();
    let accounts_enabled = accounts.iter().filter(|a| a.enabled).count();

    let ai_enabled = db.is_ai_enabled()?;
    let provider = db
        .get_preference("ai_provider")?
        .unwrap_or_else(|| "llamacpp".to_string());
    let embedding_model = db
        .get_preference("ai_embedding_model")?
        .unwrap_or_else(|| "nomic-embed-text-v1.5-q4_k_m".to_string());

    let mut warnings = Vec::new();
    if accounts_enabled == 0 {
        warnings.push("no enabled accounts — add or enable an account in the desktop app".to_string());
    }
    if !ai_enabled {
        warnings.push("AI is disabled in Settings — chat/classify/embed will fail".to_string());
    }

    Ok(DoctorReport {
        ok: accounts_enabled >= 1,
        data_dir: data_dir.display().to_string(),
        email_count,
        accounts_total,
        accounts_enabled,
        ai_enabled,
        provider,
        model: model.to_string(),
        embedding_model,
        warnings,
    })
}

/// Render the report: a success envelope in JSON mode, an aligned section in
/// pretty mode. Always returns `Ok(())` — the report's `ok` field (and the
/// warnings list) carries the verdict, so a "not ready" install does not also
/// surface as a process error.
pub fn render(report: &DoctorReport, mode: OutputMode) -> Result<()> {
    if mode == OutputMode::Json {
        return output::emit_ok(report);
    }
    println!("EmailOps doctor — {}", if report.ok { "ready" } else { "NOT ready" });
    println!("  data dir:        {}", report.data_dir);
    println!("  emails:          {}", report.email_count);
    println!(
        "  accounts:        {} ({} enabled)",
        report.accounts_total, report.accounts_enabled
    );
    println!("  ai enabled:      {}", report.ai_enabled);
    println!("  provider:        {}", report.provider);
    println!("  model:           {}", report.model);
    println!("  embedding model: {}", report.embedding_model);
    if !report.warnings.is_empty() {
        println!("  warnings:");
        for w in &report.warnings {
            println!("    - {w}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn seed_account(db: &Arc<Database>, id: &str, email: &str, enabled: bool) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, 'gmail', ?2, ?2, 0, 0, ?3)",
                rusqlite::params![id, email, enabled as i32],
            )
            .expect("seed account");
    }

    #[test]
    fn report_is_ok_with_an_enabled_account() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        let report = build_report(&db, Path::new("/tmp/x"), "m").expect("report");
        assert!(report.ok);
        assert_eq!(report.accounts_enabled, 1);
        assert!(report.warnings.iter().all(|w| !w.contains("no enabled accounts")));
    }

    #[test]
    fn report_warns_and_not_ok_without_enabled_accounts() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "off@example.com", false);
        let report = build_report(&db, Path::new("/tmp/x"), "m").expect("report");
        assert!(!report.ok);
        assert_eq!(report.accounts_total, 1);
        assert_eq!(report.accounts_enabled, 0);
        assert!(report.warnings.iter().any(|w| w.contains("no enabled accounts")));
    }

    #[test]
    fn report_warns_when_ai_disabled() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        db.set_preference("ai_enabled", "false").expect("set pref");
        let report = build_report(&db, Path::new("/tmp/x"), "m").expect("report");
        assert!(!report.ai_enabled);
        assert!(report.warnings.iter().any(|w| w.contains("AI is disabled")));
        // an enabled account still makes the install "ok" overall
        assert!(report.ok);
    }

    #[test]
    fn report_reflects_provider_and_model_preferences() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        db.set_preference("ai_provider", "openrouter").expect("set pref");
        let report = build_report(&db, Path::new("/tmp/x"), "claude-3").expect("report");
        assert_eq!(report.provider, "openrouter");
        assert_eq!(report.model, "claude-3");
    }
}
