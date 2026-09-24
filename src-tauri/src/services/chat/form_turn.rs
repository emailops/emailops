//! The turn that fills an app form.
//!
//! A `form` verdict from the query planner short-circuits the ordinary turn:
//! there is nothing to retrieve, no tool to call and nothing for the chat model
//! to synthesise. One focused completion fills the fields
//! (`services::forms::filler`), the frontend opens the form with them, and the
//! assistant message is composed here — deterministically, so the turn costs
//! exactly one model call and the wording is unit-testable rather than
//! re-rolled every run.
//!
//! Pure planner (`compose_fill_reply`, `fill_effect`) + thin executor
//! (`run_form_fill_turn`), per the repo's rule.

use super::tools::ToolEffect;
use crate::services::forms::{registry::FormDef, FormFill};
use crate::services::i18n::Language;

/// The effect that opens the form on screen with the filled values.
pub fn fill_effect(form: &FormDef, fill: &FormFill) -> ToolEffect {
    ToolEffect::FillForm {
        form_id: form.id.to_string(),
        target: form.target.to_string(),
        values: serde_json::Value::Object(fill.values.clone()),
        missing_required: fill.missing_required.clone(),
    }
}

/// What the assistant says after filling a form.
///
/// Deterministic and localised in Rust rather than generated: the useful
/// content is the form itself, now open on screen, so paying a second model
/// call to narrate it would be waste — and a generated sentence could claim a
/// field it did not fill. The sentence is the form's own (`FormDef::fill_reply`).
pub fn compose_fill_reply(form: &FormDef, fill: &FormFill, lang: Language) -> String {
    let opened = (form.fill_reply)(lang).to_string();
    if fill.missing_required.is_empty() {
        return opened;
    }
    let missing = fill.missing_required.join(", ");
    let tail = match lang {
        Language::En => format!("Still needed: {missing}."),
        Language::Es => format!("Falta por rellenar: {missing}."),
        Language::Fr => format!("Encore nécessaire : {missing}."),
        Language::De => format!("Noch erforderlich: {missing}."),
    };
    format!("{opened} {tail}")
}

/// The reasoning trace of a form-fill turn: the route, the one completion that
/// filled the form, and what it filled (or that it produced nothing usable).
/// Without it the reasoning panel had nothing to show for these turns.
#[allow(clippy::too_many_arguments)]
pub fn form_fill_trace(
    form: &FormDef,
    fill: Option<&FormFill>,
    model: &str,
    fill_ms: i64,
    prompt_tokens: u32,
    prefill_ms: Option<i64>,
    total_ms: i64,
) -> ChatTrace {
    let result_preview = match fill {
        Some(f) => {
            let filled: Vec<&str> = f.values.keys().map(String::as_str).collect();
            let mut s = format!("filled: {}", filled.join(", "));
            if !f.missing_required.is_empty() {
                s.push_str(&format!(" · missing: {}", f.missing_required.join(", ")));
            }
            s
        }
        None => "the model produced nothing usable — form opened empty".to_string(),
    };
    super::trace_steps::with_steps(ChatTrace {
        route: RouteDecision {
            mode: RouteMode::ToolsFirst,
            reason: format!("form fill ({})", form.id),
            matched_keywords: vec![],
            classifier: "planner".to_string(),
        },
        retrieval: None,
        tool_calls: vec![ToolCallTrace {
            name: "fill_form".to_string(),
            round: 0,
            arguments: serde_json::json!({ "form": form.id }),
            result_chars: i32::try_from(result_preview.len()).unwrap_or(i32::MAX),
            result_preview,
            elapsed_ms: fill_ms,
        }],
        model: model.to_string(),
        total_elapsed_ms: total_ms,
        tool_loop_ms: 0,
        llm_streaming_ms: None,
        llm_calls: vec![LlmCallTrace {
            kind: "form_fill".to_string(),
            round: 0,
            latency_ms: fill_ms,
            tool_calls_requested: 0,
            failed: fill.is_none(),
            prompt_tokens: (prompt_tokens > 0).then_some(prompt_tokens),
            prefill_ms,
            cached_prompt_tokens: None,
            prefix_plan: None,
            sys_cached_before: None,
            sys_cached_after: None,
            system_prefix_tokens: None,
            stable_tokens: None,
            dropped_front_tokens: None,
            input: None,
            output: None,
        }],
        help: None,
        steps: Vec::new(),
    })
}

/// What the assistant says when the model produced nothing usable. The form
/// still opens — empty — because an open form the user can fill beats an error
/// message they have to act on themselves.
pub fn compose_empty_reply(lang: Language) -> String {
    match lang {
        Language::En => "I opened the form but could not fill it in — please complete it.".into(),
        Language::Es => "He abierto el formulario pero no he podido rellenarlo — complétalo tú.".into(),
        Language::Fr => "J'ai ouvert le formulaire mais n'ai pas pu le remplir — complétez-le.".into(),
        Language::De => "Ich habe das Formular geöffnet, konnte es aber nicht ausfüllen — bitte ergänzen.".into(),
    }
}

// ── Executor ────────────────────────────────────────────────────────────────

use crate::db::Database;
use crate::models::{
    ChatPhase, ChatStreamEvent, ChatTrace, ChatTraceEvent, LlmCallTrace, RouteDecision, RouteMode, ToolCallTrace,
};
use crate::services::forms::filler::{fill_form, FillRun};
use crate::AppError;
use std::sync::Arc;

/// Run a form-fill turn end to end: one focused completion, one effect, one
/// deterministic answer. No retrieval, no tool loop, no second model call.
///
/// Returns `Err` only when the assistant row itself cannot be written; an
/// unusable model reply is a successful turn that opens an empty form.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_form_fill_turn(
    db: &Arc<Database>,
    provider: &dyn crate::ai::provider::AIProvider,
    conversation_id: &str,
    assistant_message_id: &str,
    form: &'static FormDef,
    language: Language,
    today: &str,
    current_values: &serde_json::Value,
    user_question: &str,
    turn_start: std::time::Instant,
) -> Result<(), AppError> {
    use super::{emit_log, emit_phase};

    emit_phase(conversation_id, assistant_message_id, ChatPhase::Generating);
    let template = crate::services::prompts::get_template(db, "forms.fill")?;

    let t_fill = std::time::Instant::now();
    let run: FillRun = fill_form(
        provider,
        &template,
        form,
        language.english_name(),
        today,
        current_values,
        user_question,
    )
    .await;
    let fill_ms = t_fill.elapsed().as_millis() as i64;

    let (answer, effect) = match &run.fill {
        Some(fill) => {
            emit_log(
                "info",
                &format!(
                    "form: filled {} ({} field(s), {} missing) [{fill_ms}ms]",
                    form.id,
                    fill.values.len(),
                    fill.missing_required.len()
                ),
            );
            (compose_fill_reply(form, fill, language), fill_effect(form, fill))
        }
        None => {
            emit_log("error", &format!("form: could not fill {} [{fill_ms}ms]", form.id));
            let empty = FormFill {
                form_id: form.id.to_string(),
                values: serde_json::Map::new(),
                missing_required: form
                    .fields
                    .iter()
                    .filter(|f| f.required)
                    .map(|f| f.key.to_string())
                    .collect(),
                dropped: Vec::new(),
            };
            (compose_empty_reply(language), fill_effect(form, &empty))
        }
    };

    // Open the form first, so it is already on screen when the sentence lands.
    crate::services::events::emit("chat-tool-effect", effect);

    let latency_ms = turn_start.elapsed().as_millis() as i64;
    if let Err(e) = db.update_chat_message_completion(assistant_message_id, &answer, None, Some(latency_ms)) {
        emit_log("error", &format!("failed to persist assistant message: {e}"));
        return Err(e);
    }

    let trace = form_fill_trace(
        form,
        run.fill.as_ref(),
        provider.model_name(),
        fill_ms,
        run.prompt_tokens,
        run.prefill_ms,
        latency_ms,
    );
    if let Err(e) = db.update_chat_message_trace(assistant_message_id, &trace) {
        emit_log("error", &format!("failed to persist reasoning trace: {e}"));
    }
    // The live UI gets traces from this event, not from the DB row: without
    // it the panel showed no reasoning for a form fill until a reload.
    crate::services::events::emit(
        "chat-trace",
        ChatTraceEvent {
            message_id: assistant_message_id.to_string(),
            conversation_id: conversation_id.to_string(),
            trace,
            referenced_email_ids: Vec::new(),
            referenced_draft_ids: Vec::new(),
        },
    );

    crate::services::events::emit(
        "chat-stream",
        ChatStreamEvent {
            message_id: assistant_message_id.to_string(),
            conversation_id: conversation_id.to_string(),
            token: answer,
            done: true,
            error: None,
            token_count: None,
            latency_ms: Some(latency_ms),
            replace: Some(true),
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::forms::registry::LENS_CREATE;
    use serde_json::json;

    fn fill(values: serde_json::Value, missing: &[&str]) -> FormFill {
        FormFill {
            form_id: "lens.create".into(),
            values: values.as_object().cloned().unwrap_or_default(),
            missing_required: missing.iter().map(|s| (*s).to_string()).collect(),
            dropped: Vec::new(),
        }
    }

    #[test]
    fn the_effect_carries_the_form_id_and_its_nav_target() {
        let effect = fill_effect(&LENS_CREATE, &fill(json!({"name": "X"}), &[]));
        match effect {
            ToolEffect::FillForm { form_id, target, .. } => {
                assert_eq!(form_id, "lens.create");
                assert_eq!(target, "view/lenses#create");
            }
            other => panic!("expected FillForm, got {other:?}"),
        }
    }

    #[test]
    fn the_effect_carries_the_values_as_a_json_object() {
        let effect = fill_effect(&LENS_CREATE, &fill(json!({"name": "Facturas"}), &[]));
        match effect {
            ToolEffect::FillForm { values, .. } => {
                assert_eq!(values.get("name").and_then(|v| v.as_str()), Some("Facturas"));
            }
            other => panic!("expected FillForm, got {other:?}"),
        }
    }

    #[test]
    fn the_effect_forwards_the_fields_the_model_could_not_fill() {
        let effect = fill_effect(&LENS_CREATE, &fill(json!({"name": "X"}), &["columns"]));
        match effect {
            ToolEffect::FillForm { missing_required, .. } => {
                assert_eq!(missing_required, vec!["columns"]);
            }
            other => panic!("expected FillForm, got {other:?}"),
        }
    }

    #[test]
    fn the_effect_serializes_with_a_kind_tag_the_frontend_dispatcher_switches_on() {
        let effect = fill_effect(&LENS_CREATE, &fill(json!({"name": "X"}), &[]));
        let wire = serde_json::to_value(&effect).expect("serializes");
        assert_eq!(wire.get("kind").and_then(|v| v.as_str()), Some("fillForm"));
        assert!(wire.get("formId").is_some(), "camelCase on the wire: {wire}");
    }

    #[test]
    fn the_lens_reply_says_the_lens_is_configured_and_asks_to_review_and_save() {
        // The form is a draft the model wrote: the user must check it and
        // press save, or no Lens exists.
        let reply = compose_fill_reply(
            &LENS_CREATE,
            &fill(json!({"name": "X", "promptText": "p"}), &[]),
            Language::Es,
        );
        assert_eq!(
            reply,
            "He configurado la lente rellenando el formulario. Puede haber errores o partes incompletas: revísalo y guárdalo para que se cree la lente."
        );
    }

    #[test]
    fn a_partial_fill_names_what_is_still_missing() {
        let reply = compose_fill_reply(
            &LENS_CREATE,
            &fill(json!({"name": "X"}), &["columns", "promptText"]),
            Language::Es,
        );
        assert!(reply.contains("columns"));
        assert!(reply.contains("promptText"));
    }

    #[test]
    fn every_language_gets_its_own_wording() {
        let f = fill(json!({"name": "X"}), &[]);
        let replies: Vec<String> = Language::ALL
            .iter()
            .map(|l| compose_fill_reply(&LENS_CREATE, &f, *l))
            .collect();
        let mut unique = replies.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), Language::ALL.len(), "untranslated reply in {replies:?}");
    }

    #[test]
    fn the_trace_shows_the_fill_call_and_what_it_filled() {
        // The reasoning panel had nothing to show for a form fill: no model
        // call, no step. It now lists the one completion and its outcome.
        let f = fill(json!({"name": "X", "columns": []}), &["promptText"]);
        let trace = form_fill_trace(&LENS_CREATE, Some(&f), "some-model", 5_000, 812, Some(640), 5_200);
        assert_eq!(trace.model, "some-model");
        assert_eq!(trace.llm_calls.len(), 1);
        assert_eq!(trace.llm_calls[0].kind, "form_fill");
        assert_eq!(trace.llm_calls[0].latency_ms, 5_000);
        assert_eq!(trace.llm_calls[0].prompt_tokens, Some(812));
        assert_eq!(trace.tool_calls.len(), 1);
        let call = &trace.tool_calls[0];
        assert_eq!(call.name, "fill_form");
        assert_eq!(call.arguments["form"], "lens.create");
        assert!(call.result_preview.contains("name"), "{}", call.result_preview);
        assert!(call.result_preview.contains("promptText"), "{}", call.result_preview);
        assert_eq!(trace.steps.len(), 3, "route, the model call, the fill");
    }

    #[test]
    fn the_trace_of_an_unusable_reply_marks_the_call_failed() {
        let trace = form_fill_trace(&LENS_CREATE, None, "m", 100, 0, None, 120);
        assert!(trace.llm_calls[0].failed);
    }

    #[test]
    fn every_language_gets_its_own_empty_wording() {
        let mut replies: Vec<String> = Language::ALL.iter().map(|l| compose_empty_reply(*l)).collect();
        replies.sort();
        replies.dedup();
        assert_eq!(replies.len(), Language::ALL.len());
    }
}
