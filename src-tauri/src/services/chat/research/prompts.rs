//! Research-mode prompts: rendering the map / condense / reduce prompts split
//! for `complete_with_prefix`, parsing the findings a batch returns, and
//! turning the report's bare references into email links.

use std::collections::HashMap;

use super::plan::Direction;

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
    pub(crate) fn render(&self) -> String {
        let mut out = format!("CONVERSATION: {}\n", self.subject);
        for m in &self.messages {
            let to = if m.to.is_empty() {
                String::new()
            } else {
                format!("To: {}\n", m.to)
            };
            out.push_str(&format!(
                "EMAIL_ID: {}\nDate: {}\nFrom: {}\n{to}{}\n\n",
                m.id, m.date, m.from, m.text
            ));
        }
        out
    }

    pub(crate) fn rendered_len(&self) -> usize {
        self.render().chars().count()
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
pub(crate) fn split_map_prompt(
    template: &str,
    question: &str,
    docs: &[ResearchDoc],
    direction: Direction,
) -> (String, String) {
    let emails = docs.iter().map(ResearchDoc::render).collect::<Vec<_>>().join("\n");
    let mut vars = HashMap::new();
    vars.insert("question", question.to_string());
    vars.insert("direction", direction_note(direction).to_string());
    vars.insert("emails", emails);
    split_at_marker(template, MAP_MARKER, &vars)
}

/// What one batch yielded: the finding lines, and the emails they cite.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BatchNotes {
    pub lines: Vec<String>,
    pub cited: Vec<String>,
}

/// Keep the findings of a map reply that cite an email of the batch.
///
/// A finding with no citation to a batch email is dropped rather than trusted:
/// it is either chatter ("Here are the findings:"), a "nothing relevant" in
/// some wording, or a claim the reduce could not link — and an unlinked claim
/// in the final report is exactly what research mode must not produce.
pub(crate) fn parse_map_notes(reply: &str, batch_ids: &[String]) -> BatchNotes {
    let mut notes = BatchNotes::default();
    for raw in reply.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let cited: Vec<&String> = batch_ids.iter().filter(|id| line.contains(id.as_str())).collect();
        if cited.is_empty() {
            continue;
        }
        let body = line.trim_start_matches(['-', '*', '•', ' ']).trim();
        notes.lines.push(format!("- {body}"));
        for id in cited {
            if !notes.cited.contains(id) {
                notes.cited.push(id.clone());
            }
        }
    }
    notes
}

/// Holds a batch's findings to who actually wrote each email. On a question
/// with a direction, a MATCH citing only emails from the other side is a
/// misreading (the supplier's quote read as the user's own): it becomes
/// CONTEXT, with who wrote it, so the report cannot state it as an answer.
/// The list and the count already exclude it (see [`collect_matches`]); this
/// keeps the prose in step. Pure.
pub(crate) fn enforce_direction(notes: BatchNotes, docs: &[ResearchDoc], direction: Direction) -> BatchNotes {
    if direction == Direction::Any {
        return notes;
    }
    let by_id: HashMap<&str, &DocMessage> = docs
        .iter()
        .flat_map(|d| d.messages.iter())
        .map(|m| (m.id.as_str(), m))
        .collect();
    let lines = notes
        .lines
        .into_iter()
        .map(|line| {
            let (tag, rest) = finding_tag(&line);
            let cited: Vec<&DocMessage> = by_id
                .iter()
                .filter(|(id, _)| line.contains(*id))
                .map(|(_, m)| *m)
                .collect();
            if tag == FindingTag::Context || cited.is_empty() {
                return line;
            }
            let wrong_side = match direction {
                Direction::Sent => cited.iter().all(|m| !m.from_user),
                Direction::Received => cited.iter().all(|m| m.from_user),
                Direction::Any => false,
            };
            if !wrong_side {
                return line;
            }
            let writer = if direction == Direction::Sent {
                format!("written by {}, not by the user", cited[0].from)
            } else {
                "written by the user".to_string()
            };
            format!("- CONTEXT: {rest} [{writer}]")
        })
        .collect();
    BatchNotes { lines, ..notes }
}

/// Join every batch's findings into the notes block, trimmed to `max_chars`.
///
/// When the notes overflow, each batch keeps an equal share of its leading
/// lines (a model lists its strongest findings first) instead of the last
/// batches being cut off entirely — the tail of the candidate list is still
/// part of what the user asked to have read.
pub(crate) fn assemble_notes(batches: &[BatchNotes], max_chars: usize) -> String {
    let total: usize = batches
        .iter()
        .flat_map(|b| b.lines.iter())
        .map(|l| l.chars().count() + 1)
        .sum();
    let non_empty = batches.iter().filter(|b| !b.lines.is_empty()).count().max(1);
    let share = if total <= max_chars {
        usize::MAX
    } else {
        max_chars / non_empty
    };
    let mut out = String::new();
    for batch in batches {
        let mut used = 0;
        for line in &batch.lines {
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

// ── Report post-processing ──────────────────────────────────────────────────

/// Longest link label built from a subject.
const MAX_LABEL_CHARS: usize = 60;

/// A subject as a Markdown link label: no brackets (they would end the label),
/// whitespace collapsed, cut to [`MAX_LABEL_CHARS`].
fn link_label(subject: &str) -> String {
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

/// Turn the bare references a report makes into real email links.
///
/// The notes cite as `(email://ID)` and a small model copies that shape (or
/// writes `[email://ID]`) instead of `[label](email://ID)`, which the chat only
/// renders as a clickable chip in the link form. Each bare reference to an
/// email that was read becomes a link labelled with its subject, and so does a
/// link whose label is its own id (`[email://ID](email://ID)`); proper links
/// and ids that were never read are left untouched (the link allowlist drops
/// the latter downstream).
pub(crate) fn relink_bare_refs(answer: &str, subjects: &HashMap<String, String>) -> String {
    use std::sync::OnceLock;
    static BARE_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let re = BARE_RE.get_or_init(|| {
        regex::Regex::new(r"\[email://([^\]\s]+)\](?:\(email://([^)\s]+)\))?|\(email://([^)\s]+)\)").unwrap()
    });
    let mut out = String::with_capacity(answer.len());
    let mut last = 0;
    for caps in re.captures_iter(answer) {
        let Some(whole) = caps.get(0) else { continue };
        // An id-labelled link's own target wins over its label.
        let Some(id) = caps
            .get(2)
            .or_else(|| caps.get(1))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str())
        else {
            continue;
        };
        // `(email://ID)` right after `]` is already the target of a link.
        let is_link_target = caps.get(3).is_some() && answer[..whole.start()].ends_with(']');
        let Some(subject) = subjects.get(id).filter(|_| !is_link_target) else {
            continue;
        };
        out.push_str(&answer[last..whole.start()]);
        out.push_str(&format!("[{}](email://{id})", link_label(subject)));
        last = whole.end();
    }
    out.push_str(&answer[last..]);
    out
}

// ── Matches, exact counts, full list ─────────────────────────────────────────

/// What the report must carry besides the model's prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportShape {
    /// "List every…", "how many…": the report ends with the complete,
    /// numbered list of matches, built in code — a model-written list stops at
    /// its output budget (a few dozen lines) and a model's count is a guess.
    FullList,
    /// Themes, trends, summaries: the prose is the answer.
    Analysis,
}

/// Words that ask for an enumeration or a count, EN/ES/FR/DE. Matched as
/// whole words or phrases on the lowercased question.
const LIST_CUES: &[&str] = &[
    // EN
    "list",
    "all the",
    "every",
    "each",
    "how many",
    "number of",
    "count",
    "enumerate",
    "table",
    // ES
    "lista",
    "listado",
    "todas",
    "todos",
    "cada",
    "cuántos",
    "cuántas",
    "cuantos",
    "cuantas",
    "número de",
    "numero de",
    "enumera",
    "tabla",
    // FR
    "liste",
    "toutes",
    "tous",
    "chaque",
    "combien",
    "nombre de",
    // DE
    "liste",
    "alle",
    "jede",
    "jeder",
    "wie viele",
    "anzahl",
    "tabelle",
];

/// Whether the question wants the full list of matches appended. Pure.
pub(crate) fn plan_report_shape(question: &str) -> ReportShape {
    let q = format!(" {} ", question.to_lowercase());
    let is_word_char = |c: char| c.is_alphanumeric();
    let hit = LIST_CUES.iter().any(|cue| {
        q.match_indices(cue).any(|(i, _)| {
            let before = q[..i].chars().next_back().is_none_or(|c| !is_word_char(c));
            let after = q[i + cue.len()..].chars().next().is_none_or(|c| !is_word_char(c));
            before && after
        })
    });
    if hit {
        ReportShape::FullList
    } else {
        ReportShape::Analysis
    }
}

/// One conversation the reading step found relevant: its first matching email
/// (oldest), that email's first finding, and how many of the conversation's
/// emails were cited. A thread of replies quoting the same request is one
/// match, not one per reply.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Match {
    pub id: String,
    /// Emails of this conversation a finding cites.
    pub emails: usize,
    pub thread_id: String,
    pub date: String,
    pub subject: String,
    pub finding: String,
}

/// Whether a finding answers the question or only gives background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingTag {
    /// The cited email answers the question itself: it is a match.
    Match,
    /// Related, but not an answer (a request, a reply, the other side's mail):
    /// background for the report, never a match. An untagged line — a
    /// user-edited map prompt without the tags — counts as a match.
    Context,
}

/// A finding's tag and the text after its bullet and tag. Reads the shapes a
/// small model writes: `CONTEXT:`, `**CONTEXT:**`, `[Context]`, any case.
pub(crate) fn finding_tag(line: &str) -> (FindingTag, &str) {
    let body = line.trim_start_matches(['-', '*', '•', ' ']);
    let bare = body.trim_start_matches(['*', '[', ' ']);
    for (word, tag) in [("context", FindingTag::Context), ("match", FindingTag::Match)] {
        let Some(head) = bare.get(..word.len()) else { continue };
        let after = &bare[word.len()..];
        // A tag, not a word that starts the sentence ("Matched…", "Match fees…").
        if head.eq_ignore_ascii_case(word) && after.starts_with([':', '*', ']']) {
            return (tag, after.trim_start_matches(['*', ']', ':', ' ']));
        }
    }
    (FindingTag::Match, body.trim())
}

/// A finding line without its bullet, its tag and its `email://` references.
fn finding_text(line: &str) -> String {
    use std::sync::OnceLock;
    static REF_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let re = REF_RE.get_or_init(|| regex::Regex::new(r"\[[^\]]*\]\(email://[^)\s]+\)|\(email://[^)\s]+\)").unwrap());
    let stripped = re.replace_all(finding_tag(line).1, "");
    stripped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([',', ';', ':'])
        .to_string()
}

/// The first `MATCH` finding citing each email; `CONTEXT` lines never make one.
fn first_findings(notes: &[BatchNotes]) -> HashMap<&str, String> {
    let mut first: HashMap<&str, String> = HashMap::new();
    for batch in notes {
        for line in batch.lines.iter().filter(|l| finding_tag(l).0 == FindingTag::Match) {
            for id in &batch.cited {
                if line.contains(id.as_str()) {
                    first.entry(id.as_str()).or_insert_with(|| finding_text(line));
                }
            }
        }
    }
    first
}

/// Every conversation a finding cites, in reading order (oldest first), each
/// with its first cited email and that email's first finding. Taken from the
/// map notes, before any condense round, so merging notes for the report never
/// drops a match from the list or the count.
///
/// A question with a direction counts a conversation only when a cited email
/// is on that side: for "what have I sent", one the user wrote. A thread where
/// the user asked for a quote and only the supplier's reply was cited is a
/// quote received — the code knows who wrote each email, the model need not.
pub(crate) fn collect_matches(docs: &[ResearchDoc], notes: &[BatchNotes], direction: Direction) -> Vec<Match> {
    let first = first_findings(notes);
    docs.iter()
        .filter_map(|d| {
            let cited: Vec<&DocMessage> = d
                .messages
                .iter()
                .filter(|m| first.contains_key(m.id.as_str()))
                .collect();
            let on_side = match direction {
                Direction::Sent => cited.iter().any(|m| m.from_user),
                Direction::Received => cited.iter().any(|m| !m.from_user),
                Direction::Any => true,
            };
            if !on_side {
                return None;
            }
            let cited: Vec<&DocMessage> = match direction {
                Direction::Sent => cited.into_iter().filter(|m| m.from_user).collect(),
                Direction::Received => cited.into_iter().filter(|m| !m.from_user).collect(),
                Direction::Any => cited,
            };
            let head = cited.first()?;
            Some(Match {
                id: head.id.clone(),
                emails: cited.len(),
                thread_id: d.thread_id.clone(),
                date: head.date.clone(),
                subject: d.subject.clone(),
                finding: first.get(head.id.as_str()).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// Point every link to an email of a matched conversation at that
/// conversation's representative, so the report never cites two emails of one
/// thread, then drop the bullets that became a repeat: a link-only bullet
/// citing a conversation an earlier link-only bullet of the same run already
/// cites. Pure.
pub(crate) fn canonicalize_links(answer: &str, representative: &HashMap<String, String>) -> String {
    let pointed = point_links_at_representatives(answer, representative);
    drop_repeated_link_bullets(&drop_adjacent_repeat_links(&pointed))
}

/// Drops a link that directly follows another link (only spaces between) to
/// the same email: `[A](email://e1)[B](email://e1)` reads as one citation.
fn drop_adjacent_repeat_links(answer: &str) -> String {
    use std::sync::OnceLock;
    static LINK_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let re = LINK_RE.get_or_init(|| regex::Regex::new(r"\[[^\]]*\]\(email://([^)\s]+)\)").unwrap());
    let mut out = String::with_capacity(answer.len());
    let mut last_end = 0;
    let mut prev: Option<(usize, String)> = None;
    for caps in re.captures_iter(answer) {
        let (Some(whole), Some(id)) = (caps.get(0), caps.get(1)) else {
            continue;
        };
        let repeat = prev
            .as_ref()
            .is_some_and(|(end, prev_id)| prev_id == id.as_str() && answer[*end..whole.start()].trim().is_empty());
        if repeat {
            // Keep the previous link; skip the gap and this one.
            last_end = whole.end();
        } else {
            out.push_str(&answer[last_end..whole.end()]);
            last_end = whole.end();
        }
        prev = Some((whole.end(), id.as_str().to_string()));
    }
    out.push_str(&answer[last_end..]);
    out
}

fn point_links_at_representatives(answer: &str, representative: &HashMap<String, String>) -> String {
    use std::sync::OnceLock;
    static LINK_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let re = LINK_RE.get_or_init(|| regex::Regex::new(r"email://([^)\s\]]+)").unwrap());
    re.replace_all(answer, |caps: &regex::Captures| {
        let id = &caps[1];
        format!("email://{}", representative.get(id).map_or(id, String::as_str))
    })
    .into_owned()
}

/// The email a line cites when it is nothing but a bullet with one link.
fn link_only_bullet_target(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix(['*', '-'])?.trim();
    let target = rest.strip_prefix('[')?.split_once("](email://")?.1.strip_suffix(')')?;
    (!target.contains([')', ' '])).then_some(target)
}

/// Drops a link-only bullet whose target an earlier bullet of the same run of
/// link-only bullets already cites. Any other line ends the run.
fn drop_repeated_link_bullets(answer: &str) -> String {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut kept: Vec<&str> = Vec::new();
    for line in answer.split('\n') {
        match link_only_bullet_target(line) {
            Some(target) if !seen.insert(target) => continue,
            Some(_) => {}
            None => seen.clear(),
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// The exact counts the report states — computed, never left to the model.
pub(crate) fn counts_line(matches: &[Match]) -> String {
    let emails: usize = matches.iter().map(|m| m.emails).sum();
    format!(
        "{emails} emails with relevant findings, in {} conversations",
        matches.len()
    )
}

/// The facts line of the report prompt: the exact counts, and — when the full
/// list will be appended — that the model must not try to write it out.
pub(crate) fn report_facts(matches: &[Match], shape: ReportShape) -> String {
    let mut facts = format!(
        "{} (exact — computed from every email read; state these numbers, never count the notes yourself).",
        counts_line(matches)
    );
    if shape == ReportShape::FullList && !matches.is_empty() {
        facts.push_str(&format!(
            " The complete numbered list of all {} matches is appended after your report automatically: do not reproduce it item by item — give the total, then group, summarise and highlight.",
            matches.len()
        ));
    }
    facts
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

/// The complete numbered list of matches, in the report's language.
/// A report that ran out of output budget ends mid-line and loses every match
/// it had not reached: drop the broken last line and end with the list of all
/// matches, built in code. A complete report is left as written. Pure.
pub(crate) fn finish_report(report: &str, cut: bool, matches: &[Match], language_code: &str) -> String {
    if !cut {
        // A report that links none of its sources leaves the user nothing to
        // open: the list, built in code, carries every match as a link.
        let list = render_match_list(matches, language_code);
        return if report.contains("](email://") || list.is_empty() {
            report.to_string()
        } else {
            format!("{report}\n\n{list}")
        };
    }
    let kept = report.rsplit_once('\n').map_or("", |(head, _)| head).trim_end();
    let list = render_match_list(matches, language_code);
    match (kept.is_empty(), list.is_empty()) {
        (_, true) => kept.to_string(),
        (true, false) => list,
        (false, false) => format!("{kept}\n\n{list}"),
    }
}

pub(crate) fn render_match_list(matches: &[Match], language_code: &str) -> String {
    if matches.is_empty() {
        return String::new();
    }
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

/// Split point of the condense prompt — same convention as map and reduce.
const CONDENSE_MARKER: &str = "QUESTION: {{question}}";

/// The condense prompt for one group of notes, split for `complete_with_prefix`.
pub(crate) fn split_condense_prompt(template: &str, question: &str, notes: &str) -> (String, String) {
    let mut vars = HashMap::new();
    vars.insert("question", question.to_string());
    vars.insert("notes", notes.to_string());
    split_at_marker(template, CONDENSE_MARKER, &vars)
}

/// Every batch's notes joined, one line each, untrimmed.
pub(crate) fn join_notes(batches: &[BatchNotes]) -> String {
    batches
        .iter()
        .flat_map(|b| b.lines.iter())
        .fold(String::new(), |mut out, line| {
            out.push_str(line);
            out.push('\n');
            out
        })
}

/// A batch's notes as the prompt will see them, in chars.
pub(crate) fn notes_len(batch: &BatchNotes) -> usize {
    batch.lines.iter().map(|l| l.chars().count() + 1).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── map prompt / notes ──

    /// A one-message conversation.
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
        vec!["gero@x.example".to_string(), "gero@work.example".to_string()]
    }

    #[test]
    fn an_alias_the_user_sends_from_is_the_user_too() {
        assert_eq!(participant("Gero", "Gero@Work.example", &me()), USER_LABEL);
    }

    #[test]
    fn the_user_is_named_as_you() {
        assert_eq!(participant("Gero", "GERO@x.example", &me()), USER_LABEL);
        assert_eq!(participant("Ana", "ana@x.example", &me()), "Ana <ana@x.example>");
        assert_eq!(participant("", "ana@x.example", &me()), "ana@x.example");
        assert_eq!(participant("ana@x.example", "ana@x.example", &[]), "ana@x.example");
    }

    #[test]
    fn recipients_name_the_user_as_you() {
        let to = vec!["ana@x.example".to_string(), "Gero <gero@x.example>".to_string()];
        assert_eq!(recipients(&to, &me()), format!("ana@x.example, {USER_LABEL}"));
        assert_eq!(recipients(&[], &me()), "");
    }

    #[test]
    fn a_message_renders_who_sent_it_to_whom() {
        let mut d = doc("e1");
        d.messages[0].from = USER_LABEL.into();
        d.messages[0].to = "ana@x.example".into();
        assert!(
            d.render().contains(&format!("From: {USER_LABEL}\nTo: ana@x.example\n")),
            "{}",
            d.render()
        );
        d.messages[0].to = String::new();
        assert!(!d.render().contains("To:"), "no recipients, no To line");
    }

    fn mine(mut d: ResearchDoc, id: &str) -> ResearchDoc {
        for m in d.messages.iter_mut().filter(|m| m.id == id) {
            m.from_user = true;
        }
        d
    }

    #[test]
    fn a_sent_question_counts_only_conversations_where_a_cited_email_is_the_users() {
        // t1: the user's own quote is cited. t2: the user asked, the supplier's
        // quote is what got cited — a quote received, not sent.
        let docs = vec![mine(conv("t1", &["e1"]), "e1"), mine(conv("t2", &["e2", "e3"]), "e2")];
        let notes = vec![BatchNotes {
            lines: vec!["- my quote (email://e1)".into(), "- their quote (email://e3)".into()],
            cited: vec!["e1".into(), "e3".into()],
        }];
        let ids = |ms: Vec<Match>| ms.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(collect_matches(&docs, &notes, Direction::Sent)), vec!["e1"]);
        assert_eq!(ids(collect_matches(&docs, &notes, Direction::Received)), vec!["e3"]);
        assert_eq!(ids(collect_matches(&docs, &notes, Direction::Any)), vec!["e1", "e3"]);
    }

    #[test]
    fn the_map_prompt_states_the_questions_direction() {
        let template = "HEAD\nQUESTION: {{question}}\n{{direction}}\n{{emails}}";
        let (_, tail) = split_map_prompt(template, "q?", &[doc("e1")], Direction::Sent);
        assert!(tail.contains(direction_note(Direction::Sent)), "{tail}");
        let (_, tail) = split_map_prompt(template, "q?", &[doc("e1")], Direction::Any);
        assert!(!tail.contains("SENT") && !tail.contains("RECEIVED"), "{tail}");
    }

    #[test]
    fn repeated_inline_links_to_one_conversation_collapse() {
        let map = HashMap::from([("e2".to_string(), "e1".to_string())]);
        assert_eq!(
            canonicalize_links("Paid [A](email://e1)[B](email://e2) and [C](email://e9).", &map),
            "Paid [A](email://e1) and [C](email://e9)."
        );
    }

    #[test]
    fn a_conversation_renders_once_with_every_message_id() {
        let text = conv("t1", &["e1", "e2"]).render();
        assert!(text.starts_with("CONVERSATION: Invoice 42\n"), "{text}");
        assert_eq!(text.matches("EMAIL_ID: ").count(), 2);
        assert!(text.contains("EMAIL_ID: e1\n") && text.contains("EMAIL_ID: e2\n"));
    }

    #[test]
    fn links_to_any_email_of_a_conversation_point_at_its_representative() {
        let map = HashMap::from([
            ("e2".to_string(), "e1".to_string()),
            ("e3".to_string(), "e1".to_string()),
        ]);
        let out = canonicalize_links("[a](email://e2) and [b](email://e3), [c](email://e9)", &map);
        assert_eq!(out, "[a](email://e1) and [b](email://e1), [c](email://e9)");
    }

    #[test]
    fn map_prompt_keeps_the_batch_out_of_the_prefix() {
        let tmpl = "Extract findings.\n\nQUESTION: {{question}}\n\nEMAILS:\n{{emails}}";
        let (prefix, suffix) = split_map_prompt(tmpl, "¿qué facturas?", &[doc("e1"), doc("e2")], Direction::Any);
        assert_eq!(prefix, "Extract findings.\n\n");
        assert!(suffix.starts_with("QUESTION: ¿qué facturas?"));
        assert!(suffix.contains("EMAIL_ID: e1") && suffix.contains("EMAIL_ID: e2"));
        // The prefix is identical for another question and batch.
        let (other, _) = split_map_prompt(tmpl, "other", &[doc("e9")], Direction::Any);
        assert_eq!(prefix, other);
    }

    #[test]
    fn map_prompt_without_marker_still_renders_everything() {
        let (prefix, suffix) = split_map_prompt("Q={{question}} E={{emails}}", "q", &[doc("e1")], Direction::Any);
        assert!(prefix.contains("Q=q") && prefix.contains("EMAIL_ID: e1"));
        assert!(suffix.is_empty());
    }

    #[test]
    fn notes_keep_only_findings_citing_a_batch_email() {
        let reply = "Here are the findings:\n\
- Alice asks to pay invoice 42 by Friday (email://e1)\n\
* Bob confirms the refund (email://e2) (email://e1)\n\
- Something about email://zzz\n\
NONE";
        let notes = parse_map_notes(reply, &ids(&["e1", "e2"]));
        assert_eq!(
            notes.lines,
            vec![
                "- Alice asks to pay invoice 42 by Friday (email://e1)".to_string(),
                "- Bob confirms the refund (email://e2) (email://e1)".to_string(),
            ]
        );
        assert_eq!(notes.cited, ids(&["e1", "e2"]));
    }

    #[test]
    fn a_none_reply_yields_no_notes() {
        assert_eq!(parse_map_notes("NONE", &ids(&["e1"])), BatchNotes::default());
        assert_eq!(parse_map_notes("", &ids(&["e1"])), BatchNotes::default());
    }

    #[test]
    fn notes_fit_whole_when_under_budget() {
        let b = vec![
            BatchNotes {
                lines: vec!["- a (email://1)".into()],
                cited: ids(&["1"]),
            },
            BatchNotes {
                lines: vec!["- b (email://2)".into()],
                cited: ids(&["2"]),
            },
        ];
        assert_eq!(assemble_notes(&b, 1000), "- a (email://1)\n- b (email://2)\n");
    }

    #[test]
    fn overflowing_notes_keep_a_share_of_every_batch() {
        let batch = |tag: &str| BatchNotes {
            lines: (0..10).map(|i| format!("- {tag}{i} ..........")).collect(),
            cited: vec![],
        };
        let notes = assemble_notes(&[batch("a"), batch("b")], 100);
        assert!(notes.chars().count() <= 100, "{notes}");
        assert!(notes.contains("- a0") && notes.contains("- b0"), "{notes}");
        assert!(!notes.contains("- a9"));
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
        let (prefix, suffix) = split_map_prompt(CHAT_RESEARCH_MAP, "Q?", &[doc("e1")], Direction::Any);
        assert!(!prefix.contains("Q?") && !prefix.contains("e1"));
        assert!(suffix.contains("Q?") && suffix.contains("EMAIL_ID: e1"));
        assert!(!suffix.contains("{{"), "unrendered placeholder: {suffix}");

        let (prefix, suffix) = split_reduce_prompt(
            CHAT_RESEARCH_REDUCE,
            "Reply in Spanish.",
            "Q?",
            "read 5",
            "7 emails with relevant findings",
            "- n (email://e1)",
            Direction::Any,
        );
        assert!(prefix.contains("Reply in Spanish."));
        for per_turn in ["Q?", "read 5", "7 emails with relevant findings", "email://e1)"] {
            assert!(!prefix.contains(per_turn), "{per_turn} leaked into the cached prefix");
            assert!(suffix.contains(per_turn));
        }
        assert!(!prefix.contains("{{") && !suffix.contains("{{"));
    }

    fn subjects() -> HashMap<String, String> {
        HashMap::from([
            ("e1".to_string(), "Invoice 42".to_string()),
            ("e2".to_string(), "Re: [Q3] refund ]".to_string()),
        ])
    }

    #[test]
    fn relink_turns_bracketed_ids_into_subject_links() {
        let out = relink_bare_refs("Pay by Friday [email://e1].", &subjects());
        assert_eq!(out, "Pay by Friday [Invoice 42](email://e1).");
    }

    #[test]
    fn relink_turns_parenthesised_ids_into_subject_links() {
        let out = relink_bare_refs("Pay by Friday (email://e1) and refund (email://e2)", &subjects());
        assert_eq!(
            out,
            "Pay by Friday [Invoice 42](email://e1) and refund [Re: Q3 refund](email://e2)"
        );
    }

    #[test]
    fn relink_leaves_proper_links_and_unknown_ids_alone() {
        let text = "See [the invoice](email://e1), and [email://zzz].";
        assert_eq!(relink_bare_refs(text, &subjects()), text);
    }

    #[test]
    fn relink_relabels_a_link_whose_label_is_its_id() {
        // What a small model writes when told to cite as a link: the id as label.
        let out = relink_bare_refs("*   [email://e1](email://e1)", &subjects());
        assert_eq!(out, "*   [Invoice 42](email://e1)");
    }

    #[test]
    fn repeated_link_bullets_collapse_after_canonicalizing() {
        let map = HashMap::from([("e2".to_string(), "e1".to_string())]);
        let answer = "*   **Acme:** quote sent.\n    *   [A](email://e1)\n    *   [B](email://e2)\n*   **Beta:** quote.\n    *   [A](email://e1)";
        assert_eq!(
            canonicalize_links(answer, &map),
            "*   **Acme:** quote sent.\n    *   [A](email://e1)\n*   **Beta:** quote.\n    *   [A](email://e1)",
            "the second cite of one conversation under one bullet goes; another bullet may cite it again"
        );
    }

    #[test]
    fn relink_labels_an_email_without_subject_generically() {
        let map = HashMap::from([("e3".to_string(), "   ".to_string())]);
        assert_eq!(relink_bare_refs("x [email://e3]", &map), "x [email](email://e3)");
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

    #[test]
    fn join_notes_and_notes_len_agree() {
        let b = BatchNotes {
            lines: vec!["- a (email://1)".into(), "- bb (email://2)".into()],
            cited: ids(&["1", "2"]),
        };
        assert_eq!(
            join_notes(std::slice::from_ref(&b)),
            "- a (email://1)\n- bb (email://2)\n"
        );
        assert_eq!(notes_len(&b), join_notes(&[b]).chars().count());
    }

    // ── matches, counts, full list ──

    #[test]
    fn a_list_or_count_question_gets_the_full_list() {
        for q in [
            "dame una lista con todas las peticiones de contacto",
            "List every invoice from Hetzner",
            "¿Cuántas facturas he recibido este año?",
            "how many customers wrote about pricing?",
            "enumera los proveedores",
            "combien de demandes de contact ?",
            "Wie viele Rechnungen?",
            "all the emails where someone asks for a demo",
        ] {
            assert_eq!(plan_report_shape(q), ReportShape::FullList, "{q}");
        }
    }

    #[test]
    fn an_analysis_question_gets_no_appended_list() {
        for q in [
            "¿Qué temas principales han salido con clientes?",
            "Research how downloads evolved over the last 3 months",
            "summarise the recurring issues users report",
        ] {
            assert_eq!(plan_report_shape(q), ReportShape::Analysis, "{q}");
        }
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
    fn a_cut_report_drops_its_broken_line_and_ends_with_every_match() {
        let matches = vec![
            m("e1", "t1", "2024-01-02", "quote A"),
            m("e9", "t9", "2024-03-04", "quote B"),
        ];
        let cut = "Summary.\n*   **A:** sent.\n    *   [email://e9";
        assert_eq!(
            finish_report(cut, true, &matches, "en"),
            format!("Summary.\n*   **A:** sent.\n\n{}", render_match_list(&matches, "en"))
        );
    }

    #[test]
    fn a_report_that_links_nothing_ends_with_every_match() {
        let matches = vec![m("e1", "t1", "2024-01-02", "quote A")];
        let prose = "You sent one quote, to Acme.";
        assert_eq!(
            finish_report(prose, false, &matches, "en"),
            format!("{prose}\n\n{}", render_match_list(&matches, "en"))
        );
    }

    #[test]
    fn a_match_the_user_did_not_write_is_background_on_a_sent_question() {
        // The model read the supplier's reply as the user's own quote.
        let mut supplier = conv("t2", &["e2", "e3"]);
        supplier.messages[0].from_user = true; // e2: the user's request
        let docs = vec![mine(conv("t1", &["e1"]), "e1"), supplier];
        let notes = BatchNotes {
            lines: vec![
                "- MATCH: Sent a quote (email://e1)".into(),
                "- MATCH: The user sent a 90 EUR quote (email://e3)".into(),
                "- CONTEXT: Asked for a quote (email://e2)".into(),
            ],
            cited: ids(&["e1", "e3", "e2"]),
        };
        let out = enforce_direction(notes.clone(), &docs, Direction::Sent);
        assert_eq!(out.lines[0], notes.lines[0]);
        assert_eq!(
            out.lines[1],
            "- CONTEXT: The user sent a 90 EUR quote (email://e3) [written by Alice <alice@example.com>, not by the user]"
        );
        assert_eq!(out.lines[2], notes.lines[2]);
        assert_eq!(out.cited, notes.cited);
        assert_eq!(enforce_direction(notes.clone(), &docs, Direction::Any), notes);
    }

    #[test]
    fn a_match_the_user_wrote_is_background_on_a_received_question() {
        let docs = vec![mine(conv("t1", &["e1"]), "e1")];
        let notes = BatchNotes {
            lines: vec!["- MATCH: Got a quote (email://e1)".into()],
            cited: ids(&["e1"]),
        };
        let out = enforce_direction(notes, &docs, Direction::Received);
        assert_eq!(
            out.lines[0],
            "- CONTEXT: Got a quote (email://e1) [written by the user]"
        );
    }

    #[test]
    fn a_complete_report_is_left_as_written() {
        let matches = vec![m("e1", "t1", "", "")];
        let linked = "All done: [the quote](email://e1).";
        assert_eq!(finish_report(linked, false, &matches, "en"), linked);
    }

    #[test]
    fn matches_take_each_conversations_first_finding_without_its_references() {
        let docs = vec![conv("t1", &["e1", "e2"]), conv("t2", &["e3"])];
        let notes = vec![BatchNotes {
            lines: vec![
                "- Alice asks for a quote (email://e1)".into(),
                "- Alice follows up [Invoice 42](email://e1)".into(),
                "- Bob too (email://e3) (email://e1)".into(),
            ],
            cited: ids(&["e1", "e3"]),
        }];
        let matches = collect_matches(&docs, &notes, Direction::Any);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].id, "e1");
        assert_eq!(matches[0].finding, "Alice asks for a quote");
        assert_eq!(matches[1].id, "e3");
        assert_eq!(matches[1].finding, "Bob too");
    }

    #[test]
    fn a_context_finding_is_kept_as_a_note_but_never_makes_a_match() {
        // The user's request for a supplier's quote is related to "which
        // quotes have I sent?" but does not answer it.
        let docs = vec![conv("t1", &["e1"]), conv("t2", &["e2"])];
        let notes = vec![BatchNotes {
            lines: vec![
                "- MATCH: Sent a quote for 8,400 EUR (email://e1)".into(),
                "- CONTEXT: Requested a translation quote (email://e2)".into(),
            ],
            cited: ids(&["e1", "e2"]),
        }];
        let matches = collect_matches(&docs, &notes, Direction::Any);
        assert_eq!(matches.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["e1"]);
        assert_eq!(
            matches[0].finding, "Sent a quote for 8,400 EUR",
            "the tag is not part of the finding"
        );
    }

    #[test]
    fn finding_tags_are_read_in_the_shapes_a_small_model_writes() {
        for line in ["- CONTEXT: x", "- **CONTEXT:** x", "- [Context] x", "* context: x"] {
            assert_eq!(finding_tag(line), (FindingTag::Context, "x"), "{line}");
        }
        for line in ["- MATCH: x", "- **Match**: x", "- x"] {
            assert_eq!(finding_tag(line), (FindingTag::Match, "x"), "{line}");
        }
    }

    #[test]
    fn a_finding_that_starts_with_the_word_is_not_a_tag() {
        assert_eq!(
            finding_tag("- Contextual help shipped"),
            (FindingTag::Match, "Contextual help shipped")
        );
        assert_eq!(
            finding_tag("- Matched invoice 42"),
            (FindingTag::Match, "Matched invoice 42")
        );
        assert_eq!(
            finding_tag("- Match fees were paid"),
            (FindingTag::Match, "Match fees were paid")
        );
    }

    #[test]
    fn a_conversation_is_one_match_however_many_of_its_emails_are_cited() {
        // A thread of replies all quoting the same request used to show up as
        // five matches in the progress, five lines in the list and five in
        // the count.
        let docs = vec![conv("t1", &["e1", "e2", "e3"]), conv("t2", &["e4"])];
        let notes = vec![BatchNotes {
            lines: vec![
                "- Budget requested (email://e1)".into(),
                "- Follow-up on the budget (email://e2)".into(),
                "- Budget accepted (email://e3)".into(),
                "- Another client asks (email://e4)".into(),
            ],
            cited: ids(&["e1", "e2", "e3", "e4"]),
        }];
        let matches = collect_matches(&docs, &notes, Direction::Any);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].id, "e1", "the conversation's first matching email");
        assert_eq!(matches[0].emails, 3);
        assert_eq!(matches[0].finding, "Budget requested");
        assert_eq!(matches[1].emails, 1);
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
    fn the_full_list_says_how_many_emails_a_conversation_holds() {
        let matches = vec![Match {
            emails: 3,
            ..m("e1", "t1", "2026-01-02", "Budget requested")
        }];
        let list = render_match_list(&matches, "es");
        assert!(list.contains("— Budget requested (3 correos)"), "{list}");
        assert!(render_match_list(&matches, "en").contains("(3 emails)"));
    }

    #[test]
    fn report_facts_give_exact_counts_and_announce_the_list_only_when_appended() {
        let matches = vec![m("e1", "t1", "", ""), m("e2", "t2", "", "")];
        let list = report_facts(&matches, ReportShape::FullList);
        assert!(
            list.contains("2 emails with relevant findings, in 2 conversations"),
            "{list}"
        );
        assert!(list.contains("appended"), "{list}");
        let analysis = report_facts(&matches, ReportShape::Analysis);
        assert!(analysis.contains("2 emails"), "{analysis}");
        assert!(!analysis.contains("appended"), "{analysis}");
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
}
