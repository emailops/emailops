//! The reading (map) step's contract: how a batch of conversations is shown
//! to the model, the JSON shape its reply must take, and what code makes of
//! that reply.
//!
//! The reply is structured, not free text. Each entry is one conversation's
//! verdict — `{conversation, text, tag, emails}` — naming the conversation and
//! its emails by short batch labels (`C1`, `E3`), never by id. On llama.cpp a
//! grammar confines those labels to the batch's own, so an entry can only cite
//! what was read; elsewhere the provider's JSON-schema mode does the same job
//! and code drops anything that slips through. Because there is at most one
//! entry per conversation and each is bounded, the reply always fits the
//! step's output budget: nothing is cut mid-batch.

use std::collections::HashMap;

use crate::ai::json_shape::JsonShape;

use super::plan::Direction;
use super::prompts::{DocMessage, ResearchDoc};

/// Longest entry text, in characters.
pub(crate) const MAX_FINDING_CHARS: usize = 240;
/// Most emails one entry may cite.
const MAX_FINDING_EMAILS: usize = 3;

/// Whether an entry answers the question or only gives background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingTag {
    /// The conversation contains what the question asks about: a match.
    Match,
    /// Related, but not an answer (a request, a question about it, a reply,
    /// the same thing from someone else): background, never a match.
    Context,
}

/// One conversation's verdict from the reading step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Finding {
    /// Index of the conversation in the run's reading order.
    pub doc: usize,
    pub tag: FindingTag,
    pub text: String,
    /// Ids of the conversation's emails the entry comes from (only emails of
    /// that conversation).
    pub emails: Vec<String>,
}

/// The labels one batch is shown with: `C1…` for its conversations, `E1…`
/// for its emails, numbered through the batch in reading order.
pub(crate) struct BatchLabels<'a> {
    /// Index of the batch's first conversation in the run.
    first_doc: usize,
    docs: &'a [ResearchDoc],
}

impl<'a> BatchLabels<'a> {
    pub(crate) fn new(first_doc: usize, docs: &'a [ResearchDoc]) -> Self {
        Self { first_doc, docs }
    }

    fn conversation_labels(&self) -> Vec<String> {
        (1..=self.docs.len()).map(|i| format!("C{i}")).collect()
    }

    /// Every email of the batch with its label, in order.
    fn emails(&self) -> impl Iterator<Item = (String, usize, &'a DocMessage)> {
        self.docs
            .iter()
            .enumerate()
            .flat_map(|(c, d)| d.messages.iter().map(move |m| (c, m)))
            .enumerate()
            .map(|(e, (c, m))| (format!("E{}", e + 1), c, m))
    }

    /// The batch as the reading step sees it.
    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        let mut emails = self.emails().peekable();
        for (c, doc) in self.docs.iter().enumerate() {
            out.push_str(&format!("CONVERSATION C{}: {}\n", c + 1, doc.subject));
            while let Some((label, _, m)) = emails.next_if(|(_, ec, _)| *ec == c) {
                let to = if m.to.is_empty() {
                    String::new()
                } else {
                    format!("To: {}\n", m.to)
                };
                out.push_str(&format!(
                    "EMAIL {label} · {}\nFrom: {}\n{to}{}\n\n",
                    m.date, m.from, m.text
                ));
            }
        }
        out
    }

    /// The JSON shape the reply must take: at most one entry per conversation,
    /// each citing only this batch's labels.
    pub(crate) fn shape(&self) -> JsonShape {
        let email_labels: Vec<String> = self.emails().map(|(label, _, _)| label).collect();
        let entry = JsonShape::object(vec![
            ("conversation", JsonShape::one_of(&self.conversation_labels())),
            (
                "text",
                JsonShape::String {
                    max_len: MAX_FINDING_CHARS,
                },
            ),
            ("tag", JsonShape::one_of(&["match", "context"])),
            (
                "emails",
                JsonShape::array(JsonShape::one_of(&email_labels), 1, MAX_FINDING_EMAILS),
            ),
        ]);
        JsonShape::object(vec![("findings", JsonShape::array(entry, 0, self.docs.len()))])
    }

    /// The findings in a reading reply. An email label that is not in the
    /// entry's own conversation is dropped (the grammar keeps labels in the
    /// batch but cannot tie them to one conversation); an entry for a
    /// conversation already covered is merged into the first. `Err` when the
    /// reply is not the expected JSON — a provider without structured output
    /// that ignored the instructions.
    pub(crate) fn parse(&self, reply: &str) -> Result<Vec<Finding>, String> {
        let value = parse_json_object(reply)?;
        let entries = value
            .get("findings")
            .and_then(|f| f.as_array())
            .ok_or_else(|| "the reading reply has no \"findings\" list".to_string())?;
        let emails: HashMap<String, (usize, &DocMessage)> =
            self.emails().map(|(label, c, m)| (label, (c, m))).collect();
        let mut out: Vec<Finding> = Vec::new();
        for entry in entries {
            let Some(c) = entry
                .get("conversation")
                .and_then(|v| v.as_str())
                .and_then(|l| l.strip_prefix('C'))
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| n.checked_sub(1))
                .filter(|c| *c < self.docs.len())
            else {
                continue;
            };
            let text = entry
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let tag = match entry.get("tag").and_then(|v| v.as_str()) {
                Some("context") => FindingTag::Context,
                _ => FindingTag::Match,
            };
            let cited: Vec<String> = entry
                .get("emails")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|l| l.as_str())
                .filter_map(|l| emails.get(l))
                .filter(|(ec, _)| *ec == c)
                .map(|(_, m)| m.id.clone())
                .collect();
            let doc = self.first_doc + c;
            match out.iter_mut().find(|f| f.doc == doc) {
                Some(first) => {
                    // One verdict per conversation: a second entry adds its
                    // emails, and a match anywhere makes the conversation one.
                    if tag == FindingTag::Match {
                        first.tag = FindingTag::Match;
                    }
                    for id in cited {
                        if !first.emails.contains(&id) {
                            first.emails.push(id);
                        }
                    }
                }
                None => out.push(Finding {
                    doc,
                    tag,
                    text,
                    emails: cited,
                }),
            }
        }
        Ok(out)
    }
}

/// The JSON object in a reply: the reply itself, or — from a provider that
/// wraps it in prose or a code fence — the span from its first `{` to its
/// last `}`.
fn parse_json_object(reply: &str) -> Result<serde_json::Value, String> {
    let trimmed = reply.trim();
    serde_json::from_str::<serde_json::Value>(trimmed)
        .or_else(|first| {
            let start = trimmed.find('{').ok_or_else(|| first.to_string())?;
            let end = trimmed.rfind('}').ok_or_else(|| first.to_string())?;
            serde_json::from_str(&trimmed[start..=end]).map_err(|e| e.to_string())
        })
        .map_err(|e| format!("the reading reply is not JSON: {e}"))
}

/// Holds the findings to who actually wrote each email. On a question with a
/// direction, a match citing only emails from the other side is a misreading
/// (the supplier's quote read as the user's own): it becomes background, with
/// who wrote it, so neither the list nor the report can state it as an
/// answer. Pure.
pub(crate) fn enforce_direction(findings: Vec<Finding>, docs: &[ResearchDoc], direction: Direction) -> Vec<Finding> {
    if direction == Direction::Any {
        return findings;
    }
    findings
        .into_iter()
        .map(|mut f| {
            if f.tag != FindingTag::Match {
                return f;
            }
            let cited: Vec<&DocMessage> = docs
                .get(f.doc)
                .into_iter()
                .flat_map(|d| d.messages.iter())
                .filter(|m| f.emails.contains(&m.id))
                .collect();
            let on_side = match direction {
                Direction::Sent => cited.iter().any(|m| m.from_user),
                Direction::Received => cited.iter().any(|m| !m.from_user),
                Direction::Any => true,
            };
            if !on_side {
                f.tag = FindingTag::Context;
                let writer = match (direction, cited.first()) {
                    (Direction::Sent, Some(m)) => format!("written by {}, not by the user", m.from),
                    (Direction::Sent, None) => "no email by the user cited".to_string(),
                    _ => "written by the user".to_string(),
                };
                f.text = format!("{} [{writer}]", f.text);
            }
            f
        })
        .collect()
}

/// One conversation the reading step found to answer the question: its first
/// cited email on the question's side (the representative the list and the
/// report link to), how many of its emails were cited, and the verdict's text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Match {
    pub id: String,
    /// Emails of this conversation the verdict cites.
    pub emails: usize,
    pub thread_id: String,
    pub date: String,
    pub subject: String,
    pub finding: String,
}

/// Every conversation with a match verdict, in reading order. Taken from the
/// reading findings, before any condense round, so merging notes for the
/// report never drops a match from the list or the count. Expects findings
/// already held to the direction (see [`enforce_direction`]).
pub(crate) fn collect_matches(docs: &[ResearchDoc], findings: &[Finding], direction: Direction) -> Vec<Match> {
    let mut out = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let Some(f) = findings.iter().find(|f| f.doc == i && f.tag == FindingTag::Match) else {
            continue;
        };
        let cited: Vec<&DocMessage> = doc
            .messages
            .iter()
            .filter(|m| f.emails.contains(&m.id))
            .filter(|m| match direction {
                Direction::Sent => m.from_user,
                Direction::Received => !m.from_user,
                Direction::Any => true,
            })
            .collect();
        // A verdict whose labels all fell outside its conversation still names
        // the conversation: link its first email.
        let Some(head) = cited.first().copied().or_else(|| doc.messages.first()) else {
            continue;
        };
        out.push(Match {
            id: head.id.clone(),
            emails: cited.len().max(1),
            thread_id: doc.thread_id.clone(),
            date: head.date.clone(),
            subject: doc.subject.clone(),
            finding: f.text.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, from_user: bool) -> DocMessage {
        DocMessage {
            id: id.into(),
            date: "2026-09-01".into(),
            from: if from_user {
                super::super::prompts::USER_LABEL.into()
            } else {
                "Alice <alice@example.com>".into()
            },
            to: String::new(),
            from_user,
            text: "The email text.".into(),
        }
    }

    fn conv(thread: &str, messages: Vec<DocMessage>) -> ResearchDoc {
        ResearchDoc {
            thread_id: thread.into(),
            subject: format!("Subject {thread}"),
            messages,
        }
    }

    fn batch() -> Vec<ResearchDoc> {
        vec![
            conv("t1", vec![msg("id-a", true)]),
            conv("t2", vec![msg("id-b", true), msg("id-c", false)]),
        ]
    }

    #[test]
    fn a_batch_is_shown_with_short_labels_not_ids() {
        let docs = batch();
        let text = BatchLabels::new(0, &docs).render();
        assert!(
            text.starts_with("CONVERSATION C1: Subject t1\nEMAIL E1 · 2026-09-01\n"),
            "{text}"
        );
        assert!(text.contains("CONVERSATION C2: Subject t2\nEMAIL E2 ·"), "{text}");
        assert!(text.contains("EMAIL E3 ·"), "{text}");
        assert!(!text.contains("id-a"), "ids stay out of the prompt: {text}");
    }

    #[test]
    fn the_reply_shape_allows_only_this_batchs_labels() {
        let docs = batch();
        let schema = BatchLabels::new(0, &docs).shape().to_json_schema();
        let entry = &schema["properties"]["findings"]["items"];
        assert_eq!(
            schema["properties"]["findings"]["maxItems"], 2,
            "one entry per conversation"
        );
        assert_eq!(
            entry["properties"]["conversation"]["enum"],
            serde_json::json!(["C1", "C2"])
        );
        assert_eq!(
            entry["properties"]["emails"]["items"]["enum"],
            serde_json::json!(["E1", "E2", "E3"])
        );
        assert_eq!(entry["properties"]["text"]["maxLength"], MAX_FINDING_CHARS);
    }

    #[test]
    fn a_reply_becomes_findings_on_the_runs_conversations_and_ids() {
        let docs = batch();
        let reply = r#"{"findings":[
            {"conversation":"C2","text":"Supplier quoted 900 EUR","tag":"context","emails":["E3"]},
            {"conversation":"C1","text":"Sent a quote for 8,400 EUR","tag":"match","emails":["E1"]}]}"#;
        let found = BatchLabels::new(10, &docs).parse(reply).unwrap();
        assert_eq!(
            found,
            vec![
                Finding {
                    doc: 11,
                    tag: FindingTag::Context,
                    text: "Supplier quoted 900 EUR".into(),
                    emails: vec!["id-c".into()]
                },
                Finding {
                    doc: 10,
                    tag: FindingTag::Match,
                    text: "Sent a quote for 8,400 EUR".into(),
                    emails: vec!["id-a".into()]
                },
            ]
        );
    }

    #[test]
    fn an_email_label_from_another_conversation_is_dropped() {
        let docs = batch();
        let reply = r#"{"findings":[{"conversation":"C1","text":"x","tag":"match","emails":["E1","E3"]}]}"#;
        let found = BatchLabels::new(0, &docs).parse(reply).unwrap();
        assert_eq!(found[0].emails, vec!["id-a".to_string()]);
    }

    #[test]
    fn two_entries_for_one_conversation_are_one_verdict() {
        let docs = batch();
        let reply = r#"{"findings":[
            {"conversation":"C2","text":"asked","tag":"context","emails":["E2"]},
            {"conversation":"C2","text":"quoted","tag":"match","emails":["E3"]}]}"#;
        let found = BatchLabels::new(0, &docs).parse(reply).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tag, FindingTag::Match);
        assert_eq!(found[0].emails, vec!["id-b".to_string(), "id-c".to_string()]);
    }

    #[test]
    fn a_reply_wrapped_in_a_code_fence_still_parses() {
        let docs = batch();
        let reply = "```json\n{\"findings\":[]}\n```";
        assert_eq!(BatchLabels::new(0, &docs).parse(reply).unwrap(), vec![]);
    }

    #[test]
    fn a_reply_that_is_not_json_is_an_error_not_an_empty_batch() {
        let docs = batch();
        assert!(BatchLabels::new(0, &docs).parse("- MATCH: a quote (E1)").is_err());
    }

    fn finding(doc: usize, tag: FindingTag, emails: &[&str]) -> Finding {
        Finding {
            doc,
            tag,
            text: "a quote".into(),
            emails: emails.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_match_citing_only_the_other_side_becomes_background() {
        let docs = batch();
        let findings = vec![
            finding(0, FindingTag::Match, &["id-a"]),
            finding(1, FindingTag::Match, &["id-c"]),
        ];
        let held = enforce_direction(findings.clone(), &docs, Direction::Sent);
        assert_eq!(held[0], findings[0]);
        assert_eq!(held[1].tag, FindingTag::Context);
        assert_eq!(
            held[1].text,
            "a quote [written by Alice <alice@example.com>, not by the user]"
        );
        assert_eq!(enforce_direction(findings.clone(), &docs, Direction::Any), findings);
        let received = enforce_direction(findings, &docs, Direction::Received);
        assert_eq!(
            received[0].tag,
            FindingTag::Context,
            "the user's own email is not received"
        );
        assert_eq!(received[1].tag, FindingTag::Match);
    }

    #[test]
    fn matches_are_the_match_verdicts_in_reading_order_linked_to_their_side() {
        let docs = batch();
        let findings = vec![
            finding(1, FindingTag::Match, &["id-b", "id-c"]),
            finding(0, FindingTag::Context, &["id-a"]),
        ];
        let matches = collect_matches(&docs, &findings, Direction::Sent);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "id-b", "the user's email, not the reply");
        assert_eq!(matches[0].emails, 1);
        assert_eq!(matches[0].subject, "Subject t2");
        let any = collect_matches(&docs, &findings, Direction::Any);
        assert_eq!(any[0].emails, 2);
    }
}
