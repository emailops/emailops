//! Tauri command surface for the Lenses feature.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::{AppHandle, Emitter, State};

use crate::models::error::{AppError, Result};
use crate::models::lens::{
    ColumnFilter, ColumnValueCount, CreateLensInput, Lens, LensRowsPage, LensRunHandle, LensRunHistoryEntry,
    LensRunKind, LensSchema, LensScope, LensStatus, LensSummary, PreviewRow, SortSpec, UpdateLensInput,
};
use crate::models::AppLogEvent;
use crate::services::ai::AiService;
use crate::services::lenses::{extractor, runner, scope as scope_eval, templates};
use crate::AppState;

/// Per-Lens cancel flags. A Lens with an entry here has an in-flight run that
/// will poll the flag and exit early when it flips to `true`. Cleaned up when
/// the run completes (success, failure, or cancel).
static CANCEL_FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    CANCEL_FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Claim a lens for a run, or `None` when one is already queued or running.
///
/// The cancel-flag map doubles as the registry of in-flight runs: an entry
/// exists from the moment `run_lens` submits until the runner clears it. A
/// second click while that entry is there must be a no-op — the queue does not
/// deduplicate (`services::task_queue::submit_with_priority`), so without this
/// each click piled another identical backfill onto the AI queue.
fn try_register_cancel_flag(lens_id: &str) -> Option<Arc<AtomicBool>> {
    // A poisoned mutex here means another thread panicked while holding the
    // lock — there is no meaningful recovery for the cancel-flags map.
    #[allow(clippy::expect_used)]
    let mut guard = cancel_flags().lock().expect("cancel-flags mutex poisoned");
    if guard.contains_key(lens_id) {
        return None;
    }
    let flag = Arc::new(AtomicBool::new(false));
    guard.insert(lens_id.to_string(), flag.clone());
    Some(flag)
}

fn clear_cancel_flag(lens_id: &str) {
    #[allow(clippy::expect_used)]
    let mut guard = cancel_flags().lock().expect("cancel-flags mutex poisoned");
    guard.remove(lens_id);
}

fn emit_log(_app: &AppHandle, level: &str, message: &str) {
    crate::services::logger::log(level, "lens", message);
}

// ── CRUD ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_lenses(state: State<'_, AppState>) -> Result<Vec<LensSummary>> {
    state.db.list_lenses()
}

#[tauri::command]
pub async fn get_lens(state: State<'_, AppState>, lens_id: String) -> Result<Lens> {
    state.db.get_lens(&lens_id)
}

#[tauri::command]
pub async fn create_lens(state: State<'_, AppState>, app: AppHandle, input: CreateLensInput) -> Result<Lens> {
    let lens = state.db.create_lens(&input)?;
    emit_log(&app, "info", &format!("Lens '{}' created", lens.name));
    Ok(lens)
}

#[tauri::command]
pub async fn update_lens(state: State<'_, AppState>, lens_id: String, input: UpdateLensInput) -> Result<Lens> {
    state.db.update_lens(&lens_id, &input)
}

#[tauri::command]
pub async fn delete_lens(state: State<'_, AppState>, lens_id: String) -> Result<()> {
    state.db.delete_lens(&lens_id)
}

#[tauri::command]
pub async fn duplicate_lens(state: State<'_, AppState>, lens_id: String, new_name: String) -> Result<Lens> {
    let existing = state.db.get_lens(&lens_id)?;
    let input = CreateLensInput {
        name: new_name,
        icon: existing.icon,
        // A duplicate is always a custom Lens — clear the template link.
        template_key: None,
        account_id: existing.account_id,
        scope: existing.scope,
        schema: existing.schema,
        prompt_text: existing.prompt_text,
        model_provider: existing.model_provider,
        model_name: existing.model_name,
    };
    state.db.create_lens(&input)
}

// ── Templates ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_lens_templates() -> Result<Vec<templates::LensTemplate>> {
    Ok(templates::manifest())
}

#[tauri::command]
pub async fn create_lens_from_template(
    state: State<'_, AppState>,
    app: AppHandle,
    template_key: String,
    name: Option<String>,
    account_id: Option<String>,
) -> Result<Lens> {
    let tpl = templates::get(&template_key).ok_or_else(|| AppError::NotFound(format!("template '{template_key}'")))?;
    let input = CreateLensInput {
        name: name.unwrap_or(tpl.name),
        icon: Some(tpl.icon),
        template_key: Some(tpl.key),
        account_id,
        scope: tpl.default_scope,
        schema: tpl.schema,
        prompt_text: tpl.prompt,
        model_provider: None,
        model_name: None,
    };
    let lens = state.db.create_lens(&input)?;
    emit_log(&app, "info", &format!("Lens '{}' created from template", lens.name));
    Ok(lens)
}

// ── Rows ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_lens_rows(
    state: State<'_, AppState>,
    lens_id: String,
    sort: Option<SortSpec>,
    filters: Option<Vec<ColumnFilter>>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<LensRowsPage> {
    state.db.get_lens_rows_filtered(
        &lens_id,
        sort.as_ref(),
        filters.as_deref().unwrap_or_default(),
        limit.unwrap_or(200),
        offset.unwrap_or(0),
    )
}

/// Distinct values of one column with their row counts, for the Excel-style
/// column filter.
#[tauri::command]
pub async fn get_lens_column_values(
    state: State<'_, AppState>,
    lens_id: String,
    key: String,
) -> Result<Vec<ColumnValueCount>> {
    state.db.get_lens_column_values(&lens_id, &key)
}

/// The rows the user has excluded, so the "include" command below has a screen
/// on which to name one. Without this listing, excluding was irreversible in
/// practice even though the undo existed.
#[tauri::command]
pub async fn get_excluded_lens_rows(
    state: State<'_, AppState>,
    lens_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<LensRowsPage> {
    state
        .db
        .get_excluded_lens_rows(&lens_id, limit.unwrap_or(200), offset.unwrap_or(0))
}

#[tauri::command]
pub async fn update_lens_row_override(
    state: State<'_, AppState>,
    lens_id: String,
    email_id: String,
    overrides: serde_json::Value,
) -> Result<()> {
    state.db.set_lens_row_override(&lens_id, &email_id, &overrides)
}

#[tauri::command]
pub async fn exclude_lens_row(state: State<'_, AppState>, lens_id: String, email_id: String) -> Result<()> {
    state.db.add_lens_exclusion(&lens_id, &email_id)
}

#[tauri::command]
pub async fn include_lens_row(state: State<'_, AppState>, lens_id: String, email_id: String) -> Result<()> {
    state.db.remove_lens_exclusion(&lens_id, &email_id)
}

// ── Runs ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_lens(
    state: State<'_, AppState>,
    app: AppHandle,
    lens_id: String,
    kind: Option<LensRunKind>,
) -> Result<LensRunHandle> {
    let kind = kind.unwrap_or(LensRunKind::Backfill);
    let db = state.db.clone();
    // Resolve the provider before submitting to the background queue so the
    // user sees an immediate, logged error if the AI provider is mis-configured
    // (e.g. OpenRouter selected but no API key stored). Without this log the
    // command returns Err but nothing appears in the output panel.
    let provider = AiService::load_provider(&db).map_err(|e| {
        emit_log(&app, "error", &format!("Lens run failed to start: {e}"));
        e
    })?;
    let lens_id_for_task = lens_id.clone();
    let lens_id_for_cleanup = lens_id.clone();
    let app_for_task = app.clone();
    let label = format!("lens:{kind}:{lens_id}", kind = kind.as_str());
    // A run for this lens may already be queued or running. Claim it, or tell
    // the user it is already on its way instead of queueing a duplicate.
    let Some(cancel) = try_register_cancel_flag(&lens_id) else {
        emit_log(
            &app,
            "info",
            &format!("Lens run already queued or running ({label}) — ignoring the repeated request"),
        );
        return Ok(LensRunHandle {
            run_id: String::new(),
            lens_id,
        });
    };
    let cancel_for_task = cancel.clone();

    // Submit to the background AI queue so the user-facing thread returns
    // immediately. Progress and completion are reported via `app-log`.
    // The user is watching a button for this, so it goes ahead of the
    // background classify/junk/embed backlog — on a concurrency-1 queue that
    // backlog is hours, and the run looked like it had done nothing at all.
    emit_log(&app, "info", &format!("Lens run queued ({label})"));
    state
        .ai_background
        .submit_priority(&label, async move {
            let result = match kind {
                LensRunKind::Backfill | LensRunKind::Incremental => {
                    runner::backfill_lens(
                        db.clone(),
                        provider,
                        lens_id_for_task,
                        Some(app_for_task.clone()),
                        Some(cancel_for_task),
                    )
                    .await
                }
                LensRunKind::Reextract => {
                    runner::reextract_lens(
                        db.clone(),
                        provider,
                        lens_id_for_task,
                        Some(app_for_task.clone()),
                        Some(cancel_for_task),
                    )
                    .await
                }
                LensRunKind::Single => Err(AppError::InvalidInput(
                    "Single re-extract requires an email id — use reextract_lens_row".into(),
                )),
            };
            clear_cancel_flag(&lens_id_for_cleanup);
            if let Err(e) = result {
                let _ = app_for_task.emit(
                    "app-log",
                    AppLogEvent {
                        level: "error".to_string(),
                        source: "lens".to_string(),
                        message: format!("Lens run failed: {e}"),
                    },
                );
            }
        })
        .await;

    // We don't have the run_id ahead of time (it's inserted inside the runner).
    // Return a placeholder so the UI can still subscribe to log events.
    Ok(LensRunHandle {
        run_id: String::new(),
        lens_id,
    })
}

#[tauri::command]
pub async fn get_lens_status(state: State<'_, AppState>, lens_id: String) -> Result<LensStatus> {
    state.db.get_lens_status(&lens_id)
}

#[tauri::command]
pub async fn list_lens_runs(
    state: State<'_, AppState>,
    lens_id: String,
    limit: Option<i64>,
) -> Result<Vec<LensRunHistoryEntry>> {
    state.db.list_lens_runs(&lens_id, limit.unwrap_or(20))
}

/// Request cancellation of an in-flight run. The runner polls the flag between
/// emails and exits with status `cancelled` after finishing the current one.
///
/// If no in-memory flag is registered but the DB still has a `running` row
/// (orphaned by a crash or process restart), we force-finish the DB row so the
/// UI can move on instead of being stuck on a no-op Cancel button.
///
/// Returns `false` only when there is nothing to cancel.
#[tauri::command]
pub async fn cancel_lens_run(state: State<'_, AppState>, lens_id: String) -> Result<bool> {
    let flag = {
        #[allow(clippy::expect_used)]
        let map = cancel_flags().lock().expect("cancel-flags mutex poisoned");
        map.get(&lens_id).cloned()
    };
    if let Some(flag) = flag {
        flag.store(true, Ordering::Relaxed);
        return Ok(true);
    }
    // No live worker — clean up any DB row stuck in `running`.
    let recovered = state.db.force_cancel_running_lens_runs(&lens_id)?;
    Ok(recovered > 0)
}

#[tauri::command]
pub async fn reextract_lens_row(
    state: State<'_, AppState>,
    app: AppHandle,
    lens_id: String,
    email_id: String,
) -> Result<()> {
    let db = state.db.clone();
    let provider = AiService::load_provider(&db).map_err(|e| {
        emit_log(&app, "error", &format!("Row re-extract failed to start: {e}"));
        e
    })?;
    let label = format!("lens:single:{lens_id}");
    let lens_id_owned = lens_id.clone();
    let email_id_owned = email_id.clone();

    // The user is watching a button for this, so it goes ahead of the
    // background classify/junk/embed backlog — on a concurrency-1 queue that
    // backlog is hours, and the run looked like it had done nothing at all.
    emit_log(&app, "info", &format!("Lens run queued ({label})"));
    state
        .ai_background
        .submit_priority(&label, async move {
            if let Err(e) = runner::reextract_row(db, provider, lens_id_owned, email_id_owned, Some(app.clone())).await
            {
                let _ = app.emit(
                    "app-log",
                    AppLogEvent {
                        level: "error".to_string(),
                        source: "lens".to_string(),
                        message: format!("Row re-extract failed: {e}"),
                    },
                );
            }
        })
        .await;
    Ok(())
}

// ── Dry-run ────────────────────────────────────────────────────────────────

/// Run a small extraction over up to `sample_size` randomly-picked
/// scope-matching emails. Routed through `ai_queue` (interactive) since the
/// user is waiting inside the Create-Lens flow.
#[tauri::command]
pub async fn preview_lens_extraction(
    state: State<'_, AppState>,
    scope: LensScope,
    schema: LensSchema,
    prompt: String,
    sample_size: Option<i32>,
) -> Result<Vec<PreviewRow>> {
    let n = sample_size.unwrap_or(3).clamp(1, 10) as i64;
    let candidates = scope_eval::evaluate_with_limit(&state.db, &scope, n * 5)?;
    let picks: Vec<String> = candidates.into_iter().take(n as usize).collect();
    if picks.is_empty() {
        return Ok(Vec::new());
    }

    let provider = AiService::load_provider(&state.db)?;

    // Build a transient Lens object — no DB write — so we can reuse `extract_email`.
    let lens = Lens {
        id: String::new(),
        name: "(preview)".into(),
        icon: None,
        template_key: None,
        account_id: None,
        scope,
        schema,
        prompt_text: prompt,
        prompt_version: 0,
        model_provider: None,
        model_name: None,
        is_enabled: true,
        sort_order: 0,
        created_at: 0,
        updated_at: 0,
    };

    let mut out = Vec::with_capacity(picks.len());
    for email_id in picks {
        let res = extractor::extract_email(&state.db, provider.clone(), &lens, &email_id, None).await?;
        let email = state.db.get_email_by_id(&email_id)?;
        out.push(PreviewRow {
            email_id: email_id.clone(),
            email_subject: email.as_ref().map(|e| e.subject.clone()).unwrap_or_default(),
            email_sender: email.as_ref().map(|e| e.sender.clone()).unwrap_or_default(),
            data: res.data,
            status: res.status.as_str().to_string(),
            error_message: res.error_message,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this pins: "Run backfill" left the button clickable, and every
    /// click pushed another identical backfill onto a concurrency-1 queue that
    /// already had ~50 tasks on it. The cancel-flag map is the registry of
    /// runs that are queued-or-running, so claiming it is what makes a second
    /// submission a no-op.
    #[test]
    fn a_lens_can_be_claimed_once_while_a_run_is_in_flight() {
        let lens = "test-lens-claim-once";
        clear_cancel_flag(lens);

        assert!(
            try_register_cancel_flag(lens).is_some(),
            "the first submission must claim the lens"
        );
        assert!(
            try_register_cancel_flag(lens).is_none(),
            "a second click must not queue another run for the same lens"
        );

        clear_cancel_flag(lens);
        assert!(
            try_register_cancel_flag(lens).is_some(),
            "once the run finishes the lens can be run again"
        );
        clear_cancel_flag(lens);
    }

    #[test]
    fn claiming_one_lens_does_not_block_another() {
        let a = "test-lens-independent-a";
        let b = "test-lens-independent-b";
        clear_cancel_flag(a);
        clear_cancel_flag(b);

        assert!(try_register_cancel_flag(a).is_some());
        assert!(
            try_register_cancel_flag(b).is_some(),
            "a run on one lens must not stop a different lens from starting"
        );

        clear_cancel_flag(a);
        clear_cancel_flag(b);
    }
}
