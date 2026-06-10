// Per-case runner.
//
// Creates a fresh conversation in the (copied, throwaway) DB, inserts user and
// assistant message rows, invokes `services::chat::run_chat_turn`, and reads
// back the final assistant row + trace + auto-derived title.

use std::sync::Arc;
use std::time::Instant;

use crate::db::Database;
use crate::evals::case_loader::EvalCase;
use crate::evals::{EvalError, EvalResult};
use crate::models::{ChatConversation, ChatMessage, ChatTrace};
use crate::services::chat;
use crate::services::chat::{smart_body_slice, MAX_SOURCE_BODY_CHARS};
use crate::util::html::strip_html_for_fts;

/// Outcome of running one eval case end to end.
pub struct CaseOutcome {
    pub conversation_id: String,
    pub conversation_title: String,
    pub assistant_message_id: String,
    pub assistant_content: String,
    pub assistant_trace: Option<ChatTrace>,
    pub assistant_token_count: Option<i32>,
    pub assistant_latency_ms: Option<i64>,
    /// Wall-clock time inside the harness (turn dispatch + DB readback).
    pub wall_elapsed_ms: i64,
    pub sources_used: Vec<SourceSummary>,
}

/// Lightweight view of a `ChatMessageSource` for the report.
#[derive(Debug, Clone)]
pub struct SourceSummary {
    pub citation_number: i32,
    pub email_id: String,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub relevance_score: Option<f32>,
    /// The exact body snippet that was fed to the model (post HTML-strip, smart-sliced).
    pub body_snippet: String,
}

/// Run a single case against `services::chat::run_chat_turn`.
///
/// Returns `Err` only on infrastructure-level failures (DB unavailable,
/// assistant row missing, etc). LLM-side failures are captured inside the
/// assistant content / trace and reported as-is.
pub async fn run_case(db: Arc<Database>, account_id: &str, model: &str, case: &EvalCase) -> EvalResult<CaseOutcome> {
    let start = Instant::now();

    // 1. Fresh conversation. Thread-bound cases seed it with the cleaned
    //    thread (role='system' message) so `run_chat_turn` takes the
    //    thread-bound short-circuit; everything else starts as an empty chat
    //    whose title defaults to "New chat" so the auto-title logic triggers.
    let conv: ChatConversation = match case.thread_id.as_deref() {
        Some(thread_id) => crate::services::chat::create_conversation_with_thread(&db, account_id, thread_id)?,
        None => db.create_chat_conversation(account_id, "New chat")?,
    };

    // 2. User + empty assistant rows, matching the production command flow.
    let _user_msg: ChatMessage = db.insert_chat_message(&conv.id, "user", &case.question, None)?;
    let assistant_msg: ChatMessage = db.insert_chat_message(&conv.id, "assistant", "", Some(model))?;

    // 3. Single-turn cases: history is empty (the service re-adds the user
    //    question as the final message itself).
    let history: Vec<ChatMessage> = Vec::new();

    // 4. Run the turn. Errors here are service-level (e.g. Ollama unreachable),
    //    so we wrap them into EvalError but do not swallow — the runner decides
    //    whether that aborts the whole suite.
    // Eval runs use the service default (primary only). Callers that need a
    // wider scope should extend the case schema with a categories field.
    let categories: Vec<String> = crate::services::chat::DEFAULT_RAG_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();

    // run_chat_turn now takes an injected ToolRegistry so production code
    // can hold a single Arc on AppState. Evals don't share state with the
    // running app, so build a fresh default registry per case.
    let registry = std::sync::Arc::new(crate::services::chat::tools::default_registry());
    chat::run_chat_turn(
        db.clone(),
        registry,
        conv.id.clone(),
        assistant_msg.id.clone(),
        account_id.to_string(),
        case.question.clone(),
        model.to_string(),
        history,
        categories,
    )
    .await?;

    let wall_elapsed_ms = start.elapsed().as_millis() as i64;

    // 5. Read back the assistant row (final content + trace + stats + sources).
    let messages = db.get_chat_messages(&conv.id)?;
    let assistant = messages.into_iter().find(|m| m.id == assistant_msg.id).ok_or_else(|| {
        EvalError::Config(format!(
            "assistant message {} missing from conversation {}",
            assistant_msg.id, conv.id
        ))
    })?;

    // 6. Re-read the conversation to pick up the auto-derived title.
    let final_title = db
        .get_chat_conversation(&conv.id)?
        .map(|c| c.title)
        .unwrap_or_else(|| "New chat".into());

    let sources_used = assistant
        .sources
        .iter()
        .map(|s| {
            // Re-derive the exact body snippet that was fed to the model so
            // the report can show exactly what context the LLM received for
            // each source. Mirrors the logic in services::chat::build_prompt.
            let raw_body = db.get_email_body(&s.email_id).unwrap_or_default();
            let plain = strip_html_for_fts(&raw_body);
            let snippet = smart_body_slice(&plain, &case.question, MAX_SOURCE_BODY_CHARS);
            SourceSummary {
                citation_number: s.citation_number,
                email_id: s.email_id.clone(),
                subject: s.subject.clone(),
                sender: s.sender.clone(),
                sender_email: s.sender_email.clone(),
                relevance_score: s.relevance_score,
                body_snippet: snippet,
            }
        })
        .collect();

    Ok(CaseOutcome {
        conversation_id: conv.id,
        conversation_title: final_title,
        assistant_message_id: assistant.id,
        assistant_content: assistant.content,
        assistant_trace: assistant.trace,
        assistant_token_count: assistant.token_count,
        assistant_latency_ms: assistant.latency_ms,
        wall_elapsed_ms,
        sources_used,
    })
}
