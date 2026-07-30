use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;

use crate::ai::provider::CompletionOptions;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;
use crate::services::ai::AiService;
use crate::services::i18n::Language;
use crate::services::retrieval::{
    dedup_by_thread, fetch_fts, fetch_vector, fuse_rrf, FtsRequest, Ranking, VectorRequest, DEFAULT_RRF_K,
};
use crate::util::text::truncate_utf8;

/// Maximum total prompt size after substitution — leaves room for the model's
/// own generation budget. The model still gets a usable thread + RAG slice
/// even when both are large.
const MAX_PROMPT_CHARS: usize = 12_000;
/// Per-message truncation inside the thread context. The previous value of
/// 300 chars was too aggressive — long messages lost substantive content
/// before the model ever saw them.
const IN_THREAD_MSG_CHARS: usize = 1_500;
const RAG_SNIPPET_CHARS: usize = 1_500;
const RAG_TOP_K: usize = 3;
const RAG_POOL_SIZE: usize = 30;

pub const DEFAULT_PROMPT_TEMPLATE: &str = r#"You are an email assistant for {persona}.
Writing style: {style}
Language: Match the language of the original email.

{thread_context}
{rag_context}
{instructions}Write the reply (body only, no subject line, no signature):"#;

/// One past thread fed into the draft prompt as precedent. Returned to the
/// frontend so the user can see *why* the draft looks the way it does and
/// audit the retrieval.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSource {
    pub email_id: String,
    pub thread_id: String,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub timestamp: i64,
    pub score: f32,
    pub snippet: String,
    /// True when the source excerpt is the user's own reply in that thread —
    /// i.e. precedent for *how the user actually wrote*. Frontend uses this
    /// to badge the card.
    pub sent_by_user: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftResult {
    pub body: String,
    pub sources: Vec<DraftSource>,
}

fn emit_log(level: &str, message: &str) {
    crate::services::logger::log(level, "drafts", message);
}

/// Generate a reply draft for `email_id`.
///
/// Runs synchronously on the caller's tokio task. Frontend-facing callers
/// submit this through `ai_queue` so the UI thread is not blocked while
/// Ollama runs; UI events flow through the global `events`/`logger` sinks, so
/// the eval harness and CLI can call it without a tauri runtime.
pub async fn generate_draft(db: &Arc<Database>, email_id: &str, instructions: Option<&str>) -> Result<DraftResult> {
    let email = db
        .get_email_by_id(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {} not found", email_id)))?;
    let thread = db.get_thread(&email.account_id, &email.thread_id)?;

    let persona = db
        .get_preference("draft_persona")?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "a freelance CTO and technical consultant".to_string());
    let style = db
        .get_preference("draft_style")?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            "concise, friendly but professional, uses short paragraphs, avoids corporate jargon".to_string()
        });
    let prompt_template = db
        .get_preference("draft_prompt_template")?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT_TEMPLATE.to_string());

    let user_email = db.get_account(&email.account_id)?.map(|a| a.email).unwrap_or_default();

    emit_log(
        "info",
        &format!("Generating draft for '{}'…", truncate_utf8(&email.subject, 60)),
    );

    // ── Retrieval ─────────────────────────────────────────────────────────
    // Embed the inbound email and retrieve precedent threads (vector + FTS
    // fused with RRF, deduped to one email per thread, excluding the current
    // thread). If embeddings aren't available we skip silently — the draft
    // still works without RAG, just with less stylistic grounding.
    let sources = match retrieve_rag_sources(db, &email, &user_email).await {
        Ok(s) => s,
        Err(e) => {
            emit_log(
                "warn",
                &format!("retrieval skipped ({}); generating without precedent", e),
            );
            Vec::new()
        }
    };

    // ── Prompt assembly ──────────────────────────────────────────────────
    let thread_context = build_thread_context(&thread);
    let rag_context = build_rag_context(&sources, &user_email);
    let instructions_section = match instructions {
        Some(i) if !i.trim().is_empty() => format!("Additional instructions: {}\n\n", i.trim()),
        _ => String::new(),
    };

    let mut prompt = prompt_template
        .replace("{persona}", &persona)
        .replace("{style}", &style)
        .replace("{thread_context}", &thread_context)
        .replace("{rag_context}", &rag_context)
        .replace("{instructions}", &instructions_section);

    if prompt.len() > MAX_PROMPT_CHARS {
        prompt = truncate_utf8(&prompt, MAX_PROMPT_CHARS).to_string();
    }

    // ── Model call ───────────────────────────────────────────────────────
    emit_log("info", "calling model…");
    let ai = AiService::new(db.clone())?;
    let config = AiService::get_config(db)?;
    let start = std::time::Instant::now();
    let draft = ai
        .complete(
            &prompt,
            "generate_draft",
            Some(CompletionOptions {
                temperature: Some(0.7),
                max_tokens: Some(800),
                think: None,
            }),
        )
        .await?;
    let elapsed = start.elapsed().as_millis();
    let body = draft.trim().to_string();

    crate::services::logger::log(
        "debug",
        "ai",
        format!(
            "draft reply generated for '{}' (provider={}, model={}, thread={} msgs, sources={}, prompt={} chars, draft={} chars, {}ms)",
            truncate_utf8(&email.subject, 40),
            config.provider,
            config.model,
            thread.len(),
            sources.len(),
            prompt.len(),
            body.len(),
            elapsed
        ),
    );
    emit_log(
        "success",
        &format!("draft generated ({} chars, {}ms)", body.len(), elapsed),
    );

    Ok(DraftResult { body, sources })
}

/// Validate and normalize the recipient list + subject for a new-email draft.
///
/// Pure: trims each recipient, drops blanks, and rejects an empty recipient
/// set or empty subject. Extracted from `generate_new_draft` so the guard is
/// unit-tested without a DB or model call — both the chat tool and the compose
/// `generate_new_draft` command rely on it to reject bad input before any AI
/// work is queued.
fn clean_new_draft_inputs(to: &[String], subject: &str) -> Result<(Vec<String>, String)> {
    let to_clean: Vec<String> = to
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if to_clean.is_empty() {
        return Err(AppError::InvalidInput("at least one recipient required".into()));
    }
    let subject = subject.trim();
    if subject.is_empty() {
        return Err(AppError::InvalidInput("subject required".into()));
    }
    Ok((to_clean, subject.to_string()))
}

/// Assemble the new-email draft prompt.
///
/// Pure so the language directive is unit-tested without a DB or model call.
/// The key line is `Write the entire email in {language}.` — sourced from
/// `resolve_ai_language`, the same explicit-language convention the chat,
/// classification, task, and memory prompts use. The previous soft "Match the
/// language of the subject" hint let the model default to English even when the
/// user had explicitly chosen another output language.
fn build_new_draft_prompt(
    persona: &str,
    style: &str,
    language: Language,
    to: &[String],
    subject: &str,
    instructions_section: &str,
) -> String {
    format!(
        "You are an email assistant for {persona}.\n\
Writing style: {style}\n\
Write the entire email in {lang}.\n\n\
Compose a NEW email (not a reply). There is no prior thread to reference.\n\n\
Recipients: {recipients}\n\
Subject: {subject}\n\n\
{instructions_section}Write the body only (no subject line, no greeting headers, no signature):",
        persona = persona,
        style = style,
        lang = language.english_name(),
        recipients = to.join(", "),
        subject = subject,
        instructions_section = instructions_section,
    )
}

/// Generate a draft for a brand-new email (no existing thread). Used by the
/// chat `generate_email_draft` tool when the user asks for a new message
/// rather than a reply ("draft a new email to billing@stripe…").
///
/// Mirrors `generate_draft` but skips the thread-context and RAG steps — a
/// new-email path has no inbound message to embed against. Persona / style /
/// prompt-template preferences still apply, so the draft sounds like the
/// user. Sources is returned empty; the result body still flows through the
/// same `DraftResult` shape so callers can save it via the existing
/// `db.save_draft` path.
pub async fn generate_new_draft(
    db: &Arc<Database>,
    account_id: &str,
    to: &[String],
    subject: &str,
    instructions: Option<&str>,
) -> Result<DraftResult> {
    let (to_clean, subject) = clean_new_draft_inputs(to, subject)?;
    let subject = subject.as_str();

    let persona = db
        .get_preference("draft_persona")?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "a freelance CTO and technical consultant".to_string());
    let style = db
        .get_preference("draft_style")?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            "concise, friendly but professional, uses short paragraphs, avoids corporate jargon".to_string()
        });

    emit_log(
        "info",
        &format!(
            "Generating new email draft to {} subject='{}'…",
            to_clean.join(","),
            truncate_utf8(subject, 60)
        ),
    );

    let instructions_section = match instructions {
        Some(i) if !i.trim().is_empty() => format!("Additional instructions: {}\n\n", i.trim()),
        _ => String::new(),
    };

    // Honor the user's explicit AI output language (ai_output_language_v2 →
    // ai_output_language → ui_language → English), same as chat / classify /
    // tasks / memory. A short subject alone left the model defaulting to
    // English despite an explicit `es` preference.
    let language = crate::services::i18n::resolve_ai_language(db)?;

    // Distinct from the reply template because the model needs to be told
    // there's no inbound thread — otherwise small models hallucinate one.
    let prompt = build_new_draft_prompt(&persona, &style, language, &to_clean, subject, &instructions_section);

    let prompt = if prompt.len() > MAX_PROMPT_CHARS {
        truncate_utf8(&prompt, MAX_PROMPT_CHARS).to_string()
    } else {
        prompt
    };

    emit_log("info", "calling model…");
    let ai = AiService::new(db.clone())?;
    let _account = db.get_account(account_id)?;
    let start = std::time::Instant::now();
    let draft = ai
        .complete(
            &prompt,
            "generate_new_draft",
            Some(CompletionOptions {
                temperature: Some(0.7),
                max_tokens: Some(800),
                think: None,
            }),
        )
        .await?;
    let elapsed = start.elapsed().as_millis();
    let body = draft.trim().to_string();
    emit_log(
        "success",
        &format!("new draft generated ({} chars, {}ms)", body.len(), elapsed),
    );
    Ok(DraftResult {
        body,
        sources: Vec::new(),
    })
}

/// Vector + FTS retrieval of similar past threads. Returns up to `RAG_TOP_K`
/// `DraftSource` entries, each one representing the *most relevant* email in
/// a past thread (with thread-level dedup so we don't show three messages
/// from the same conversation).
///
/// For each surviving thread we prefer the user's own reply (best precedent
/// of voice) and fall back to the latest message in that thread otherwise.
async fn retrieve_rag_sources(db: &Arc<Database>, email: &Email, user_email: &str) -> Result<Vec<DraftSource>> {
    let ai = AiService::new(db.clone())?;

    let embed_text = format!("{}\n{}", email.subject, truncate_utf8(&email.snippet, 2000));
    emit_log("info", "retrieving similar threads…");

    let query_vec = ai.embed(&embed_text).await?;
    let fts_query = build_fts_query(&email.subject, &email.snippet);

    let fts_hits = fetch_fts(
        db,
        FtsRequest {
            account_id: &email.account_id,
            query: &fts_query,
            categories: None,
            sender_email_eq: None,
            limit: RAG_POOL_SIZE as i32,
        },
    )
    .unwrap_or_default();
    let vec_hits = fetch_vector(
        db,
        VectorRequest {
            account_id: &email.account_id,
            embedding: &query_vec,
            categories: None,
            limit: RAG_POOL_SIZE,
        },
    )
    .unwrap_or_default();

    let fts_ids: Vec<String> = fts_hits.iter().map(|(id, _)| id.clone()).collect();
    let vec_ids: Vec<String> = vec_hits.iter().map(|(id, _)| id.clone()).collect();
    let ranked = fuse_rrf(
        &[
            Ranking {
                ids_in_order: &fts_ids,
                weight: 1.0,
            },
            Ranking {
                ids_in_order: &vec_ids,
                weight: 1.0,
            },
        ],
        DEFAULT_RRF_K,
    );

    // Resolve thread ids for every candidate so dedup can collapse to one
    // email per thread. We batch lookups to minimise DB round-trips.
    let unique_ids: HashSet<&str> = ranked.iter().map(|(id, _)| id.as_str()).collect();
    let mut tid_lookup: HashMap<String, String> = HashMap::new();
    for id in unique_ids {
        if let Ok(Some(e)) = db.get_email_by_id(id) {
            tid_lookup.insert(id.to_string(), e.thread_id);
        }
    }
    let deduped = dedup_by_thread(ranked, |id| tid_lookup.get(id).cloned());

    let mut sources: Vec<DraftSource> = Vec::new();
    let current_thread_id = email.thread_id.as_str();

    for (eid, score) in deduped.into_iter() {
        if sources.len() >= RAG_TOP_K {
            break;
        }
        let candidate = match db.get_email_by_id(&eid)? {
            Some(e) => e,
            None => continue,
        };
        if candidate.thread_id == current_thread_id {
            continue;
        }

        let context_email = pick_thread_context_email(db, &candidate.account_id, &candidate.thread_id, user_email)
            .unwrap_or_else(|| candidate.clone());

        let body = db.get_email_body(&context_email.id).unwrap_or_default();
        let body_clean = strip_html_for_prompt(&body);
        let snippet = if body_clean.trim().is_empty() {
            truncate_utf8(&context_email.snippet, RAG_SNIPPET_CHARS).to_string()
        } else {
            truncate_utf8(&body_clean, RAG_SNIPPET_CHARS).to_string()
        };
        let sent_by_user = !user_email.is_empty() && context_email.sender_email.eq_ignore_ascii_case(user_email);

        sources.push(DraftSource {
            email_id: context_email.id.clone(),
            thread_id: context_email.thread_id.clone(),
            subject: context_email.subject.clone(),
            sender: context_email.sender.clone(),
            sender_email: context_email.sender_email.clone(),
            timestamp: context_email.timestamp,
            score,
            snippet,
            sent_by_user,
        });
    }

    emit_log("info", &format!("found {} similar threads for context", sources.len()));
    Ok(sources)
}

fn build_thread_context(thread: &[Email]) -> String {
    if thread.is_empty() {
        return String::new();
    }
    let mut s = String::from("Email thread (oldest first):\n");
    for (i, msg) in thread.iter().enumerate() {
        s.push_str(&format!(
            "\n--- Message {} ---\nFrom: {} <{}>\nSubject: {}\n{}\n",
            i + 1,
            msg.sender,
            msg.sender_email,
            msg.subject,
            truncate_utf8(&msg.snippet, IN_THREAD_MSG_CHARS),
        ));
    }
    if let Some(last) = thread.last() {
        s.push_str(&format!(
            "\nThe latest message is from {}. Write a reply to it.\n",
            last.sender
        ));
    }
    s
}

fn build_rag_context(sources: &[DraftSource], user_email: &str) -> String {
    if sources.is_empty() {
        return String::new();
    }
    let mut s = String::from("\nSimilar past threads (use for tone and precedent — do not quote verbatim):\n");
    for (i, src) in sources.iter().enumerate() {
        let role = if !user_email.is_empty() && src.sender_email.eq_ignore_ascii_case(user_email) {
            "your reply"
        } else {
            "received"
        };
        s.push_str(&format!(
            "\n[Precedent {} — {} — from {}]\nSubject: {}\n{}\n",
            i + 1,
            role,
            src.sender,
            src.subject,
            src.snippet,
        ));
    }
    s
}

/// Build a coarse FTS5 query from the inbound email's subject + snippet.
/// Strips short noise tokens, dedups, and caps at 30 terms — enough for
/// `bm25` to pull back relevant threads without busting parser limits.
fn build_fts_query(subject: &str, snippet: &str) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let text = format!("{} {}", subject, snippet);
    for w in text.split(|c: char| !c.is_alphanumeric()) {
        let w = w.to_lowercase();
        let len = w.chars().count();
        if !(3..=30).contains(&len) {
            continue;
        }
        if !seen.insert(w.clone()) {
            continue;
        }
        out.push(w);
        if out.len() >= 30 {
            break;
        }
    }
    out.join(" ")
}

/// Cheap HTML → plain-text for prompt insertion. Strips tags and collapses
/// whitespace. Intentionally not a real sanitizer — we never render this,
/// we only feed it to the model.
fn strip_html_for_prompt(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            out.push(' ');
            continue;
        }
        if !in_tag {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pick_thread_context_email(db: &Database, account_id: &str, thread_id: &str, user_email: &str) -> Option<Email> {
    let thread = db.get_thread(account_id, thread_id).ok()?;
    if !user_email.is_empty() {
        if let Some(reply) = thread
            .iter()
            .rev()
            .find(|m| m.sender_email.eq_ignore_ascii_case(user_email))
        {
            return Some(reply.clone());
        }
    }
    thread.into_iter().last()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_email(id: &str, sender: &str, sender_email: &str, subject: &str, snippet: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            thread_id: format!("thread-{id}"),
            message_id: None,
            subject: subject.to_string(),
            sender: sender.to_string(),
            sender_email: sender_email.to_string(),
            recipients: vec![],
            cc: vec![],
            body: String::new(),
            snippet: snippet.to_string(),
            timestamp: 1000,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "inbox".to_string(),
            is_sent: false,
            headers: None,
        }
    }

    fn make_source(sender_email: &str) -> DraftSource {
        DraftSource {
            email_id: "e1".to_string(),
            thread_id: "t1".to_string(),
            subject: "Test Subject".to_string(),
            sender: "Test Sender".to_string(),
            sender_email: sender_email.to_string(),
            timestamp: 1000,
            score: 0.9,
            snippet: "Test snippet".to_string(),
            sent_by_user: sender_email == "me@example.com",
        }
    }

    // ── build_new_draft_prompt ─────────────────────────────────────────────

    use crate::services::i18n::Language;

    #[test]
    fn build_new_draft_prompt_injects_resolved_language_spanish() {
        let prompt = build_new_draft_prompt(
            "a CTO",
            "concise",
            Language::Es,
            &["x@y.com".to_string()],
            "Facturas",
            "",
        );
        assert!(
            prompt.contains("in Spanish"),
            "must instruct the model to write in the resolved language (Spanish); got:\n{prompt}"
        );
    }

    #[test]
    fn build_new_draft_prompt_injects_resolved_language_english() {
        let prompt = build_new_draft_prompt(
            "a CTO",
            "concise",
            Language::En,
            &["x@y.com".to_string()],
            "Invoices",
            "",
        );
        assert!(
            prompt.contains("in English"),
            "English pref must yield an English directive"
        );
    }

    #[test]
    fn build_new_draft_prompt_does_not_hardcode_language_match_hint() {
        // The old vague "Match the language of the subject" line let the model
        // default to English despite an explicit es preference — it must be
        // gone in favour of the deterministic directive.
        let prompt = build_new_draft_prompt("p", "s", Language::Es, &["x@y.com".to_string()], "Facturas", "");
        assert!(
            !prompt.contains("Match the language"),
            "the soft subject-matching hint must be replaced by the explicit directive"
        );
    }

    #[test]
    fn build_new_draft_prompt_includes_recipients_subject_and_instructions() {
        let prompt = build_new_draft_prompt(
            "p",
            "s",
            Language::En,
            &["a@b.com".to_string(), "c@d.com".to_string()],
            "Kickoff",
            "Additional instructions: be brief\n\n",
        );
        assert!(
            prompt.contains("a@b.com, c@d.com"),
            "recipients must be joined into the prompt"
        );
        assert!(prompt.contains("Kickoff"), "subject must appear");
        assert!(prompt.contains("be brief"), "instructions section must be spliced in");
    }

    // ── clean_new_draft_inputs ─────────────────────────────────────────────

    #[test]
    fn clean_new_draft_inputs_trims_and_keeps_valid_recipients() {
        let (to, subject) = clean_new_draft_inputs(&["  a@x.com ".to_string(), "b@y.com".to_string()], "  Hello  ")
            .expect("valid input must pass");
        assert_eq!(to, vec!["a@x.com".to_string(), "b@y.com".to_string()]);
        assert_eq!(subject, "Hello", "subject must be trimmed");
    }

    #[test]
    fn clean_new_draft_inputs_drops_blank_recipients() {
        let (to, _) = clean_new_draft_inputs(&["".to_string(), "   ".to_string(), "keep@x.com".to_string()], "S")
            .expect("one valid recipient is enough");
        assert_eq!(to, vec!["keep@x.com".to_string()], "blank recipients must be filtered");
    }

    #[test]
    fn clean_new_draft_inputs_rejects_empty_recipient_set() {
        let err = clean_new_draft_inputs(&[], "Subject").expect_err("no recipients must fail");
        assert!(matches!(err, AppError::InvalidInput(_)), "must be InvalidInput");
    }

    #[test]
    fn clean_new_draft_inputs_rejects_all_blank_recipients() {
        let err =
            clean_new_draft_inputs(&["  ".to_string(), "".to_string()], "Subject").expect_err("all-blank must fail");
        assert!(matches!(err, AppError::InvalidInput(_)), "must be InvalidInput");
    }

    #[test]
    fn clean_new_draft_inputs_rejects_empty_subject() {
        let err = clean_new_draft_inputs(&["a@x.com".to_string()], "   ").expect_err("blank subject must fail");
        assert!(matches!(err, AppError::InvalidInput(_)), "must be InvalidInput");
    }

    // ── build_fts_query ────────────────────────────────────────────────────

    #[test]
    fn build_fts_query_empty_input_returns_empty() {
        assert_eq!(build_fts_query("", ""), "");
    }

    #[test]
    fn build_fts_query_includes_long_tokens() {
        let q = build_fts_query("invoice payment", "");
        assert!(q.contains("invoice"), "3+ char token must be included");
        assert!(q.contains("payment"), "3+ char token must be included");
    }

    #[test]
    fn build_fts_query_filters_tokens_shorter_than_3_chars() {
        // "hi", "to", "a", "be" are <= 2 chars — must all be filtered out
        let q = build_fts_query("hi to", "a be");
        assert!(q.is_empty(), "all tokens < 3 chars must be filtered; got: '{q}'");
    }

    #[test]
    fn build_fts_query_deduplicates_tokens() {
        let q = build_fts_query("invoice budget", "invoice approval");
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let unique_count = {
            let mut s = std::collections::HashSet::new();
            s.extend(tokens.iter().copied());
            s.len()
        };
        assert_eq!(tokens.len(), unique_count, "duplicate tokens must be removed");
    }

    #[test]
    fn build_fts_query_caps_at_30_terms() {
        // Build 40 distinct long tokens
        let subject = (0..20_u32).map(|i| format!("wordabc{i}")).collect::<Vec<_>>().join(" ");
        let snippet = (20..40_u32)
            .map(|i| format!("wordabc{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let q = build_fts_query(&subject, &snippet);
        let count = q.split_whitespace().count();
        assert_eq!(count, 30, "output must be capped at exactly 30 terms; got {count}");
    }

    #[test]
    fn build_fts_query_lowercases_tokens() {
        let q = build_fts_query("Invoice PAYMENT Budget", "");
        assert!(q.contains("invoice"), "tokens must be lowercased");
        assert!(q.contains("payment"), "tokens must be lowercased");
        assert!(q.contains("budget"), "tokens must be lowercased");
    }

    // ── strip_html_for_prompt ──────────────────────────────────────────────

    #[test]
    fn strip_html_plain_text_unchanged() {
        let result = strip_html_for_prompt("Hello World");
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn strip_html_removes_tags_preserves_content() {
        let result = strip_html_for_prompt("<p>Hello <b>world</b></p>");
        assert!(!result.contains('<'), "no angle brackets must remain");
        assert!(!result.contains('>'), "no angle brackets must remain");
        assert!(result.contains("Hello"), "text content must be preserved");
        assert!(result.contains("world"), "text content must be preserved");
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        let result = strip_html_for_prompt("<p>  Hello  </p>  <p>World</p>");
        assert!(!result.contains("   "), "multiple spaces must be collapsed");
        assert!(
            result.starts_with("Hello") || result.contains("Hello "),
            "collapsed result must contain Hello"
        );
    }

    #[test]
    fn strip_html_empty_input_returns_empty() {
        assert_eq!(strip_html_for_prompt(""), "");
    }

    #[test]
    fn strip_html_handles_only_tags() {
        let result = strip_html_for_prompt("<div><span></span></div>");
        assert!(result.trim().is_empty(), "only-tags input must produce empty text");
    }

    // ── build_thread_context ───────────────────────────────────────────────

    #[test]
    fn build_thread_context_empty_thread_returns_empty() {
        assert_eq!(build_thread_context(&[]), "");
    }

    #[test]
    fn build_thread_context_includes_sender_and_subject() {
        let email = make_email(
            "e1",
            "Alice",
            "alice@example.com",
            "Budget Review",
            "Please review the attached",
        );
        let ctx = build_thread_context(&[email]);
        assert!(ctx.contains("Alice"), "sender name must appear in context");
        assert!(ctx.contains("alice@example.com"), "sender email must appear");
        assert!(ctx.contains("Budget Review"), "subject must appear");
        assert!(ctx.contains("Please review"), "snippet must appear");
    }

    #[test]
    fn build_thread_context_mentions_write_reply_to_latest() {
        let e1 = make_email("e1", "Alice", "alice@example.com", "Hello", "hi");
        let e2 = make_email("e2", "Bob", "bob@example.com", "Re: Hello", "reply here");
        let ctx = build_thread_context(&[e1, e2]);
        assert!(ctx.contains("Bob"), "latest sender must appear");
        assert!(
            ctx.to_lowercase().contains("reply"),
            "context must prompt the model to write a reply"
        );
    }

    // ── build_rag_context ─────────────────────────────────────────────────

    #[test]
    fn build_rag_context_empty_sources_returns_empty() {
        assert_eq!(build_rag_context(&[], "me@example.com"), "");
    }

    #[test]
    fn build_rag_context_labels_own_reply_as_your_reply() {
        let src = make_source("me@example.com");
        let ctx = build_rag_context(&[src], "me@example.com");
        assert!(
            ctx.contains("your reply"),
            "own outgoing email must be labelled 'your reply'"
        );
    }

    #[test]
    fn build_rag_context_labels_received_message_correctly() {
        let src = make_source("client@corp.com");
        let ctx = build_rag_context(&[src], "me@example.com");
        assert!(ctx.contains("received"), "incoming email must be labelled 'received'");
    }

    #[test]
    fn build_rag_context_includes_subject_and_snippet() {
        let src = make_source("client@corp.com");
        let ctx = build_rag_context(&[src], "me@example.com");
        assert!(ctx.contains("Test Subject"), "subject must appear in RAG context");
        assert!(ctx.contains("Test snippet"), "snippet must appear in RAG context");
    }
}
