// Shared helpers across chat_eval and chat_shortcut_eval.

use tauri::{AppHandle, Manager};

use crate::db::Database;
use crate::evals::{EvalError, EvalResult};

/// Build a Tauri `App` with Wry runtime using the test `mock_context`, so we
/// have a valid `AppHandle<Wry>` for event emission even without a real window.
/// Events emitted during the eval go to no listeners and are dropped — that is
/// the intended behavior; we read results back from the DB.
pub fn build_mock_app() -> EvalResult<AppHandle> {
    let context = tauri::test::mock_context::<tauri::Wry, _>(tauri::test::noop_assets());
    let app = tauri::Builder::default()
        .build(context)
        .map_err(|e| EvalError::Config(format!("failed to build mock Tauri app: {}", e)))?;
    Ok(app.app_handle().clone())
}

/// Env var names recognised by `apply_eval_model_override_from_env`.
pub const EVAL_MODEL_ENV: &str = "EMAILOPS_EVAL_MODEL";
pub const EVAL_PROVIDER_ENV: &str = "EMAILOPS_EVAL_PROVIDER";

/// Default model evals run against when neither an env override nor a per-case
/// `model:` is set. Matches the app's catalog default (local llama.cpp GGUF).
pub const DEFAULT_EVAL_MODEL: &str = "qwen3.5-4b-q4_k_m";

/// Apply a model+provider override to the (already copied) eval DB so every
/// downstream call to `AiService::load_provider(&db)` picks up the requested
/// model. Reads `EMAILOPS_EVAL_MODEL` / `EMAILOPS_EVAL_PROVIDER` from the
/// environment; returns `Ok(None)` (and leaves prefs untouched) when no
/// override is set.
///
/// Defaults `EMAILOPS_EVAL_PROVIDER` to `llamacpp` when `EMAILOPS_EVAL_MODEL`
/// is set without a matching provider — the catalog models used for
/// 4B-vs-9B comparisons are llama.cpp GGUFs.
///
/// This is the single point of truth for the `make eval-all MODEL=…` target.
/// Eval runners call it immediately after `prepare_eval_db` so the override
/// affects only the isolated temp copy, never the user's prod DB.
pub fn apply_eval_model_override_from_env(db: &Database) -> EvalResult<Option<(String, String)>> {
    let Some(model) = read_non_empty_env(EVAL_MODEL_ENV) else {
        return Ok(None);
    };
    let provider = read_non_empty_env(EVAL_PROVIDER_ENV).unwrap_or_else(|| "llamacpp".to_string());
    db.set_preference("ai_provider", &provider)?;
    db.set_preference("ai_model", &model)?;
    eprintln!(
        "[eval] model override applied via env: provider={} model={}",
        provider, model
    );
    Ok(Some((provider, model)))
}

/// Pin the (already copied) eval DB to a provider+model for the duration of an
/// eval run. Evals must default to the app's *default* provider — local
/// llama.cpp — rather than inheriting whatever the copied prod DB happened to
/// have configured (often Ollama from day-to-day use).
///
/// An explicit `EMAILOPS_EVAL_MODEL` env override still wins: when it is set,
/// `apply_eval_model_override_from_env` has already written the desired
/// provider+model, so this leaves the prefs untouched.
///
/// `case_model` is the model the case/suite requested (YAML `model:` or
/// `--model`); it becomes `ai_model` whenever no env override is active.
pub fn pin_eval_provider(db: &Database, case_model: &str) -> EvalResult<()> {
    if read_non_empty_env(EVAL_MODEL_ENV).is_some() {
        // Env override already pinned provider+model; respect it.
        return Ok(());
    }
    db.set_preference("ai_provider", "llamacpp")?;
    db.set_preference("ai_model", case_model)?;
    Ok(())
}

/// What a preflight concluded about the model a run is about to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPreflight {
    /// The local GGUF is on disk (or the provider needs no local file).
    Ready,
    /// A served provider (Ollama, OpenRouter) — nothing local to check.
    NotLocal,
    /// Local provider, but the file is missing.
    Missing { path: std::path::PathBuf },
    /// Local provider, but no `app_data_dir` preference to resolve a path from.
    Unresolvable,
}

/// Pure: decide whether a run can proceed, given the provider and whether the
/// model file is on disk. `exists` is injected so this is testable without
/// touching a filesystem.
pub fn plan_model_preflight(provider: &str, model_path: Option<&std::path::Path>, exists: bool) -> ModelPreflight {
    if provider != "llamacpp" {
        return ModelPreflight::NotLocal;
    }
    match model_path {
        None => ModelPreflight::Unresolvable,
        Some(_) if exists => ModelPreflight::Ready,
        Some(path) => ModelPreflight::Missing {
            path: path.to_path_buf(),
        },
    }
}

/// Fail before the first case when the run's model cannot possibly answer.
///
/// Without this, a missing GGUF is discovered once per case, at turn time: a
/// 46-case suite spent ~90 minutes reporting the same "model file not found"
/// 46 times, and the report attributed each one to the feature the case
/// belonged to, as if the cases had failed on their merits.
///
/// `models` is every distinct model the run will use — the suite default plus
/// any per-case `model:` pins — so a run cannot get halfway in and then die on
/// a model only one case asked for.
pub fn preflight_models<'a>(db: &Database, models: impl IntoIterator<Item = &'a str>) -> EvalResult<()> {
    use crate::ai::{model_catalog::ModelKind, model_manager};

    let provider = db
        .get_preference("ai_provider")?
        .unwrap_or_else(|| "llamacpp".to_string());
    let app_data_dir = db.get_preference("app_data_dir")?.map(std::path::PathBuf::from);

    let mut missing: Vec<String> = Vec::new();
    for model in models {
        let path = app_data_dir
            .as_ref()
            .map(|dir| model_manager::model_path(dir, ModelKind::Chat, model));
        match plan_model_preflight(&provider, path.as_deref(), path.as_deref().is_some_and(|p| p.exists())) {
            ModelPreflight::Ready | ModelPreflight::NotLocal => {}
            ModelPreflight::Missing { path } => missing.push(format!("{model} → {}", path.display())),
            ModelPreflight::Unresolvable => {
                return Err(EvalError::Config(format!(
                    "provider is '{provider}' but the eval DB has no `app_data_dir` preference, \
                     so '{model}' cannot be located"
                )))
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }
    Err(EvalError::Config(format!(
        "model file(s) not found — download them, or pick an installed model with \
         {EVAL_MODEL_ENV}=<id> / --model <id>:\n  {}",
        missing.join("\n  ")
    )))
}

fn read_non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `std::env::set_var` mutates process-global state. Wrap eval-model-override
    /// tests in a mutex so they don't stomp on each other when `cargo test`
    /// runs them concurrently.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn fresh_db() -> Database {
        Database::new_for_testing().expect("test db")
    }

    fn clear_eval_env() {
        std::env::remove_var(EVAL_MODEL_ENV);
        std::env::remove_var(EVAL_PROVIDER_ENV);
    }

    #[test]
    fn returns_none_when_env_unset() {
        let _g = env_lock();
        clear_eval_env();
        let db = fresh_db();
        let result = apply_eval_model_override_from_env(&db).expect("ok");
        assert!(result.is_none());
        assert!(db.get_preference("ai_model").expect("read").is_none());
        assert!(db.get_preference("ai_provider").expect("read").is_none());
    }

    #[test]
    fn defaults_provider_to_llamacpp_when_only_model_set() {
        let _g = env_lock();
        clear_eval_env();
        std::env::set_var(EVAL_MODEL_ENV, "qwen3.5-9b-q4_k_m");
        let db = fresh_db();
        let (provider, model) = apply_eval_model_override_from_env(&db)
            .expect("ok")
            .expect("override applied");
        assert_eq!(provider, "llamacpp");
        assert_eq!(model, "qwen3.5-9b-q4_k_m");
        assert_eq!(
            db.get_preference("ai_model").expect("read"),
            Some("qwen3.5-9b-q4_k_m".to_string())
        );
        assert_eq!(
            db.get_preference("ai_provider").expect("read"),
            Some("llamacpp".to_string())
        );
        clear_eval_env();
    }

    #[test]
    fn honours_explicit_provider_env() {
        let _g = env_lock();
        clear_eval_env();
        std::env::set_var(EVAL_MODEL_ENV, "llama3.1:8b");
        std::env::set_var(EVAL_PROVIDER_ENV, "ollama");
        let db = fresh_db();
        let (provider, model) = apply_eval_model_override_from_env(&db)
            .expect("ok")
            .expect("override applied");
        assert_eq!(provider, "ollama");
        assert_eq!(model, "llama3.1:8b");
        clear_eval_env();
    }

    #[test]
    fn pin_eval_provider_forces_llamacpp_when_env_unset() {
        let _g = env_lock();
        clear_eval_env();
        let db = fresh_db();
        pin_eval_provider(&db, "qwen3.5-4b-q4_k_m").expect("pin ok");
        assert_eq!(
            db.get_preference("ai_provider").expect("read"),
            Some("llamacpp".to_string())
        );
        assert_eq!(
            db.get_preference("ai_model").expect("read"),
            Some("qwen3.5-4b-q4_k_m".to_string())
        );
    }

    #[test]
    fn pin_eval_provider_respects_env_override() {
        let _g = env_lock();
        clear_eval_env();
        let db = fresh_db();
        // Simulate apply_eval_model_override_from_env having run for an Ollama override.
        std::env::set_var(EVAL_MODEL_ENV, "llama3.1:8b");
        db.set_preference("ai_provider", "ollama").expect("set");
        db.set_preference("ai_model", "llama3.1:8b").expect("set");

        // pin must not clobber the deliberate env-driven override.
        pin_eval_provider(&db, "qwen3.5-4b-q4_k_m").expect("pin ok");
        assert_eq!(
            db.get_preference("ai_provider").expect("read"),
            Some("ollama".to_string())
        );
        assert_eq!(
            db.get_preference("ai_model").expect("read"),
            Some("llama3.1:8b".to_string())
        );
        clear_eval_env();
    }

    // ── plan_model_preflight ──────────────────────────────────────────────────

    #[test]
    fn a_present_local_model_is_ready() {
        assert_eq!(
            plan_model_preflight("llamacpp", Some(std::path::Path::new("/m/x.gguf")), true),
            ModelPreflight::Ready
        );
    }

    #[test]
    fn a_missing_local_model_is_reported_before_anything_runs() {
        // The whole point: 46 cases each failed at turn time with the same
        // "model file not found", 90 minutes in. One check up front says it once.
        let path = std::path::PathBuf::from("/m/x.gguf");
        assert_eq!(
            plan_model_preflight("llamacpp", Some(&path), false),
            ModelPreflight::Missing { path }
        );
    }

    #[test]
    fn a_local_model_with_no_resolvable_path_is_reported_too() {
        // No `app_data_dir` preference: the provider would fail with "no chat
        // model configured", which is just as fatal and just as worth saying early.
        assert_eq!(
            plan_model_preflight("llamacpp", None, false),
            ModelPreflight::Unresolvable
        );
    }

    #[test]
    fn a_served_provider_has_no_local_file_to_check() {
        // Ollama and OpenRouter answer over HTTP; a missing local GGUF says
        // nothing about them, so the preflight must not block the run.
        for provider in ["ollama", "openrouter"] {
            assert_eq!(
                plan_model_preflight(provider, None, false),
                ModelPreflight::NotLocal,
                "{provider} must not be preflighted for a local file"
            );
        }
    }

    #[test]
    fn ignores_blank_env_values() {
        let _g = env_lock();
        clear_eval_env();
        std::env::set_var(EVAL_MODEL_ENV, "   ");
        let db = fresh_db();
        let result = apply_eval_model_override_from_env(&db).expect("ok");
        assert!(result.is_none(), "blank EMAILOPS_EVAL_MODEL should be treated as unset");
        clear_eval_env();
    }
}
