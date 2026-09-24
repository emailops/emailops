//! Research-mode prompts: the conversations as read, who is who in them, and
//! the map / condense / reduce prompts split for `complete_with_prefix`. What
//! the reading step returns lives in `reading`, the notes and the report's
//! links in `notes`.

use std::collections::HashMap;

use super::plan::Direction;
use super::reading::Match;

/// How the person asking is named in a message's From / To. Decided in code
/// from the account's address, so the model never has to guess which of the
/// people in a thread is the user.
pub(crate) const USER_LABEL: &str = "YOU (the user)";

/// One message of a conversation as the map step reads it: its new content
/// only (the shared thread reader strips what earlier messages said).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DocMessage {
    pub id: String,
    pub date: String,
    /// The sender, or [`USER_LABEL`].
    pub from: String,
    /// The recipients, the user as [`USER_LABEL`]; empty when unknown.
    pub to: String,
    /// The user wrote this message.
    pub from_user: bool,
    pub text: String,
}

use super::plan::is_user_address as is_user;

/// A sender as the map step shows it: the user as [`USER_LABEL`], anyone
/// else as `Name <address>`. Pure.
pub(crate) fn participant(name: &str, address: &str, user_addresses: &[String]) -> String {
    if is_user(address, user_addresses) {
        USER_LABEL.to_string()
    } else if name.trim().is_empty() || name.trim() == address.trim() {
        address.trim().to_string()
    } else {
        format!("{} <{}>", name.trim(), address.trim())
    }
}

/// Recipients as the map step shows them, the user as [`USER_LABEL`]. Each
/// entry is a bare address or `Name <address>`. Pure.
pub(crate) fn recipients(list: &[String], user_addresses: &[String]) -> String {
    list.iter()
        .map(|r| {
            let address = r
                .rsplit_once('<')
                .and_then(|(_, rest)| rest.split_once('>'))
                .map_or(r.as_str(), |(addr, _)| addr);
            if is_user(address, user_addresses) {
                USER_LABEL.to_string()
            } else {
                r.trim().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the map and reduce steps are told about the question's direction.
/// Empty when the question has none.
pub(crate) fn direction_note(direction: Direction) -> &'static str {
    match direction {
        Direction::Sent => {
            "DIRECTION: the question is about what the user SENT (messages From: YOU). Something \
             another person sent to the user (a quote, offer or invoice the user received or asked \
             for) is not something the user sent: leave it out."
        }
        Direction::Received => {
            "DIRECTION: the question is about what the user RECEIVED (messages To: YOU from \
             someone else). Something the user wrote and sent to others is not something the user \
             received: leave it out."
        }
        Direction::Any => "",
    }
}

/// One conversation as the map step reads it. The unit of reading is the
/// thread, not the email: a reply re-quoting the whole conversation would
/// otherwise be read, and cited, once per reply.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResearchDoc {
    pub thread_id: String,
    pub subject: String,
    pub messages: Vec<DocMessage>,
}

impl ResearchDoc {
    /// Length of the conversation as the reading step shows it, for batching.
    pub(crate) fn rendered_len(&self) -> usize {
        super::reading::BatchLabels::new(0, std::slice::from_ref(self))
            .render()
            .chars()
            .count()
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = &String> {
        self.messages.iter().map(|m| &m.id)
    }
}

/// The map prompt's split point: everything above is the same on every batch
/// of every research turn, so it stays decoded in the one-shot prefix slot.
const MAP_MARKER: &str = "QUESTION: {{question}}";
/// Same for the reduce prompt.
const REDUCE_MARKER: &str = "QUESTION: {{question}}";

/// Render a template and cut it at `marker` into (invariant head, per-call
/// tail). A user-edited template without the marker still works; it only
/// forfeits the prefix cache.
fn split_at_marker(template: &str, marker: &str, vars: &HashMap<&str, String>) -> (String, String) {
    let (head, tail) = match template.find(marker) {
        Some(idx) => template.split_at(idx),
        None => (template, ""),
    };
    (
        crate::services::prompts::render(head, vars),
        crate::services::prompts::render(tail, vars),
    )
}

/// The map prompt for one batch, split for `complete_with_prefix`.
/// `batch` is the batch as `BatchLabels::render` shows it.
pub(crate) fn split_map_prompt(template: &str, question: &str, batch: &str, direction: Direction) -> (String, String) {
    let mut vars = HashMap::new();
    vars.insert("question", question.to_string());
    vars.insert("direction", direction_note(direction).to_string());
    vars.insert("emails", batch.to_string());
    split_at_marker(template, MAP_MARKER, &vars)
}

/// The reduce prompt, split for `complete_with_prefix`. `coverage` is the
/// per-turn "read N emails, M relevant" line — it rides in the tail so the
/// head stays identical across research turns.
pub(crate) fn split_reduce_prompt(
    template: &str,
    language_instruction: &str,
    question: &str,
    coverage: &str,
    counts: &str,
    notes: &str,
    direction: Direction,
) -> (String, String) {
    let mut vars = HashMap::new();
    vars.insert("direction", direction_note(direction).to_string());
    vars.insert("counts", counts.to_string());
    vars.insert("language_instruction", language_instruction.to_string());
    vars.insert("question", question.to_string());
    vars.insert("coverage", coverage.to_string());
    vars.insert("notes", notes.to_string());
    split_at_marker(template, REDUCE_MARKER, &vars)
}

/// The coverage line the report ends on.
/// `planned` is how many were gathered: fewer read means the user stopped it.
pub(crate) fn coverage_line(
    analyzed: usize,
    planned: usize,
    relevant: usize,
    batches: usize,
    failed_batches: usize,
) -> String {
    let read = if analyzed < planned {
        format!("stopped by the user after reading {analyzed} of {planned} emails")
    } else {
        format!("read {analyzed} emails")
    };
    let mut line = format!("{read} in {batches} batches; {relevant} of them had relevant findings");
    if failed_batches > 0 {
        line.push_str(&format!(" ({failed_batches} batches could not be read)"));
    }
    line
}

// ── Matches, exact counts, full list ─────────────────────────────────────────

/// The exact counts the report states — computed, never left to the model.
pub(crate) fn counts_line(matches: &[Match]) -> String {
    let emails: usize = matches.iter().map(|m| m.emails).sum();
    format!(
        "{emails} emails with relevant findings, in {} conversations",
        matches.len()
    )
}

/// The facts line of the report prompt: the exact counts, computed in code.
pub(crate) fn report_facts(matches: &[Match]) -> String {
    format!(
        "{} (exact — computed from every email read; state these numbers, never count the notes yourself).",
        counts_line(matches)
    )
}

/// The answer of a research the user cancelled, in the report's language.
pub(crate) fn cancelled_note(language_code: &str, read: usize, planned: usize) -> String {
    match language_code {
        "es" => format!("Investigación cancelada por el usuario tras leer {read} de {planned} correos."),
        "fr" => format!("Recherche annulée par l'utilisateur après la lecture de {read} e-mails sur {planned}."),
        "de" => format!("Recherche vom Benutzer abgebrochen, nachdem {read} von {planned} E-Mails gelesen wurden."),
        _ => format!("Research cancelled by the user after reading {read} of {planned} emails."),
    }
}

/// Split point of the condense prompt — same convention as map and reduce.
const CONDENSE_MARKER: &str = "QUESTION: {{question}}";

/// The condense prompt for one group of notes, split for `complete_with_prefix`.
pub(crate) fn split_condense_prompt(template: &str, question: &str, notes: &str) -> (String, String) {
    let mut vars = HashMap::new();
    vars.insert("question", question.to_string());
    vars.insert("notes", notes.to_string());
    split_at_marker(template, CONDENSE_MARKER, &vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── map prompt / notes ──

    /// A one-message conversation.
    /// The batch as the reading step shows it.
    fn batch(docs: &[ResearchDoc]) -> String {
        super::super::reading::BatchLabels::new(0, docs).render()
    }

    fn doc(id: &str) -> ResearchDoc {
        conv(&format!("thread-{id}"), &[id])
    }

    /// A conversation of several messages.
    fn conv(thread: &str, ids: &[&str]) -> ResearchDoc {
        ResearchDoc {
            thread_id: thread.into(),
            subject: "Invoice 42".into(),
            messages: ids
                .iter()
                .map(|id| DocMessage {
                    id: (*id).into(),
                    date: "2026-09-01".into(),
                    from: "Alice <alice@example.com>".into(),
                    to: String::new(),
                    from_user: false,
                    text: "Please pay by Friday.".into(),
                })
                .collect(),
        }
    }

    // ── who wrote to whom ──

    fn me() -> Vec<String> {
        vec!["sam@x.example".to_string(), "sam@work.example".to_string()]
    }

    #[test]
    fn an_alias_the_user_sends_from_is_the_user_too() {
        assert_eq!(participant("Sam", "Sam@Work.example", &me()), USER_LABEL);
    }

    #[test]
    fn the_user_is_named_as_you() {
        assert_eq!(participant("Sam", "SAM@x.example", &me()), USER_LABEL);
        assert_eq!(participant("Ana", "ana@x.example", &me()), "Ana <ana@x.example>");
        assert_eq!(participant("", "ana@x.example", &me()), "ana@x.example");
        assert_eq!(participant("ana@x.example", "ana@x.example", &[]), "ana@x.example");
    }

    #[test]
    fn recipients_name_the_user_as_you() {
        let to = vec!["ana@x.example".to_string(), "Sam <sam@x.example>".to_string()];
        assert_eq!(recipients(&to, &me()), format!("ana@x.example, {USER_LABEL}"));
        assert_eq!(recipients(&[], &me()), "");
    }

    #[test]
    fn the_map_prompt_states_the_questions_direction() {
        let template = "HEAD\nQUESTION: {{question}}\n{{direction}}\n{{emails}}";
        let (_, tail) = split_map_prompt(template, "q?", &batch(&[doc("e1")]), Direction::Sent);
        assert!(tail.contains(direction_note(Direction::Sent)), "{tail}");
        let (_, tail) = split_map_prompt(template, "q?", &batch(&[doc("e1")]), Direction::Any);
        assert!(!tail.contains("SENT") && !tail.contains("RECEIVED"), "{tail}");
    }

    #[test]
    fn map_prompt_keeps_the_batch_out_of_the_prefix() {
        let tmpl = "Extract findings.\n\nQUESTION: {{question}}\n\nEMAILS:\n{{emails}}";
        let (prefix, suffix) =
            split_map_prompt(tmpl, "¿qué facturas?", &batch(&[doc("e1"), doc("e2")]), Direction::Any);
        assert_eq!(prefix, "Extract findings.\n\n");
        assert!(suffix.starts_with("QUESTION: ¿qué facturas?"));
        assert!(suffix.contains("EMAIL E1") && suffix.contains("EMAIL E2"));
        // The prefix is identical for another question and batch.
        let (other, _) = split_map_prompt(tmpl, "other", &batch(&[doc("e9")]), Direction::Any);
        assert_eq!(prefix, other);
    }

    #[test]
    fn map_prompt_without_marker_still_renders_everything() {
        let (prefix, suffix) =
            split_map_prompt("Q={{question}} E={{emails}}", "q", &batch(&[doc("e1")]), Direction::Any);
        assert!(prefix.contains("Q=q") && prefix.contains("EMAIL E1"));
        assert!(suffix.is_empty());
    }

    #[test]
    fn reduce_prompt_keeps_per_turn_content_out_of_the_prefix() {
        let tmpl = "Write the report. {{language_instruction}}\n\nQUESTION: {{question}}\nCOVERAGE: {{coverage}}\nNOTES:\n{{notes}}";
        let (prefix, suffix) = split_reduce_prompt(
            tmpl,
            "Reply in Spanish.",
            "q?",
            "read 10",
            "3 emails",
            "- n (email://1)",
            Direction::Any,
        );
        assert_eq!(prefix, "Write the report. Reply in Spanish.\n\n");
        assert!(suffix.contains("q?") && suffix.contains("read 10") && suffix.contains("email://1"));
    }

    #[test]
    fn the_default_prompts_split_on_their_markers() {
        use crate::services::prompts::defaults::{CHAT_RESEARCH_MAP, CHAT_RESEARCH_REDUCE};
        let (prefix, suffix) = split_map_prompt(CHAT_RESEARCH_MAP, "Q?", &batch(&[doc("e1")]), Direction::Any);
        assert!(!prefix.contains("Q?") && !prefix.contains("CONVERSATION C1"));
        assert!(suffix.contains("Q?") && suffix.contains("EMAIL E1"));
        assert!(!suffix.contains("{{"), "unrendered placeholder: {suffix}");

        let (prefix, suffix) = split_reduce_prompt(
            CHAT_RESEARCH_REDUCE,
            "Reply in Spanish.",
            "Q?",
            "read 5",
            "7 emails with relevant findings",
            "- MATCH [1]: a note",
            Direction::Any,
        );
        assert!(prefix.contains("Reply in Spanish."));
        for per_turn in ["Q?", "read 5", "7 emails with relevant findings", "- MATCH [1]: a note"] {
            assert!(!prefix.contains(per_turn), "{per_turn} leaked into the cached prefix");
            assert!(suffix.contains(per_turn));
        }
        assert!(!prefix.contains("{{") && !suffix.contains("{{"));
    }

    #[test]
    fn coverage_mentions_failed_batches_only_when_some_failed() {
        assert_eq!(
            coverage_line(40, 40, 12, 4, 0),
            "read 40 emails in 4 batches; 12 of them had relevant findings"
        );
        assert!(coverage_line(40, 40, 12, 4, 1).ends_with("(1 batches could not be read)"));
    }

    #[test]
    fn coverage_says_when_the_user_stopped_the_reading() {
        assert_eq!(
            coverage_line(30, 100, 12, 3, 0),
            "stopped by the user after reading 30 of 100 emails in 3 batches; 12 of them had relevant findings"
        );
    }

    #[test]
    fn condense_prompt_keeps_the_notes_out_of_the_prefix() {
        use crate::services::prompts::defaults::CHAT_RESEARCH_CONDENSE;
        let (prefix, suffix) = split_condense_prompt(CHAT_RESEARCH_CONDENSE, "Q?", "- n (email://e1)");
        assert!(!prefix.contains("Q?") && !prefix.contains("email://e1"));
        assert!(suffix.contains("Q?") && suffix.contains("- n (email://e1)"));
        assert!(!prefix.contains("{{") && !suffix.contains("{{"));
        let (other, _) = split_condense_prompt(CHAT_RESEARCH_CONDENSE, "other", "- x (email://e2)");
        assert_eq!(prefix, other, "the head is shared by every condense call");
    }

    // ── matches, counts, full list ──

    fn m(id: &str, thread: &str, date: &str, finding: &str) -> Match {
        Match {
            id: id.into(),
            emails: 1,
            thread_id: thread.into(),
            date: date.into(),
            subject: format!("Subject {id}"),
            finding: finding.into(),
        }
    }

    #[test]
    fn counts_are_exact_emails_and_conversations() {
        let matches = vec![
            Match {
                emails: 2,
                ..m("e1", "t1", "", "")
            },
            m("e3", "t2", "", ""),
        ];
        assert_eq!(
            counts_line(&matches),
            "3 emails with relevant findings, in 2 conversations"
        );
    }

    #[test]
    fn report_facts_give_exact_counts() {
        let matches = vec![m("e1", "t1", "", ""), m("e2", "t2", "", "")];
        let facts = report_facts(&matches);
        assert!(
            facts.contains("2 emails with relevant findings, in 2 conversations"),
            "{facts}"
        );
    }

    #[test]
    fn the_cancellation_note_says_how_far_the_reading_got() {
        assert_eq!(
            cancelled_note("es", 10, 30),
            "Investigación cancelada por el usuario tras leer 10 de 30 correos."
        );
        assert_eq!(
            cancelled_note("en", 0, 30),
            "Research cancelled by the user after reading 0 of 30 emails."
        );
    }
}
