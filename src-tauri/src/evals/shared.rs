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
