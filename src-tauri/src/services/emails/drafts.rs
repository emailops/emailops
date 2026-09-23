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
/// The message being answered keeps up to this many chars of its cleaned body
/// before older messages get any budget.
const TARGET_MSG_MAX_CHARS: usize = 6_000;
/// Below this, an older message's excerpt is too short to be useful: drop the
/// oldest messages instead of shaving everyone down to noise.
const MIN_OLDER_MSG_CHARS: usize = 300;
/// The thread always gets at least this much, even when a long custom
/// template, precedents or instructions eat most of `MAX_PROMPT_CHARS`.
const MIN_THREAD_CHARS: usize = 2_000;
const RAG_SNIPPET_CHARS: usize = 1_500;
/// How many of the user's past messages to the same correspondent are shown
/// as voice samples, and how much of each.
const STYLE_SAMPLES_K: usize = 2;
const STYLE_SAMPLE_CHARS: usize = 800;
/// Shorter samples ("Ok, gracias") say nothing about how the user writes.
const MIN_STYLE_SAMPLE_CHARS: usize = 40;
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
    let thread = thread_up_to(db.get_thread(&email.account_id, &email.thread_id)?, &email.id);

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
    let thread_messages = load_thread_messages(db, &thread);
    let style_samples = load_style_samples(db, &email, &user_email);
    let rag_context = format!(
        "{}{}",
        build_style_context(&style_samples, &email.sender),
        build_rag_context(&sources, &user_email)
    );
    let prompt = plan_reply_prompt(&ReplyPromptInput {
        template: &prompt_template,
        persona: &persona,
        style: &style,
        thread: &thread_messages,
        rag_context: &rag_context,
        instructions,
    });

    // ── Model call ───────────────────────────────────────────────────────
    emit_log("info", "calling model…");
    let ai = AiService::new(db.clone())?;
    let config = AiService::get_config(db)?;
    let start = std::time::Instant::now();
    let draft = ai
        .complete_with_prefix(
            &prompt.prefix,
            &prompt.suffix,
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
            prompt.prefix.len() + prompt.suffix.len(),
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
        let body_clean = crate::services::thread_clean::clean_email_body(&body, usize::MAX);
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

/// One message of the thread being answered, with its cleaned body (quotes
/// and signature stripped). Input to [`plan_reply_prompt`].
#[derive(Debug, Clone)]
struct ThreadMessage {
    sender: String,
    sender_email: String,
    subject: String,
    body: String,
}

struct ReplyPromptInput<'a> {
    template: &'a str,
    persona: &'a str,
    style: &'a str,
    /// Oldest first; the last message is the one being answered.
    thread: &'a [ThreadMessage],
    rag_context: &'a str,
    instructions: Option<&'a str>,
}

/// The reply prompt split for the prefix KV cache: `prefix` depends only on
/// the template and the persona/style prefs, so it is identical on every
/// draft; `suffix` carries the per-draft thread, precedents and instructions.
#[derive(Debug, Clone, PartialEq)]
struct ReplyPrompt {
    prefix: String,
    suffix: String,
}

/// Placeholders whose content changes on every draft. The prompt is split
/// at the first of them so the static head can stay cached.
const PER_DRAFT_PLACEHOLDERS: [&str; 3] = ["{thread_context}", "{rag_context}", "{instructions}"];

/// Assemble the reply-draft prompt. Pure.
///
/// The thread is fitted into whatever `MAX_PROMPT_CHARS` leaves after the
/// template, precedents and instructions (never less than
/// `MIN_THREAD_CHARS`), shared out by [`allocate_thread_budget`], so the
/// closing instruction is never truncated away.
fn plan_reply_prompt(input: &ReplyPromptInput<'_>) -> ReplyPrompt {
    let template = input
        .template
        .replace("{persona}", input.persona)
        .replace("{style}", input.style);
    let split = PER_DRAFT_PLACEHOLDERS
        .iter()
        .filter_map(|p| template.find(p))
        .min()
        .unwrap_or(template.len());
    let (prefix, suffix_template) = template.split_at(split);

    let instructions_section = match input.instructions {
        Some(i) if !i.trim().is_empty() => format!("Additional instructions: {}\n\n", i.trim()),
        _ => String::new(),
    };

    let fixed_len = template.len() + input.rag_context.len() + instructions_section.len();
    let overhead = closing_instruction(input.thread.len(), "").len() + THREAD_HEADER_CHARS * input.thread.len();
    let thread_budget = MAX_PROMPT_CHARS
        .saturating_sub(fixed_len + overhead)
        .max(MIN_THREAD_CHARS);
    let thread_context = build_thread_context(input.thread, thread_budget);

    let suffix = suffix_template
        .replace("{thread_context}", &thread_context)
        .replace("{rag_context}", input.rag_context)
        .replace("{instructions}", &instructions_section);

    ReplyPrompt {
        prefix: prefix.to_string(),
        suffix,
    }
}

/// Share `budget` chars across a thread's message bodies (`lens`, oldest
/// first; the last one is being answered). Pure.
///
/// The answered message is served first, up to `TARGET_MSG_MAX_CHARS`. Older
/// messages then split the rest fairly (water-filling: short messages are
/// kept whole, long ones share what remains). When a fair share would drop
/// below `MIN_OLDER_MSG_CHARS`, the oldest messages are dropped (0) instead.
fn allocate_thread_budget(lens: &[usize], budget: usize) -> Vec<usize> {
    let mut alloc = vec![0; lens.len()];
    let Some((&target_len, older)) = lens.split_last() else {
        return alloc;
    };
    let target = target_len.min(TARGET_MSG_MAX_CHARS).min(budget);
    alloc[lens.len() - 1] = target;
    let remaining = budget - target;

    // Keep the newest older messages that still leave each a useful share.
    let mut first_kept = older.len();
    while first_kept > 0 {
        let n = older.len() - first_kept + 1;
        let useful = MIN_OLDER_MSG_CHARS.min(older[first_kept - 1]);
        if remaining / n < useful {
            break;
        }
        first_kept -= 1;
    }

    // Water-fill the kept range: shortest first, each takes min(len, fair share).
    let mut kept: Vec<usize> = (first_kept..older.len()).collect();
    kept.sort_by_key(|&i| older[i]);
    let mut left = remaining;
    let mut slots = kept.len();
    for i in kept {
        let share = left / slots;
        let take = older[i].min(share);
        alloc[i] = take;
        left -= take;
        slots -= 1;
    }
    alloc
}

/// Load each thread message's body, cleaned of quoted history and signature
/// (a reply's quote repeats the whole thread and would eat the budget). Falls
/// back to the list preview when the body is missing or unreadable.
fn load_thread_messages(db: &Database, thread: &[Email]) -> Vec<ThreadMessage> {
    thread
        .iter()
        .map(|msg| {
            let body = match db.get_email_body(&msg.id) {
                Ok(raw) => crate::services::thread_clean::clean_email_body(&raw, usize::MAX),
                Err(e) => {
                    emit_log(
                        "warn",
                        &format!("body unavailable for {} ({}); using preview", msg.id, e),
                    );
                    String::new()
                }
            };
            ThreadMessage {
                sender: msg.sender.clone(),
                sender_email: msg.sender_email.clone(),
                subject: msg.subject.clone(),
                body: if body.trim().is_empty() {
                    msg.snippet.clone()
                } else {
                    body
                },
            }
        })
        .collect()
}

/// The user's own recent messages to the sender of `email`, cleaned — the
/// best evidence of how they greet, sign off and pitch the register with this
/// person. Empty when the user is the sender, or on a DB error (logged).
fn load_style_samples(db: &Database, email: &Email, user_email: &str) -> Vec<String> {
    if user_email.is_empty() || email.sender_email.eq_ignore_ascii_case(user_email) {
        return Vec::new();
    }
    let replies = match db.get_user_replies_to_correspondent(
        &email.account_id,
        &email.sender_email,
        &email.thread_id,
        email.timestamp,
        STYLE_SAMPLES_K * 3,
    ) {
        Ok(r) => r,
        Err(e) => {
            emit_log("warn", &format!("style samples skipped ({})", e));
            return Vec::new();
        }
    };
    replies
        .iter()
        .filter_map(|r| match db.get_email_body(&r.id) {
            Ok(raw) => Some(crate::services::thread_clean::clean_email_body(&raw, usize::MAX)),
            Err(e) => {
                emit_log("warn", &format!("style sample body unavailable for {} ({})", r.id, e));
                None
            }
        })
        .collect()
}

/// Render up to `STYLE_SAMPLES_K` useful samples (newest first) of how the
/// user writes to `correspondent`. Pure.
fn build_style_context(samples: &[String], correspondent: &str) -> String {
    let useful: Vec<&String> = samples
        .iter()
        .filter(|s| s.trim().chars().count() >= MIN_STYLE_SAMPLE_CHARS)
        .take(STYLE_SAMPLES_K)
        .collect();
    if useful.is_empty() {
        return String::new();
    }
    let mut s = format!(
        "\nHow you usually write to {correspondent} (match this greeting, sign-off, register and length; do not copy the content):\n"
    );
    for (i, sample) in useful.iter().enumerate() {
        s.push_str(&format!(
            "\n[Your past message {}]\n{}\n",
            i + 1,
            truncate_chars(sample.trim(), STYLE_SAMPLE_CHARS)
        ));
    }
    s
}

/// Cut an oldest-first thread at the message being replied to.
///
/// Later messages are the future from the draft's point of view: showing them
/// makes the model answer the wrong message, and in `draft_eval` it leaked the
/// user's real reply (the ground truth) into the prompt. An unknown target
/// keeps the whole thread.
fn thread_up_to(mut thread: Vec<Email>, target_id: &str) -> Vec<Email> {
    if let Some(pos) = thread.iter().position(|e| e.id == target_id) {
        thread.truncate(pos + 1);
    }
    thread
}

/// The instruction that closes the thread block: which message to answer and
/// the grounding rules. Never truncated.
fn closing_instruction(message_number: usize, sender: &str) -> String {
    format!(
        "\nWrite a reply to Message {} (from {}). Address the reply to {}. Answer every question and request in it. \
Do not invent facts, prices, dates or commitments that are not in the thread or the \
instructions; write a [placeholder] instead. You cannot attach files or send anything: \
never say something is attached, enclosed or already sent (\"te adjunto\", \"please find \
attached\", \"I've sent the invite\"). Only where the thread asks for a file, write \
[attach: <that file, named as in the thread>] for the user to attach.\n",
        message_number, sender, sender
    )
}

/// Per-message header allowance ("--- Message N ---", From, Subject) when
/// sizing the thread budget.
const THREAD_HEADER_CHARS: usize = 160;

fn build_thread_context(thread: &[ThreadMessage], budget: usize) -> String {
    let Some(last) = thread.last() else {
        return String::new();
    };
    let lens: Vec<usize> = thread.iter().map(|m| m.body.chars().count()).collect();
    let alloc = allocate_thread_budget(&lens, budget);
    let mut s = String::from("Email thread (oldest first):\n");
    let dropped = alloc[..alloc.len() - 1].iter().take_while(|&&a| a == 0).count();
    if dropped > 0 {
        s.push_str(&format!("\n[{dropped} earlier message(s) omitted]\n"));
    }
    for (i, (msg, &chars)) in thread.iter().zip(&alloc).enumerate().skip(dropped) {
        s.push_str(&format!(
            "\n--- Message {} ---\nFrom: {} <{}>\nSubject: {}\n{}\n",
            i + 1,
            msg.sender,
            msg.sender_email,
            msg.subject,
            truncate_chars(&msg.body, chars),
        ));
    }
    s.push_str(&closing_instruction(thread.len(), &last.sender));
    s
}

/// Char-aware cut with a "…" marker so the model knows the text continues.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

fn build_rag_context(sources: &[DraftSource], user_email: &str) -> String {
    if sources.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\nSimilar past threads. These are other conversations with other people: use them only \
for tone and phrasing, never reuse their names, facts or answers, and do not quote them:\n",
    );
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
            references: None,
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

    // ── build_thread_context ───────────────────────────────────────────────

    #[test]
    fn build_thread_context_empty_thread_returns_empty() {
        assert_eq!(build_thread_context(&[], 1_000), "");
    }

    #[test]
    fn build_thread_context_includes_sender_and_subject() {
        let ctx = build_thread_context(&[msg("Alice", "Please review the attached")], 1_000);
        assert!(ctx.contains("Alice"), "sender name must appear in context");
        assert!(ctx.contains("alice@example.com"), "sender email must appear");
        assert!(ctx.contains("Project kickoff"), "subject must appear");
        assert!(ctx.contains("Please review"), "body must appear");
    }

    #[test]
    fn build_thread_context_forbids_claiming_attachments_or_sends() {
        // The draft cannot attach or send anything: "te adjunto…" in a draft
        // goes out with no attachment. It must leave a bracketed note instead.
        let ctx = build_thread_context(&[msg("Alice", "can you send me the form?")], 1_000);
        assert!(ctx.contains("cannot attach files or send anything"));
        assert!(ctx.contains("[attach:"));
    }

    #[test]
    fn build_thread_context_mentions_write_reply_to_latest() {
        let ctx = build_thread_context(&[msg("Alice", "hi"), msg("Bob", "reply here")], 1_000);
        assert!(ctx.contains("Bob"), "latest sender must appear");
        assert!(
            ctx.to_lowercase().contains("reply"),
            "context must prompt the model to write a reply"
        );
    }

    #[test]
    fn build_thread_context_notes_omitted_messages() {
        let mut thread: Vec<ThreadMessage> = (0..10).map(|_| msg("Alice", &"x".repeat(1_000))).collect();
        thread.push(msg("Bob", "latest"));
        let ctx = build_thread_context(&thread, 1_500);
        assert!(ctx.contains("earlier message(s) omitted"));
        assert!(!ctx.contains("--- Message 1 ---"));
        assert!(ctx.contains("--- Message 11 ---"));
    }

    // ── thread_up_to ──────────────────────────────────────────────────────

    fn ids(thread: &[Email]) -> Vec<&str> {
        thread.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn thread_up_to_drops_messages_after_the_target() {
        // Drafting on an older message must not show the model later
        // replies — including the user's own real answer (the eval leak).
        let thread = vec![
            make_email("e1", "Alice", "alice@example.com", "Hi", "one"),
            make_email("e2", "Me", "me@example.com", "Re: Hi", "two"),
            make_email("e3", "Alice", "alice@example.com", "Re: Hi", "three"),
        ];
        assert_eq!(ids(&thread_up_to(thread, "e1")), vec!["e1"]);
    }

    #[test]
    fn thread_up_to_keeps_everything_up_to_and_including_the_target() {
        let thread = vec![
            make_email("e1", "Alice", "alice@example.com", "Hi", "one"),
            make_email("e2", "Me", "me@example.com", "Re: Hi", "two"),
            make_email("e3", "Alice", "alice@example.com", "Re: Hi", "three"),
        ];
        assert_eq!(ids(&thread_up_to(thread, "e3")), vec!["e1", "e2", "e3"]);
    }

    #[test]
    fn thread_up_to_unknown_target_keeps_whole_thread() {
        let thread = vec![
            make_email("e1", "Alice", "alice@example.com", "Hi", "one"),
            make_email("e2", "Me", "me@example.com", "Re: Hi", "two"),
        ];
        assert_eq!(ids(&thread_up_to(thread, "missing")), vec!["e1", "e2"]);
    }

    // ── allocate_thread_budget ────────────────────────────────────────────

    #[test]
    fn allocate_budget_keeps_everything_when_it_fits() {
        assert_eq!(allocate_thread_budget(&[100, 200, 300], 1_000), vec![100, 200, 300]);
    }

    #[test]
    fn allocate_budget_gives_the_target_priority() {
        // The last message is the one being answered: it keeps its full
        // length (up to the target cap) before older messages get anything.
        let alloc = allocate_thread_budget(&[3_000, 3_000], 3_500);
        assert_eq!(alloc[1], 3_000);
        assert_eq!(alloc[0], 500);
    }

    #[test]
    fn allocate_budget_caps_the_target() {
        let alloc = allocate_thread_budget(&[20_000], 50_000);
        assert_eq!(alloc, vec![TARGET_MSG_MAX_CHARS]);
    }

    #[test]
    fn allocate_budget_fair_shares_older_messages() {
        // One huge old message must not starve the short ones.
        let alloc = allocate_thread_budget(&[10_000, 300, 300, 100], 2_000);
        assert_eq!(alloc[3], 100, "target kept whole");
        assert_eq!(alloc[1], 300, "short messages kept whole");
        assert_eq!(alloc[2], 300);
        assert_eq!(alloc[0], 1_300, "the long one gets the remainder");
    }

    #[test]
    fn allocate_budget_drops_oldest_when_shares_get_too_small() {
        // 10 older messages, 1_000 chars left for them: a fair share would be
        // 100 chars each, below the useful minimum — drop the oldest instead.
        let mut lens = vec![1_000; 10];
        lens.push(500);
        let alloc = allocate_thread_budget(&lens, 1_500);
        assert_eq!(alloc[10], 500);
        assert!(alloc[..10].iter().all(|&a| a == 0 || a >= MIN_OLDER_MSG_CHARS));
        assert!(alloc[9] > 0, "newest older message survives");
        assert_eq!(alloc[0], 0, "oldest message dropped");
        assert!(alloc.iter().sum::<usize>() <= 1_500);
    }

    // ── plan_reply_prompt ─────────────────────────────────────────────────

    fn msg(sender: &str, body: &str) -> ThreadMessage {
        ThreadMessage {
            sender: sender.to_string(),
            sender_email: format!("{}@example.com", sender.to_lowercase()),
            subject: "Project kickoff".to_string(),
            body: body.to_string(),
        }
    }

    fn plan(thread: &[ThreadMessage], rag: &str, instructions: Option<&str>) -> ReplyPrompt {
        plan_reply_prompt(&ReplyPromptInput {
            template: DEFAULT_PROMPT_TEMPLATE,
            persona: "a consultant",
            style: "brief",
            thread,
            rag_context: rag,
            instructions,
        })
    }

    #[test]
    fn reply_prompt_prefix_is_stable_across_threads() {
        // The prefix is the KV-cache anchor: it must not depend on the thread.
        let a = plan(&[msg("Alice", "first thread")], "", None);
        let b = plan(&[msg("Bob", "second thread")], "precedent", Some("say yes"));
        assert_eq!(a.prefix, b.prefix);
        assert!(a.prefix.contains("a consultant") && a.prefix.contains("brief"));
        assert!(!a.prefix.contains("first thread"));
    }

    #[test]
    fn reply_prompt_uses_full_bodies_not_previews() {
        let long_body = format!("{} the key question is at the end?", "context ".repeat(80));
        let p = plan(&[msg("Alice", &long_body)], "", None);
        assert!(p.suffix.contains("the key question is at the end?"));
    }

    #[test]
    fn reply_prompt_never_cuts_the_closing_instruction() {
        let huge = "x".repeat(60_000);
        let p = plan(&[msg("Alice", &huge), msg("Bob", &huge)], "", Some("say yes"));
        assert!(p.suffix.trim_end().ends_with("no signature):"));
        assert!(p.suffix.contains("say yes"));
        assert!(p.prefix.len() + p.suffix.len() <= MAX_PROMPT_CHARS + 500);
    }

    #[test]
    fn reply_prompt_names_the_message_to_answer() {
        let p = plan(&[msg("Alice", "hi"), msg("Bob", "can we meet?")], "", None);
        assert!(p.suffix.contains("Message 2"));
        assert!(p.suffix.contains("from Bob"));
    }

    #[test]
    fn reply_prompt_custom_template_without_placeholders_is_all_prefix() {
        let p = plan_reply_prompt(&ReplyPromptInput {
            template: "Just write something nice.",
            persona: "p",
            style: "s",
            thread: &[msg("Alice", "hi")],
            rag_context: "",
            instructions: None,
        });
        assert_eq!(format!("{}{}", p.prefix, p.suffix), "Just write something nice.");
    }

    // ── build_style_context ───────────────────────────────────────────────

    #[test]
    fn style_context_empty_without_samples() {
        assert_eq!(build_style_context(&[], "Ana"), "");
    }

    #[test]
    fn style_context_skips_too_short_samples() {
        // "Ok, gracias" says nothing about how the user writes.
        let samples = vec!["Ok, gracias".to_string()];
        assert_eq!(build_style_context(&samples, "Ana"), "");
    }

    #[test]
    fn style_context_names_the_correspondent_and_keeps_k_samples() {
        let samples: Vec<String> = (1..=5)
            .map(|i| format!("Hola Ana, sample number {i} with enough words to be useful. Un abrazo"))
            .collect();
        let ctx = build_style_context(&samples, "Ana");
        assert!(ctx.contains("to Ana"));
        assert!(ctx.contains("sample number 1") && ctx.contains("sample number 2"));
        assert!(!ctx.contains(&format!("sample number {}", STYLE_SAMPLES_K + 1)));
    }

    #[test]
    fn style_context_caps_each_sample() {
        let samples = vec!["palabra ".repeat(1_000)];
        let ctx = build_style_context(&samples, "Ana");
        assert!(ctx.chars().count() < STYLE_SAMPLE_CHARS + 300);
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
    fn build_rag_context_marks_precedents_as_other_people() {
        // A precedent greeting "Hi Rafael" leaked into a reply to Nadia: the
        // label must say these are other conversations, for tone only.
        let ctx = build_rag_context(&[make_source("me@example.com")], "me@example.com");
        assert!(ctx.contains("other conversations with other people"));
        assert!(ctx.contains("never reuse their names"));
    }

    #[test]
    fn closing_instruction_names_who_to_greet() {
        let closing = closing_instruction(2, "Nadia Brunner");
        assert!(closing.contains("Address the reply to Nadia Brunner"));
    }

    #[test]
    fn build_rag_context_includes_subject_and_snippet() {
        let src = make_source("client@corp.com");
        let ctx = build_rag_context(&[src], "me@example.com");
        assert!(ctx.contains("Test Subject"), "subject must appear in RAG context");
        assert!(ctx.contains("Test snippet"), "snippet must appear in RAG context");
    }
}
