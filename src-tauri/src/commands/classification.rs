use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::ClassificationRule;
use crate::services::classification::{self, ClassificationConfig};
use crate::AppState;

fn emit_log(_app: &AppHandle, level: &str, message: &str) {
    crate::services::logger::log(level, "classification", message);
}

#[tauri::command]
pub async fn get_classification_config(state: State<'_, AppState>) -> Result<ClassificationConfig, AppError> {
    classification::get_config(&state.db)
}

#[tauri::command]
pub async fn set_classification_config(
    state: State<'_, AppState>,
    config: ClassificationConfig,
) -> Result<(), AppError> {
    classification::save_config(&state.db, &config)
}

#[tauri::command]
pub async fn classify_previous_emails(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let db = state.db.clone();
    emit_log(&app, "info", "Starting classification of previous emails...");

    let task_label = format!("classify:account:{}", account_id);
    state
        .ai_background
        .submit_named(&task_label, async move {
            match classification::classify_all_emails(&db, &account_id).await {
                Ok(count) => {
                    emit_log(&app, "success", &format!("Classified {} emails", count));
                }
                Err(e) => {
                    emit_log(&app, "error", &format!("Classification failed: {}", e));
                }
            }
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn get_email_tags(
    state: State<'_, AppState>,
    email_id: String,
) -> Result<Vec<crate::models::EmailTag>, AppError> {
    classification::get_email_tags(&state.db, &email_id)
}

#[tauri::command]
pub async fn get_email_tags_batch(
    state: State<'_, AppState>,
    email_ids: Vec<String>,
) -> Result<Vec<crate::models::EmailTag>, AppError> {
    classification::get_email_tags_batch(&state.db, &email_ids)
}

#[tauri::command]
pub async fn count_unclassified_emails(state: State<'_, AppState>, account_id: String) -> Result<i32, AppError> {
    classification::count_unclassified(&state.db, &account_id)
}

/// `account_id: None` aggregates priorities across every enabled account
/// (unified "All accounts" view).
#[tauri::command]
pub async fn get_tag_priorities(
    state: State<'_, AppState>,
    account_id: Option<String>,
    tag_type: String,
    limit: Option<i32>,
) -> Result<Vec<crate::models::TagPriority>, AppError> {
    crate::services::tag_priority::get_priorities(&state.db, account_id.as_deref(), &tag_type, limit.unwrap_or(50))
}

#[tauri::command]
pub async fn reclassify_all_emails(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let db = state.db.clone();
    emit_log(&app, "info", "Starting reclassification of all emails...");

    let task_label = format!("reclassify:account:{}", account_id);
    state
        .ai_background
        .submit_named(&task_label, async move {
            match classification::reclassify_all_emails(&db, &account_id).await {
                Ok(count) => {
                    emit_log(&app, "success", &format!("Reclassified {} emails", count));
                }
                Err(e) => {
                    emit_log(&app, "error", &format!("Reclassification failed: {}", e));
                }
            }
        })
        .await;

    Ok(())
}

// -- Classification rules CRUD --

#[tauri::command]
pub async fn list_classification_rules(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<ClassificationRule>, AppError> {
    classification::list_rules(&state.db, &account_id)
}

#[tauri::command]
pub async fn create_classification_rule(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    name: String,
    sender_pattern: Option<String>,
    subject_pattern: Option<String>,
    priority: String,
    intent: String,
    topic: String,
) -> Result<ClassificationRule, AppError> {
    let rule = classification::create_rule(
        &state.db,
        &account_id,
        &name,
        sender_pattern.as_deref(),
        subject_pattern.as_deref(),
        &priority,
        &intent,
        &topic,
    )?;

    // Reclassify affected emails in background via task queue
    let db = state.db.clone();
    let rule_clone = rule.clone();
    let task_label = format!("reclassify:rule_create:{}", rule_clone.id);
    state
        .ai_background
        .submit_named(&task_label, async move {
            if let Err(e) = classification::reclassify_affected_emails(&db, &rule_clone).await {
                emit_log(&app, "error", &format!("Reclassify after create failed: {}", e));
            }
        })
        .await;

    Ok(rule)
}

#[tauri::command]
pub async fn update_classification_rule(
    app: AppHandle,
    state: State<'_, AppState>,
    rule: ClassificationRule,
) -> Result<(), AppError> {
    // Get the old rule to also reclassify emails that matched the old pattern
    let old_rule = state
        .db
        .get_classification_rules(&rule.account_id)?
        .into_iter()
        .find(|r| r.id == rule.id);

    classification::update_rule(&state.db, &rule)?;

    // Reclassify emails matching both old and new patterns via task queue
    let db = state.db.clone();
    let new_rule = rule.clone();
    let task_label = format!("reclassify:rule_update:{}", new_rule.id);
    state
        .ai_background
        .submit_named(&task_label, async move {
            if let Err(e) = classification::reclassify_affected_emails(&db, &new_rule).await {
                emit_log(&app, "error", &format!("Reclassify after update failed: {}", e));
            }
            if let Some(old) = old_rule {
                if old.sender_pattern != new_rule.sender_pattern || old.subject_pattern != new_rule.subject_pattern {
                    if let Err(e) = classification::reclassify_affected_emails(&db, &old).await {
                        emit_log(&app, "error", &format!("Reclassify old pattern failed: {}", e));
                    }
                }
            }
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn delete_classification_rule(
    app: AppHandle,
    state: State<'_, AppState>,
    rule_id: String,
    account_id: String,
) -> Result<(), AppError> {
    // Get the rule before deleting to know which emails to reclassify
    let rule = state
        .db
        .get_classification_rules(&account_id)?
        .into_iter()
        .find(|r| r.id == rule_id);

    classification::delete_rule(&state.db, &rule_id, &account_id)?;

    // Reclassify emails that were matched by the deleted rule via task queue
    if let Some(deleted_rule) = rule {
        let db = state.db.clone();
        let task_label = format!("reclassify:rule_delete:{}", deleted_rule.id);
        state
            .ai_background
            .submit_named(&task_label, async move {
                if let Err(e) = classification::reclassify_affected_emails(&db, &deleted_rule).await {
                    emit_log(&app, "error", &format!("Reclassify after delete failed: {}", e));
                }
            })
            .await;
    }

    Ok(())
}
