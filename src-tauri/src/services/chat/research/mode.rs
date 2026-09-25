//! How a research answer is delivered, decided once per question before
//! reading starts.
//!
//! A list or a count needs no report: every conversation the reading step
//! matched already carries its own verdict, so code writes the answer — exact
//! counts, every match linked — and the condense and report calls are
//! skipped. Only a question that asks for something worked out across the
//! matches (a trend, a summary, a comparison, a total) pays for the report.
//! When unsure, the classifier answers `analysis`: it still serves a list,
//! only slower.

use std::collections::HashMap;

use crate::ai::json_shape::JsonShape;
use crate::models::ReportMode;

use super::notes::render_match_list;
use super::reading::Match;

/// The classifier prompt's split point, like the other research prompts.
const MODE_MARKER: &str = "QUESTION: {{question}}";

/// The classifier prompt, split for `complete_with_prefix`: its instructions
/// are the same for every question.
pub(crate) fn split_mode_prompt(template: &str, question: &str) -> (String, String) {
    let (head, tail) = match template.find(MODE_MARKER) {
        Some(idx) => template.split_at(idx),
        None => (template, ""),
    };
    let vars = HashMap::from([("question", question.to_string())]);
    (
        crate::services::prompts::render(head, &vars),
        crate::services::prompts::render(tail, &vars),
    )
}

/// The only reply the classifier may give.
pub(crate) fn mode_shape() -> JsonShape {
    JsonShape::object(vec![("report", JsonShape::one_of(&["list", "count", "analysis"]))])
}

/// The mode in a classifier reply; `Analysis` for anything else — a failed
/// or garbled call must not cost the user the report.
pub(crate) fn parse_mode(reply: &str) -> ReportMode {
    let report = serde_json::from_str::<serde_json::Value>(reply.trim())
        .ok()
        .and_then(|v| v.get("report").and_then(|r| r.as_str()).map(str::to_string));
    match report.as_deref() {
        Some("list") => ReportMode::List,
        Some("count") => ReportMode::Count,
        _ => ReportMode::Analysis,
    }
}

/// `n` with the singular or plural word.
fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The answer to a list or count question, written in code from the matches:
/// the exact counts, the full list, and how much was read. Pure.
pub(crate) fn list_answer(
    mode: ReportMode,
    matches: &[Match],
    read: usize,
    batches: usize,
    failed_batches: usize,
    language_code: &str,
) -> String {
    let emails: usize = matches.iter().map(|m| m.emails).sum();
    let (conv1, convn, mail1, mailn) = match language_code {
        "es" => ("conversación", "conversaciones", "correo", "correos"),
        "fr" => ("conversation", "conversations", "e-mail", "e-mails"),
        "de" => ("Unterhaltung", "Unterhaltungen", "E-Mail", "E-Mails"),
        _ => ("conversation", "conversations", "email", "emails"),
    };
    let conversations = counted(matches.len(), conv1, convn);
    let email_count = counted(emails, mail1, mailn);
    let conversations = if mode == ReportMode::Count {
        format!("**{conversations}**")
    } else {
        conversations
    };
    let lead = if matches.is_empty() {
        let read = counted(read, mail1, mailn);
        match language_code {
            "es" => format!("Ninguna conversación de los {read} leídos responde a tu pregunta."),
            "fr" => format!("Aucune conversation parmi les {read} lus ne répond à votre question."),
            "de" => format!("Keine Unterhaltung unter den {read} gelesenen beantwortet deine Frage."),
            _ => format!("No conversation among the {read} read answers your question."),
        }
    } else {
        let one = matches.len() == 1;
        let pick = |singular: &'static str, plural: &'static str| if one { singular } else { plural };
        match language_code {
            "es" => format!(
                "{conversations} ({email_count}) {} a tu pregunta.",
                pick("responde", "responden")
            ),
            "fr" => format!(
                "{conversations} ({email_count}) {} à votre question.",
                pick("répond", "répondent")
            ),
            "de" => format!(
                "{conversations} ({email_count}) {} deine Frage.",
                pick("beantwortet", "beantworten")
            ),
            _ => format!(
                "{conversations} ({email_count}) {} your question.",
                pick("answers", "answer")
            ),
        }
    };
    let read_line = {
        let read = counted(read, mail1, mailn);
        let mut line = match language_code {
            "es" => format!("Leídos {read} en {batches} lotes."),
            "fr" => format!("{read} lus en {batches} lots."),
            "de" => format!("{read} in {batches} Durchgängen gelesen."),
            _ => format!("Read {read} in {batches} batches."),
        };
        if failed_batches > 0 {
            line.push_str(&match language_code {
                "es" => format!(" {failed_batches} lotes no se pudieron leer."),
                "fr" => format!(" {failed_batches} lots n'ont pas pu être lus."),
                "de" => format!(" {failed_batches} Durchgänge konnten nicht gelesen werden."),
                _ => format!(" {failed_batches} batches could not be read."),
            });
        }
        line
    };
    let list = render_match_list(matches, language_code);
    if list.is_empty() {
        format!("{lead}\n\n{read_line}")
    } else {
        format!("{lead}\n\n{list}\n{read_line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_names_the_mode_and_anything_else_is_analysis() {
        assert_eq!(parse_mode(r#"{"report":"list"}"#), ReportMode::List);
        assert_eq!(parse_mode(r#" {"report": "count"} "#), ReportMode::Count);
        assert_eq!(parse_mode(r#"{"report":"analysis"}"#), ReportMode::Analysis);
        assert_eq!(parse_mode(r#"{"report":"table"}"#), ReportMode::Analysis);
        assert_eq!(parse_mode("list"), ReportMode::Analysis, "not the JSON asked for");
        assert_eq!(parse_mode(""), ReportMode::Analysis);
    }

    #[test]
    fn the_classifier_may_only_answer_one_of_the_three_modes() {
        let schema = mode_shape().to_json_schema();
        assert_eq!(
            schema["properties"]["report"]["enum"],
            serde_json::json!(["list", "count", "analysis"])
        );
    }

    #[test]
    fn the_question_stays_out_of_the_cached_prefix() {
        let (head, tail) = split_mode_prompt("Decide.\nQUESTION: {{question}}", "¿cuántos?");
        assert_eq!(head, "Decide.\n");
        assert_eq!(tail, "QUESTION: ¿cuántos?");
    }

    fn a_match(id: &str, emails: usize) -> Match {
        Match {
            id: id.into(),
            emails,
            thread_id: format!("t-{id}"),
            date: "2026-09-01".into(),
            subject: format!("Quote {id}"),
            finding: "Quote sent".into(),
        }
    }

    #[test]
    fn a_list_answer_states_exact_counts_then_every_match_then_what_was_read() {
        let matches = vec![a_match("a", 2), a_match("b", 1)];
        let answer = list_answer(ReportMode::List, &matches, 434, 18, 0, "es");
        assert!(
            answer.starts_with("2 conversaciones (3 correos) responden a tu pregunta.\n\n### Lista completa (2)"),
            "{answer}"
        );
        assert!(answer.contains("[Quote a](email://a)"), "{answer}");
        assert!(answer.ends_with("Leídos 434 correos en 18 lotes."), "{answer}");
    }

    #[test]
    fn a_count_answer_leads_with_the_number() {
        let answer = list_answer(ReportMode::Count, &[a_match("a", 1)], 10, 1, 0, "en");
        assert!(
            answer.starts_with("**1 conversation** (1 email) answers your question."),
            "{answer}"
        );
    }

    #[test]
    fn no_match_says_so_and_what_was_read() {
        let answer = list_answer(ReportMode::List, &[], 40, 4, 1, "en");
        assert_eq!(
            answer,
            "No conversation among the 40 emails read answers your question.\n\nRead 40 emails in 4 batches. 1 batches could not be read."
        );
    }
}
