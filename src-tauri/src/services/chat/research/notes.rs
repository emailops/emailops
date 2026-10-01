//! Notes between reading and the report: what the condense step merges, how
//! the report sees them, and how the report's citations become links.
//!
//! Every note knows, in code, which conversations it covers. The condense step
//! merges notes by label (`N3`) and never writes a conversation or an id, so
//! merging cannot lose or invent a citation. The report cites conversations by
//! number (`[3]`, `[2, 5]`) and code turns each number into a link to that
//! conversation's email, labelled with its subject: the model never writes a
//! link, so it cannot write a broken or duplicated one.

use std::collections::HashMap;

use crate::ai::json_shape::JsonShape;

use super::prompts::ResearchDoc;
use super::reading::{Finding, FindingTag, Match};

/// Longest merged note, in characters.
const MAX_NOTE_CHARS: usize = 300;
/// Most notes one condense call returns.
pub(crate) const MAX_CONDENSED_NOTES: usize = 8;

/// A note the report (or a condense step) reads: a finding, or several merged.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Note {
    /// The conversations it covers, as indices in reading order.
    pub docs: Vec<usize>,
    pub tag: FindingTag,
    pub text: String,
}

impl From<&Finding> for Note {
    fn from(f: &Finding) -> Self {
        Note {
            docs: vec![f.doc],
            tag: f.tag,
            text: f.text.clone(),
        }
    }
}

fn tag_word(tag: FindingTag) -> &'static str {
    match tag {
        FindingTag::Match => "MATCH",
        FindingTag::Context => "CONTEXT",
    }
}

// ── Condense ────────────────────────────────────────────────────────────────

/// A group of notes as the condense step sees them, labelled `N1…`.
pub(crate) fn render_condense_input(notes: &[Note]) -> String {
    notes
        .iter()
        .enumerate()
        .map(|(i, n)| format!("N{} ({}): {}\n", i + 1, tag_word(n.tag), n.text))
        .collect()
}

/// The shape of a condense reply: merged notes, each naming the notes it
/// merges by label.
pub(crate) fn condense_shape(group_len: usize) -> JsonShape {
    let labels: Vec<String> = (1..=group_len).map(|i| format!("N{i}")).collect();
    let note = JsonShape::object(vec![
        (
            "text",
            JsonShape::String {
                max_len: MAX_NOTE_CHARS,
            },
        ),
        (
            "from",
            JsonShape::array(JsonShape::one_of(&labels), 1, group_len.max(1)),
        ),
    ]);
    JsonShape::object(vec![("notes", JsonShape::array(note, 1, MAX_CONDENSED_NOTES))])
}

/// The merged notes in a condense reply. Each covers every conversation of the
/// notes it names, and is a match only when all of those are — a merge that
/// mixed an answer with background reads as background. An input note the
/// reply never names is kept as it was, after the merged ones: a model that
/// forgets a label must not drop that conversation from the report. `Err` when
/// the reply is not the expected JSON.
pub(crate) fn parse_condensed(reply: &str, group: &[Note]) -> Result<Vec<Note>, String> {
    let value: serde_json::Value = serde_json::from_str(reply.trim())
        .or_else(|e| {
            let t = reply.trim();
            match (t.find('{'), t.rfind('}')) {
                (Some(s), Some(end)) if s < end => serde_json::from_str(&t[s..=end]).map_err(|e| e.to_string()),
                _ => Err(e.to_string()),
            }
        })
        .map_err(|e| format!("the condense reply is not JSON: {e}"))?;
    let entries = value
        .get("notes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| "the condense reply has no \"notes\" list".to_string())?;
    let mut out = Vec::new();
    let mut named = vec![false; group.len()];
    for entry in entries {
        let indices: Vec<usize> = entry
            .get("from")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str()?.strip_prefix('N')?.parse::<usize>().ok()?.checked_sub(1))
            .filter(|i| *i < group.len())
            .collect();
        for &i in &indices {
            named[i] = true;
        }
        let sources: Vec<&Note> = indices.iter().filter_map(|&i| group.get(i)).collect();
        if sources.is_empty() {
            continue; // a note that merges nothing cites nothing
        }
        let mut docs: Vec<usize> = sources.iter().flat_map(|n| n.docs.iter().copied()).collect();
        docs.sort_unstable();
        docs.dedup();
        let tag = if sources.iter().all(|n| n.tag == FindingTag::Match) {
            FindingTag::Match
        } else {
            FindingTag::Context
        };
        let text = entry
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(Note { docs, tag, text });
    }
    let left_out: Vec<Note> = group
        .iter()
        .zip(&named)
        .filter(|(_, named)| !**named)
        .map(|(n, _)| n.clone())
        .collect();
    if !left_out.is_empty() {
        super::super::emit_log(
            "debug",
            &format!(
                "research: the condense reply left {} note(s) out — kept as they were",
                left_out.len()
            ),
        );
        out.extend(left_out);
    }
    Ok(out)
}

// ── The report's view ───────────────────────────────────────────────────────

/// The conversations the notes cover, numbered 1… in reading order: the
/// numbers the report cites.
pub(crate) fn number_conversations(notes: &[Vec<Note>]) -> Vec<usize> {
    let mut docs: Vec<usize> = notes.iter().flatten().flat_map(|n| n.docs.iter().copied()).collect();
    docs.sort_unstable();
    docs.dedup();
    docs
}

/// One note as the report reads it: `- MATCH [1][4]: text`.
fn note_line(note: &Note, numbers: &HashMap<usize, usize>) -> String {
    let refs: String = note
        .docs
        .iter()
        .filter_map(|d| numbers.get(d))
        .map(|n| format!("[{n}]"))
        .collect();
    format!("- {} {refs}: {}", tag_word(note.tag), note.text)
}

/// The notes block of the report prompt: a legend of the numbered
/// conversations, then every note, trimmed to `max_chars`.
///
/// When the notes overflow, each batch keeps an equal share of its leading
/// notes instead of the last batches being cut off entirely — the tail of the
/// candidate list is still part of what the user asked to have read.
pub(crate) fn render_notes(notes: &[Vec<Note>], order: &[usize], docs: &[ResearchDoc], max_chars: usize) -> String {
    let numbers: HashMap<usize, usize> = order.iter().enumerate().map(|(i, d)| (*d, i + 1)).collect();
    let mut legend = String::from("CONVERSATIONS:\n");
    for (i, d) in order.iter().enumerate() {
        if let Some(doc) = docs.get(*d) {
            let date = doc.messages.first().map(|m| m.date.as_str()).unwrap_or("");
            legend.push_str(&format!("[{}] {} · {date}\n", i + 1, doc.subject));
        }
    }
    let lines: Vec<Vec<String>> = notes
        .iter()
        .map(|batch| batch.iter().map(|n| note_line(n, &numbers)).collect())
        .collect();
    let budget = max_chars.saturating_sub(legend.chars().count());
    let total: usize = lines.iter().flatten().map(|l| l.chars().count() + 1).sum();
    let non_empty = lines.iter().filter(|b| !b.is_empty()).count().max(1);
    let share = if total <= budget {
        usize::MAX
    } else {
        budget / non_empty
    };
    let mut out = legend;
    out.push_str("\nNOTES:\n");
    for batch in &lines {
        let mut used = 0;
        for line in batch {
            let len = line.chars().count() + 1;
            if used + len > share {
                break;
            }
            used += len;
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// A batch's notes as the prompt will see them, in chars (for condense
/// grouping).
pub(crate) fn notes_len(batch: &[Note]) -> usize {
    batch.iter().map(|n| n.text.chars().count() + 24).sum()
}

// ── Citations → links ───────────────────────────────────────────────────────

/// Longest link label built from a subject.
const MAX_LABEL_CHARS: usize = 60;

/// A subject as a Markdown link label: no brackets (they would end the label),
/// whitespace collapsed, cut to [`MAX_LABEL_CHARS`].
pub(crate) fn link_label(subject: &str) -> String {
    let cleaned: String = subject.replace(['[', ']'], "");
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.is_empty() {
        return "email".to_string();
    }
    let joined = words.join(" ");
    if joined.chars().count() <= MAX_LABEL_CHARS {
        joined
    } else {
        let cut: String = joined.chars().take(MAX_LABEL_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// What each conversation number links to: its subject as the label, and its
/// email — the match's representative when the conversation matched, else the
/// first email a note on it cited, else its first email.
pub(crate) fn citation_targets(
    order: &[usize],
    docs: &[ResearchDoc],
    matches: &[Match],
    findings: &[Finding],
) -> Vec<(String, String)> {
    order
        .iter()
        .filter_map(|d| {
            let doc = docs.get(*d)?;
            let id = matches
                .iter()
                .find(|m| m.thread_id == doc.thread_id)
                .map(|m| m.id.clone())
                .or_else(|| {
                    findings
                        .iter()
                        .find(|f| f.doc == *d)
                        .and_then(|f| f.emails.first().cloned())
                })
                .or_else(|| doc.messages.first().map(|m| m.id.clone()))?;
            Some((link_label(&doc.subject), id))
        })
        .collect()
}

/// The report with every citation — `[3]`, `[2, 5]` — turned into links to
/// the conversations it names. A number no conversation has is dropped from
/// its citation; brackets holding no conversation number at all (`[2024]`)
/// are text and stay as written. A citation right after another to the same
/// conversation is not repeated. Markdown links (`[text](…)`) are left alone.
/// Pure.
pub(crate) fn render_citations(report: &str, targets: &[(String, String)]) -> String {
    use std::sync::OnceLock;
    static CITE_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let re = CITE_RE.get_or_init(|| regex::Regex::new(r"\[\s*\d+(?:\s*[,;]\s*\d+)*\s*\]").unwrap());
    let mut out = String::with_capacity(report.len());
    let mut last = 0;
    // The conversations the citation just before this one linked, and where
    // it ended.
    let mut previous: (usize, Vec<usize>) = (usize::MAX, Vec::new());
    for m in re.find_iter(report) {
        if report[m.end()..].starts_with('(') {
            continue; // a Markdown link's label, not a citation
        }
        let adjacent = report[previous.0.min(m.start())..m.start()].trim().is_empty() && previous.0 <= m.start();
        let valid = |n: &str| {
            n.trim()
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=targets.len()).contains(n))
        };
        if !m
            .as_str()
            .trim_matches(['[', ']'])
            .split([',', ';'])
            .any(|n| valid(n).is_some())
        {
            continue; // not a citation: a year, a count, a note in brackets
        }
        let mut numbers: Vec<usize> = Vec::new();
        for n in m.as_str().trim_matches(['[', ']']).split([',', ';']) {
            let Some(n) = valid(n) else {
                continue;
            };
            if !numbers.contains(&n) && !(adjacent && previous.1.contains(&n)) {
                numbers.push(n);
            }
        }
        let links: Vec<String> = numbers
            .iter()
            .map(|n| {
                let (label, id) = &targets[n - 1];
                format!("[{label}](email://{id})")
            })
            .collect();
        let gap = &report[last..m.start()];
        // A citation that renders to nothing takes the space before it along.
        out.push_str(if links.is_empty() {
            gap.trim_end_matches(' ')
        } else {
            gap
        });
        out.push_str(&links.join(" "));
        last = m.end();
        previous = (
            m.end(),
            if adjacent {
                [previous.1, numbers].concat()
            } else {
                numbers
            },
        );
    }
    out.push_str(&report[last..]);
    out
}

// ── Closing the report ──────────────────────────────────────────────────────

/// A report that ran out of output budget ends mid-line and loses every match
/// it had not reached: drop the broken last line and end with the list of all
/// matches, built in code. A report that links none of its sources gets the
/// list too, so every match can be opened. A complete, linked report is left
/// as written. Pure.
pub(crate) fn finish_report(report: &str, cut: bool, matches: &[Match], language_code: &str) -> String {
    let list = render_match_list(matches, language_code);
    if !cut {
        return if report.contains("](email://") || list.is_empty() {
            report.to_string()
        } else {
            format!("{report}\n\n{list}")
        };
    }
    let kept = report.rsplit_once('\n').map_or("", |(head, _)| head).trim_end();
    match (kept.is_empty(), list.is_empty()) {
        (_, true) => kept.to_string(),
        (true, false) => list,
        (false, false) => format!("{kept}\n\n{list}"),
    }
}

/// The complete numbered list of matches, oldest first, in the report's
/// language. (Reading order follows each conversation's first gathered email;
/// the list follows the email it links.)
pub(crate) fn render_match_list(matches: &[Match], language_code: &str) -> String {
    if matches.is_empty() {
        return String::new();
    }
    let mut matches: Vec<&Match> = matches.iter().collect();
    // ISO dates sort as text; the sort is stable, so ties keep reading order.
    matches.sort_by(|a, b| a.date.cmp(&b.date));
    let (heading, emails_word) = match language_code {
        "es" => ("Lista completa", "correos"),
        "fr" => ("Liste complète", "e-mails"),
        "de" => ("Vollständige Liste", "E-Mails"),
        _ => ("Full list", "emails"),
    };
    let mut out = format!("### {heading} ({})\n\n", matches.len());
    for (i, m) in matches.iter().enumerate() {
        let date = if m.date.is_empty() {
            String::new()
        } else {
            format!("{} · ", m.date)
        };
        let finding = if m.finding.is_empty() {
            String::new()
        } else {
            format!(" — {}", m.finding)
        };
        let thread = if m.emails > 1 {
            format!(" ({} {emails_word})", m.emails)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{}. {date}[{}](email://{}){finding}{thread}\n",
            i + 1,
            link_label(&m.subject),
            m.id
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(docs: &[usize], tag: FindingTag, text: &str) -> Note {
        Note {
            docs: docs.to_vec(),
            tag,
            text: text.into(),
        }
    }

    fn targets() -> Vec<(String, String)> {
        vec![
            ("Quote A".into(), "id-a".into()),
            ("Quote B".into(), "id-b".into()),
            ("Quote C".into(), "id-c".into()),
        ]
    }

    #[test]
    fn a_citation_becomes_a_link_to_its_conversation() {
        assert_eq!(
            render_citations("You sent a quote [1] and another [2, 3].", &targets()),
            "You sent a quote [Quote A](email://id-a) and another [Quote B](email://id-b) [Quote C](email://id-c)."
        );
    }

    #[test]
    fn a_number_no_conversation_has_is_dropped_but_other_brackets_stay() {
        assert_eq!(render_citations("Filed in [2024].", &targets()), "Filed in [2024].");
        assert_eq!(
            render_citations("A quote [1, 9].", &targets()),
            "A quote [Quote A](email://id-a)."
        );
    }

    #[test]
    fn a_citation_repeated_right_after_itself_is_not_repeated() {
        assert_eq!(
            render_citations("A quote [1][1] and [1, 2][2].", &targets()),
            "A quote [Quote A](email://id-a) and [Quote A](email://id-a) [Quote B](email://id-b)."
        );
    }

    #[test]
    fn markdown_links_are_not_citations() {
        let text = "See [1](https://example.com) and [2].";
        assert_eq!(
            render_citations(text, &targets()),
            "See [1](https://example.com) and [Quote B](email://id-b)."
        );
    }

    #[test]
    fn merged_notes_cover_every_conversation_of_what_they_merge() {
        let group = vec![
            note(&[3], FindingTag::Match, "quote to Acme"),
            note(&[7], FindingTag::Match, "quote to Beta"),
            note(&[9], FindingTag::Context, "asked a supplier"),
        ];
        let reply = r#"{"notes":[
            {"text":"Quotes to Acme and Beta","from":["N1","N2"]},
            {"text":"Mixed","from":["N2","N3","N7"]}]}"#;
        let merged = parse_condensed(reply, &group).unwrap();
        assert_eq!(merged[0], note(&[3, 7], FindingTag::Match, "Quotes to Acme and Beta"));
        assert_eq!(
            merged[1],
            note(&[7, 9], FindingTag::Context, "Mixed"),
            "background once mixed; unknown labels dropped"
        );
    }

    #[test]
    fn a_note_the_condense_reply_left_out_is_kept_as_it_was() {
        // The reply merged N1 and N3 but never named N2: dropping it lost that
        // conversation and its citation from the report.
        let group = vec![
            note(&[3], FindingTag::Match, "quote to Acme"),
            note(&[7], FindingTag::Match, "quote to Beta"),
            note(&[9], FindingTag::Match, "quote to Gamma"),
        ];
        let reply = r#"{"notes":[{"text":"Quotes to Acme and Gamma","from":["N1","N3"]}]}"#;
        let merged = parse_condensed(reply, &group).unwrap();
        assert_eq!(
            merged,
            vec![
                note(&[3, 9], FindingTag::Match, "Quotes to Acme and Gamma"),
                note(&[7], FindingTag::Match, "quote to Beta"),
            ]
        );
    }

    #[test]
    fn a_condense_reply_that_is_not_json_is_an_error() {
        assert!(parse_condensed("- merged notes", &[]).is_err());
    }

    #[test]
    fn the_condense_shape_names_only_the_groups_labels() {
        let schema = condense_shape(3).to_json_schema();
        let note = &schema["properties"]["notes"]["items"];
        assert_eq!(
            note["properties"]["from"]["items"]["enum"],
            serde_json::json!(["N1", "N2", "N3"])
        );
        assert_eq!(schema["properties"]["notes"]["maxItems"], MAX_CONDENSED_NOTES);
    }

    fn docs(n: usize) -> Vec<ResearchDoc> {
        (0..n)
            .map(|i| ResearchDoc {
                thread_id: format!("t{i}"),
                subject: format!("Subject {i}"),
                messages: vec![super::super::prompts::DocMessage {
                    id: format!("id-{i}"),
                    date: "2026-09-01".into(),
                    from: "a".into(),
                    to: String::new(),
                    from_user: false,
                    text: "x".into(),
                }],
            })
            .collect()
    }

    #[test]
    fn the_report_sees_numbered_conversations_and_tagged_notes() {
        let notes = vec![
            vec![note(&[4], FindingTag::Match, "quote to Acme")],
            vec![note(&[2, 4], FindingTag::Context, "asked a supplier")],
        ];
        let order = number_conversations(&notes);
        assert_eq!(order, vec![2, 4]);
        let block = render_notes(&notes, &order, &docs(5), 10_000);
        assert!(
            block.starts_with("CONVERSATIONS:\n[1] Subject 2 · 2026-09-01\n[2] Subject 4 · 2026-09-01\n"),
            "{block}"
        );
        assert!(block.contains("- MATCH [2]: quote to Acme\n"), "{block}");
        assert!(block.contains("- CONTEXT [1][2]: asked a supplier\n"), "{block}");
    }

    #[test]
    fn overflowing_notes_keep_a_share_of_every_batch() {
        let long = "x".repeat(200);
        let notes: Vec<Vec<Note>> = (0..4)
            .map(|b| (0..5).map(|i| note(&[b * 5 + i], FindingTag::Match, &long)).collect())
            .collect();
        let order = number_conversations(&notes);
        let block = render_notes(&notes, &order, &docs(20), 3_000);
        for b in 0..4 {
            let first_of_batch = format!("[{}]:", b * 5 + 1);
            assert!(
                block.contains(&first_of_batch),
                "batch {b} kept its first note: {block}"
            );
        }
    }

    fn a_match(id: &str) -> Match {
        Match {
            id: id.into(),
            emails: 1,
            thread_id: "t".into(),
            date: "2024-01-02".into(),
            subject: format!("Subject {id}"),
            finding: "quote".into(),
        }
    }

    #[test]
    fn the_full_list_is_in_date_order() {
        let mut late = a_match("late");
        late.date = "2024-02-20".into();
        let mut early = a_match("early");
        early.date = "2024-02-13".into();
        let list = render_match_list(&[late, early], "en");
        assert!(
            list.find("email://early").unwrap() < list.find("email://late").unwrap(),
            "{list}"
        );
    }

    #[test]
    fn a_cut_report_drops_its_broken_line_and_ends_with_every_match() {
        let matches = vec![a_match("e1"), a_match("e9")];
        let cut = "Summary.\n*   **A:** sent.\n    *   You sent the qu";
        assert_eq!(
            finish_report(cut, true, &matches, "en"),
            format!("Summary.\n*   **A:** sent.\n\n{}", render_match_list(&matches, "en"))
        );
    }

    #[test]
    fn a_report_that_links_nothing_ends_with_every_match() {
        let matches = vec![a_match("e1")];
        assert_eq!(
            finish_report("You sent one quote.", false, &matches, "en"),
            format!("You sent one quote.\n\n{}", render_match_list(&matches, "en"))
        );
        let linked = "You sent [the quote](email://e1).";
        assert_eq!(finish_report(linked, false, &matches, "en"), linked);
    }

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
    fn the_full_list_numbers_every_match_with_its_link() {
        let matches = vec![
            m("e1", "t1", "2026-01-02", "Alice asks for a quote"),
            m("e3", "t2", "2026-01-05", "Bob [asks] too"),
        ];
        let list = render_match_list(&matches, "es");
        assert!(list.starts_with("### Lista completa (2)\n\n"), "{list}");
        assert!(
            list.contains("1. 2026-01-02 · [Subject e1](email://e1) — Alice asks for a quote\n"),
            "{list}"
        );
        assert!(
            list.contains("2. 2026-01-05 · [Subject e3](email://e3) — Bob [asks] too\n"),
            "{list}"
        );
        assert!(render_match_list(&matches, "en").starts_with("### Full list (2)"));
        assert!(render_match_list(&[], "en").is_empty());
    }

    #[test]
    fn the_full_list_says_how_many_emails_a_conversation_holds() {
        let matches = vec![Match {
            emails: 3,
            ..m("e1", "t1", "2026-01-02", "Budget requested")
        }];
        let list = render_match_list(&matches, "es");
        assert!(list.contains("— Budget requested (3 correos)"), "{list}");
        assert!(render_match_list(&matches, "en").contains("(3 emails)"));
    }
    // ── condense input and reply ──

    #[test]
    fn the_condense_step_sees_each_note_labelled_with_its_tag() {
        let group = vec![
            note(&[0], FindingTag::Match, "quote to Acme"),
            note(&[1], FindingTag::Context, "asked a supplier"),
        ];
        assert_eq!(
            render_condense_input(&group),
            "N1 (MATCH): quote to Acme\nN2 (CONTEXT): asked a supplier\n"
        );
    }

    #[test]
    fn a_condense_reply_wrapped_in_prose_is_still_read() {
        let group = vec![note(&[0], FindingTag::Match, "quote")];
        let reply = r#"Here are the notes: {"notes":[{"text":"One quote","from":["N1"]}]} Done."#;
        let merged = parse_condensed(reply, &group).unwrap();
        assert_eq!(merged, vec![note(&[0], FindingTag::Match, "One quote")]);
    }

    #[test]
    fn a_reply_with_its_braces_the_wrong_way_round_is_an_error_not_a_panic() {
        let group = vec![note(&[0], FindingTag::Match, "quote")];
        assert!(parse_condensed("} no notes here {", &group).is_err());
    }

    #[test]
    fn a_label_past_the_group_is_ignored_and_the_note_kept() {
        let group = vec![note(&[0], FindingTag::Match, "a"), note(&[1], FindingTag::Match, "b")];
        let merged = parse_condensed(r#"{"notes":[{"text":"x","from":["N1","N3"]}]}"#, &group).unwrap();
        assert_eq!(
            merged,
            vec![note(&[0], FindingTag::Match, "x"), note(&[1], FindingTag::Match, "b")]
        );
    }

    #[test]
    fn a_batch_is_measured_by_its_note_text_plus_the_line_overhead() {
        assert_eq!(notes_len(&[note(&[0], FindingTag::Match, "abc")]), 3 + 24);
    }

    // ── the report's notes block ──

    /// Two batches of three 14-char lines (15 with the newline) around an
    /// empty one; no conversations, so the legend is just its heading.
    fn three_by_two() -> Vec<Vec<Note>> {
        let batch = || vec![note(&[], FindingTag::Match, "aaaa"); 3];
        vec![batch(), vec![], batch()]
    }

    const LEGEND: &str = "CONVERSATIONS:\n";

    fn rendered_lines(max_notes_chars: usize) -> usize {
        let out = render_notes(&three_by_two(), &[], &[], LEGEND.len() + max_notes_chars);
        out.lines().filter(|l| l.starts_with("- MATCH")).count()
    }

    #[test]
    fn notes_that_fit_are_all_kept() {
        assert_eq!(rendered_lines(90), 6);
    }

    #[test]
    fn overflowing_notes_give_each_non_empty_batch_an_equal_share() {
        // 90 chars of notes in 85: each of the two batches gets 42, room for 2 lines.
        assert_eq!(rendered_lines(85), 4);
        // A share of exactly two lines keeps two lines.
        assert_eq!(rendered_lines(60), 4);
        // A share under one line keeps none.
        assert_eq!(rendered_lines(29), 0);
    }

    // ── labels and link targets ──

    #[test]
    fn a_long_subject_is_cut_to_sixty_chars_with_an_ellipsis() {
        let label = link_label(&"abcdefghij".repeat(8));
        assert_eq!(label.chars().count(), MAX_LABEL_CHARS);
        assert!(label.ends_with('…'), "{label}");
    }

    fn doc(thread_id: &str, first_email: &str) -> ResearchDoc {
        ResearchDoc {
            thread_id: thread_id.into(),
            subject: format!("Subject {thread_id}"),
            messages: vec![super::super::prompts::DocMessage {
                id: first_email.into(),
                date: "2024-01-02".into(),
                from: "sender@example.com".into(),
                to: String::new(),
                from_user: false,
                text: String::new(),
            }],
        }
    }

    fn finding(doc: usize, email: &str) -> Finding {
        Finding {
            doc,
            tag: FindingTag::Context,
            text: "context".into(),
            emails: vec![email.into()],
        }
    }

    #[test]
    fn an_unmatched_conversation_links_the_email_its_own_note_cited() {
        let docs = vec![doc("t0", "t0-first"), doc("t1", "t1-first")];
        let findings = vec![finding(0, "t0-cited"), finding(1, "t1-cited")];
        let targets = citation_targets(&[0, 1], &docs, &[], &findings);
        assert_eq!(targets[0].1, "t0-cited");
        assert_eq!(targets[1].1, "t1-cited");
    }

    #[test]
    fn the_full_list_heading_is_in_the_report_language() {
        let matches = vec![a_match("a")];
        assert!(render_match_list(&matches, "fr").starts_with("### Liste complète (1)"));
        assert!(render_match_list(&matches, "de").starts_with("### Vollständige Liste (1)"));
    }
}
