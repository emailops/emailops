// Tauri commands for the chat-with-your-emails feature.
//
// All heavy work (retrieval + LLM streaming) is pushed onto `ai_queue` so the
// frontend invoke() returns immediately and the UI can render the user's turn
// and a streaming placeholder driven by `chat-stream` / `chat-sources` events.

use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::{ChatConversation, ChatMessage};
use crate::services::ai::AiService;
use crate::services::chat;
use crate::AppState;

fn emit_log(_app: &AppHandle, level: &str, message: &str) {
    crate::services::logger::log(level, "chat", message);
}

// ── Conversations CRUD ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_chat_conversations(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<ChatConversation>, AppError> {
    chat::list_conversations(&state.db, &account_id)
}

#[tauri::command]
pub async fn create_chat_conversation(
    state: State<'_, AppState>,
    account_id: String,
    title: Option<String>,
) -> Result<ChatConversation, AppError> {
    chat::create_conversation(&state.db, &account_id, title)
}

/// Create a chat session seeded with the cleaned content of an email thread,
/// for the "Chat about this thread" entry point in the inbox row context menu.
///
/// The thread (after quote/signature/footer stripping) is stored as a single
/// role='system' message. `run_chat_turn` detects its presence and runs in
/// thread-bound mode (no RAG retrieval, no tool calls).
#[tauri::command]
pub async fn create_chat_conversation_with_thread(
    state: State<'_, AppState>,
    account_id: String,
    thread_id: String,
) -> Result<ChatConversation, AppError> {
    chat::create_conversation_with_thread(&state.db, &account_id, &thread_id)
}

#[tauri::command]
pub async fn rename_chat_conversation(state: State<'_, AppState>, id: String, title: String) -> Result<(), AppError> {
    chat::rename_conversation(&state.db, &id, &title)
}

#[tauri::command]
pub async fn delete_chat_conversation(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    chat::delete_conversation(&state.db, &id)
}

#[tauri::command]
pub async fn get_chat_messages(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<ChatMessage>, AppError> {
    chat::get_messages(&state.db, &conversation_id)
}

// ── Send a message ─────────────────────────────────────────────────────────

/// Response from `send_chat_message`: contains the pre-created user and
/// assistant message rows so the UI can render the turn immediately and key
/// incoming stream events off `assistant_message_id`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChatResponse {
    pub user_message: ChatMessage,
    pub assistant_message: ChatMessage,
}

/// Canonical set of Gmail-style categories the UI can ask RAG to search.
/// Used to reject unknown values coming from the frontend dropdown.
const VALID_CATEGORIES: &[&str] = &["primary", "updates", "promotions", "social", "forums"];

/// Resolve the category filter for a turn:
///   - `Some(list)` from the frontend → validate + use
///   - `None` → load the persisted `chat.default_categories` preference, or
///     fall back to the service-side default (primary only)
fn resolve_categories(state: &AppState, requested: Option<Vec<String>>) -> Vec<String> {
    let candidate = match requested {
        Some(list) => list,
        None => state
            .db
            .get_preference("chat.default_categories")
            .ok()
            .flatten()
            .map(|s| s.split(',').map(|t| t.to_string()).collect::<Vec<_>>())
            .unwrap_or_default(),
    };
    normalize_categories(candidate)
}

/// Lower-case, drop unknown values, and fall back to the default scope (primary)
/// when nothing valid remains. Pure so it is unit-testable.
///
/// The empty fallback is the important part: an empty category list must NEVER
/// reach the tool layer, because `search_emails` treats empty scope as "no
/// filter → ALL categories" (`tools/search_emails.rs`). Without this, a stray
/// empty selection (a corrupt pref, a `[]` from the UI before the pref loads)
/// silently widens a Primary-scoped chat to every category — the "I see Updates
/// even though only Primary is selected" bug.
fn normalize_categories(candidate: Vec<String>) -> Vec<String> {
    let filtered: Vec<String> = candidate
        .into_iter()
        .map(|c| c.trim().to_lowercase())
        .filter(|c| VALID_CATEGORIES.contains(&c.as_str()))
        .collect();
    if filtered.is_empty() {
        crate::services::chat::DEFAULT_RAG_CATEGORIES
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filtered
    }
}

#[tauri::command]
pub async fn send_chat_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    categories: Option<Vec<String>>,
) -> Result<SendChatResponse, AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput("Message is empty".into()));
    }

    let categories = resolve_categories(&state, categories);

    // Persist the current selection as the per-user default so the next
    // session starts with the same filter.
    if let Err(e) = state
        .db
        .set_preference("chat.default_categories", &categories.join(","))
    {
        emit_log(
            &app,
            "warn",
            &format!("failed to persist chat.default_categories: {}", e),
        );
    }

    // Resolve account from the conversation; refuse if the conversation does
    // not exist. This prevents the frontend from injecting an arbitrary account id.
    let account_id = state
        .db
        .get_chat_conversation_account(&conversation_id)?
        .ok_or_else(|| AppError::NotFound(format!("conversation {}", conversation_id)))?;

    // Log the chat turn to memory (best-effort, must not fail the command).
    crate::services::memory::on_chat_turn(&state.db, &account_id, trimmed);

    // Resolve the model name from preferences. For providers that have a
    // dynamic model list (Ollama), fall back to listing available models.
    let preferred_model = state.db.get_preference("ai_model")?.unwrap_or_default();
    let model = if preferred_model.is_empty() {
        AiService::load_provider(&state.db)
            .ok()
            .map(|p| p.model_name().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "qwen3.5-4b-q4_k_m".to_string())
    } else {
        preferred_model
    };

    // 1. Persist the user turn immediately (sync, fast).
    let user_message = state.db.insert_chat_message(&conversation_id, "user", trimmed, None)?;

    // 2. Pre-create an empty assistant row so the UI can stream into it.
    let assistant_message = state
        .db
        .insert_chat_message(&conversation_id, "assistant", "", Some(&model))?;

    // 3. Gather conversation history (excludes the just-inserted assistant
    //    placeholder because its content is empty — build_prompt filters
    //    non-user/assistant but still, we pass everything up to and including
    //    the user turn, not the placeholder). We query fresh from the DB so
    //    prior turns in long conversations are included correctly.
    let mut history = state.db.get_recent_chat_turns(&conversation_id, 20)?;
    // Drop the empty assistant placeholder and the new user turn — chat
    // service re-adds the user question as the final message itself.
    history.retain(|m| m.id != assistant_message.id && m.id != user_message.id);

    // 4. Dispatch the heavy work to the AI queue and return immediately.
    let db = state.db.clone();
    let registry = state.tool_registry.clone();
    let app_for_task = app.clone();
    let conv_id = conversation_id.clone();
    let user_id = user_message.id.clone();
    let assistant_id = assistant_message.id.clone();
    let user_q = trimmed.to_string();
    let model_for_task = model.clone();

    let categories_for_task = categories.clone();
    let task_label = format!("chat:turn:{}", conversation_id);
    state
        .ai_queue
        .submit_named(&task_label, async move {
            if let Err(e) = chat::run_chat_turn(
                db,
                registry,
                conv_id,
                user_id,
                assistant_id,
                account_id,
                user_q,
                model_for_task,
                history,
                categories_for_task,
            )
            .await
            {
                emit_log(&app_for_task, "error", &format!("chat turn failed: {}", e));
            }
        })
        .await;

    Ok(SendChatResponse {
        user_message,
        assistant_message,
    })
}

#[cfg(test)]
mod tests {
    use super::normalize_categories;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keeps_a_valid_single_scope() {
        assert_eq!(normalize_categories(v(&["primary"])), v(&["primary"]));
    }

    #[test]
    fn empty_falls_back_to_primary_not_everything() {
        // The whole point: empty must NOT become "all categories" downstream.
        assert_eq!(normalize_categories(v(&[])), v(&["primary"]));
    }

    #[test]
    fn all_invalid_falls_back_to_primary() {
        assert_eq!(normalize_categories(v(&["bogus", "spam"])), v(&["primary"]));
    }

    #[test]
    fn lowercases_and_drops_unknowns_keeping_valid() {
        assert_eq!(
            normalize_categories(v(&["Updates", "PROMOTIONS", "nope"])),
            v(&["updates", "promotions"])
        );
    }

    #[test]
    fn preserves_a_full_explicit_all_selection() {
        let all = v(&["primary", "updates", "promotions", "social", "forums"]);
        assert_eq!(normalize_categories(all.clone()), all);
    }
}
