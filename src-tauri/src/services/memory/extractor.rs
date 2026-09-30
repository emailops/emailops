//! Per-email memory fact extraction.
//!
//! This module owns only durable fact extraction. Pending tasks and thread
//! awaiting-reply state live in `services::tasks`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::services::app_handle::AppHandle;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::ai::provider::CompletionOptions;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Email, MemoryFact};
use crate::services::ai::AiService;
use crate::services::memory::config::MemoryConfig;

pub use crate::services::tasks::extractor::ExtractedTask;
use crate::services::tasks::extractor::{lenient_items, reply_object};

const MAX_BODY_CHARS: usize = 1500;

pub async fn extract_new_emails(db: &Arc<Database>, app: &AppHandle, account_id: &str) -> Result<u32> {
    if !db.is_ai_enabled()? {
        emit_log(
            app,
            "info",
            "memory",
            "Skipped: AI is disabled in settings (master switch off)",
        );
        return Ok(0);
    }
    let cfg = crate::services::memory::config::get_config(db)?;
    if !cfg.enabled {
        emit_log(app, "info", "memory", "Skipped: memory extraction is disabled");
        return Ok(0);
    }
    extract_batch(db, app, account_id, &cfg, None).await
}

pub async fn extract_batch(
    db: &Arc<Database>,
    app: &AppHandle,
    account_id: &str,
    cfg: &MemoryConfig,
    cancel: Option<&AtomicBool>,
) -> Result<u32> {
    if !cfg.enabled {
        return Ok(0);
    }
    let ids = db.get_memory_unextracted_email_ids(account_id, cfg.backfill_batch_size, &cfg.categories, None)?;
    if ids.is_empty() {
        return Ok(0);
    }

    let owner_email = match db.get_account(account_id)? {
        Some(a) => a.email.to_ascii_lowercase(),
        None => {
            emit_log(
                app,
                "warn",
                "memory",
                &format!("extractor: account {account_id} not found; skipping"),
            );
            return Ok(0);
        }
    };

    emit_log(
        app,
        "info",
        "memory",
        &format!("Extracting memories from {} new emails", ids.len()),
    );
    let ai = match AiService::new(db.clone()) {
        Ok(svc) => svc,
        Err(e) => {
            emit_log(
                app,
                "warn",
                "memory",
                &format!("memory extractor disabled (no AI provider): {e}"),
            );
            return Ok(0);
        }
    };

    let mut ok = 0;
    for email_id in &ids {
        // Stopped by its own Cancel button, or from the queue because the AI
        // provider or model is about to change.
        if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) || crate::services::task_queue::cancel_requested() {
            emit_log(app, "info", "memory", "Memory extraction cancelled mid-batch");
            break;
        }
        match process_email(db, &ai, &owner_email, email_id, cfg).await {
            Ok(true) => ok += 1,
            Ok(false) => {}
            Err(e) => emit_log(
                app,
                "warn",
                "memory",
                &format!("memory extractor skipped {email_id}: {e}"),
            ),
        }
    }
    emit_log(
        app,
        "success",
        "memory",
        &format!("Extracted memories for {ok}/{} emails", ids.len()),
    );
    Ok(ok)
}

async fn process_email(
    db: &Arc<Database>,
    ai: &AiService,
    owner_email: &str,
    email_id: &str,
    cfg: &MemoryConfig,
) -> Result<bool> {
    let Some(email) = db.get_email(email_id)? else {
        return Ok(true);
    };
    let is_self_authored = !owner_email.is_empty() && email.sender_email.eq_ignore_ascii_case(owner_email);
    let email_tags = db.get_email_tags(email_id).unwrap_or_default();
    let tag_values: Vec<&str> = email_tags.iter().map(|t| t.tag_value.as_str()).collect();
    let tag_skip = !cfg.excluded_tags.is_empty() && cfg.is_tag_excluded(tag_values.iter().copied());
    let skip = cfg.is_sender_excluded(&email.sender_email)
        || tag_skip
        || looks_like_calendar_invite(db, &email)
        || (cfg.extract_from_self_only && !is_self_authored);

    if skip {
        db.mark_memory_facts_extracted(email_id, Utc::now().timestamp())?;
        return Ok(false);
    }

    // A failed call or unparseable reply propagates without marking the
    // email, so the next batch retries it (the caller logs it at warn).
    let extracted = run_llm_extraction(db, ai, &email).await?;
    write_extraction(
        db,
        &email,
        &extracted,
        derive_company_tag(&email.recipients, &email.cc, owner_email).as_deref(),
    )?;
    db.mark_memory_facts_extracted(email_id, Utc::now().timestamp())?;
    Ok(true)
}

#[derive(Debug, Default, Deserialize, serde::Serialize, Clone)]
pub struct ExtractedPayload {
    #[serde(default)]
    pub tasks: Vec<ExtractedTask>,
    #[serde(default)]
    pub facts: Vec<ExtractedFact>,
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct ExtractedFact {
    #[serde(alias = "subjectKind")]
    pub subject_kind: String,
    #[serde(alias = "subjectKey")]
    pub subject_key: String,
    pub fact: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub vigency: Option<String>,
}

pub async fn extract_for_eval(db: &Arc<Database>, ai: &AiService, email: &Email) -> Result<ExtractedPayload> {
    run_llm_extraction(db, ai, email).await
}

async fn run_llm_extraction(db: &Arc<Database>, ai: &AiService, email: &Email) -> Result<ExtractedPayload> {
    let prompt = build_prompt(db, email)?;
    let res = ai
        .complete(
            &prompt,
            "memory_extraction",
            Some(CompletionOptions {
                temperature: Some(0.1),
                max_tokens: Some(400),
                think: None,
                json_shape: None,
            }),
        )
        .await?;
    parse_payload(&res).ok_or_else(|| AppError::AiError("memory extraction reply held no JSON object".into()))
}

fn parse_payload(raw: &str) -> Option<ExtractedPayload> {
    let obj = reply_object(raw)?;
    Some(ExtractedPayload {
        tasks: lenient_items(&obj, "tasks", "memory"),
        facts: lenient_items(&obj, "facts", "memory"),
    })
}

fn build_prompt(db: &Arc<Database>, email: &Email) -> Result<String> {
    let raw = db.get_email_body(&email.id).unwrap_or_default();
    // What this email adds to its thread — not its quoted history, which the
    // earlier messages were already read for.
    let body = crate::services::thread_reader::message_new_content(db, email, &raw);
    let body_trimmed = truncate_utf8(&body, MAX_BODY_CHARS);
    let snippet = if body_trimmed.is_empty() {
        email.snippet.as_str()
    } else {
        body_trimmed
    };
    let language = crate::services::i18n::resolve_ai_language(db)?;
    let language_clause = format!(
        "\nOutput language: write ALL natural-language fields (fact text) in {lang}. Keep structural values like subjectKind, domain, and vigency exactly as specified.\n",
        lang = language.english_name(),
    );
    let template = crate::services::prompts::get_template(db, "memory.extract_facts")?;
    let mut vars: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    vars.insert("today", Utc::now().format("%Y-%m-%d").to_string());
    vars.insert("language_clause", language_clause);
    vars.insert("sender", email.sender.clone());
    vars.insert("sender_email", email.sender_email.clone());
    vars.insert("subject", email.subject.clone());
    vars.insert("snippet", snippet.to_string());
    Ok(crate::services::prompts::render(&template, &vars))
}

fn write_extraction(
    db: &Arc<Database>,
    email: &Email,
    extracted: &ExtractedPayload,
    company: Option<&str>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    for fact in &extracted.facts {
        let text = fact.fact.trim();
        if text.is_empty() || is_silly_fact(text) {
            continue;
        }
        let kind = normalize_kind(&fact.subject_kind);
        let key = fact.subject_key.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let confidence = fact.confidence.unwrap_or(0.6).clamp(0.0, 1.0);
        let existing = db
            .get_memory_facts_by_subject(&email.account_id, &kind, &key)
            .unwrap_or_default();
        if let Some(dup) = existing
            .iter()
            .find(|f| super::consolidation::texts_equivalent(&f.fact, text))
        {
            let delta = (confidence * 0.2).max(0.05);
            db.bump_memory_fact_score(&dup.id, delta, now)?;
            continue;
        }

        let row = MemoryFact {
            id: Uuid::new_v4().to_string(),
            account_id: email.account_id.clone(),
            subject_kind: kind,
            subject_key: key,
            fact: text.to_string(),
            source: "extraction".to_string(),
            source_email_id: Some(email.id.clone()),
            confidence,
            score: confidence,
            status: "candidate".to_string(),
            last_used_at: None,
            domain: normalize_domain(fact.domain.as_deref()),
            vigency: normalize_vigency(fact.vigency.as_deref()),
            company: company.map(|c| c.to_string()),
            created_at: now,
            updated_at: now,
        };
        db.insert_memory_fact(&row)?;
    }
    Ok(())
}

/// Invites are recognised structurally — a calendar part attached to the
/// email, iCalendar content in the body, a calendar-notifier sender or an
/// invite subject prefix — never by a meeting link, which ordinary mail
/// carries in signatures.
fn looks_like_calendar_invite(db: &Arc<Database>, email: &Email) -> bool {
    if subject_looks_like_invite(&email.subject) || sender_is_calendar_notifier(&email.sender_email) {
        return true;
    }
    match db.get_email_attachment_metas(&email.id) {
        Ok(metas)
            if metas
                .iter()
                .any(crate::services::calendar::invite::is_invite_attachment) =>
        {
            return true
        }
        Ok(_) => {}
        Err(e) => crate::services::logger::log(
            "warn",
            "memory",
            format!("invite check: attachment lookup failed for {}: {e}", email.id),
        ),
    }
    match db.get_email_body(&email.id) {
        Ok(body) => body_looks_like_invite(body.get(..body.len().min(8192)).unwrap_or(&body)),
        Err(e) => {
            crate::services::logger::log(
                "warn",
                "memory",
                format!("invite check: body lookup failed for {}: {e}", email.id),
            );
            false
        }
    }
}

pub fn looks_like_calendar_invite_parts(subject: &str, sender_email: &str, body_head: &str) -> bool {
    subject_looks_like_invite(subject) || sender_is_calendar_notifier(sender_email) || body_looks_like_invite(body_head)
}

fn subject_looks_like_invite(subject: &str) -> bool {
    let s = subject.trim().to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        "invitation:",
        "updated invitation:",
        "canceled event:",
        "cancelled event:",
        "canceled:",
        "cancelled:",
        "declined:",
        "accepted:",
        "tentative:",
        "tentatively accepted:",
        "new event:",
        "event reminder:",
        "invitación:",
        "invitación actualizada:",
        "evento cancelado:",
        "invitation :",
        "invitation mise à jour :",
        "événement annulé :",
        "einladung:",
        "aktualisierte einladung:",
        "abgesagter termin:",
        "convite:",
        "convite atualizado:",
    ];
    PREFIXES.iter().any(|p| s.starts_with(p))
}

fn sender_is_calendar_notifier(sender_email: &str) -> bool {
    let s = sender_email.trim().to_ascii_lowercase();
    s == "calendar-notification@google.com"
        || s.ends_with("@calendar-notification.google.com")
        || s.starts_with("noreply-calendar@")
}

/// iCalendar content or MIME markers inlined in the body text.
fn body_looks_like_invite(body: &str) -> bool {
    body.contains("BEGIN:VCALENDAR")
        || body.contains("text/calendar")
        || body.contains("METHOD:REQUEST")
        || body.contains("METHOD:CANCEL")
        || body.contains("METHOD:REPLY")
}

pub(super) fn derive_company_tag(recipients: &[String], cc: &[String], owner_email: &str) -> Option<String> {
    let owner_domain = crate::util::email_addr::extract_domain(owner_email);
    // domain -> (count, lex-smallest contributing address)
    let mut buckets: std::collections::HashMap<String, (u32, String)> = std::collections::HashMap::new();
    for addr in recipients.iter().chain(cc.iter()) {
        let Some(domain) = crate::util::email_addr::extract_domain(addr) else {
            continue;
        };
        if owner_domain.as_deref() == Some(domain.as_str()) {
            continue;
        }
        let (_, addr_only) = crate::util::email_addr::split_name_addr(addr);
        let addr_only = if addr_only.is_empty() {
            addr.trim().to_ascii_lowercase()
        } else {
            addr_only
        };
        buckets
            .entry(domain)
            .and_modify(|(c, anchor)| {
                *c += 1;
                if addr_only < *anchor {
                    *anchor = addr_only.clone();
                }
            })
            .or_insert((1, addr_only));
    }
    let (domain, (_, anchor)) = buckets
        .into_iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| b.0.cmp(&a.0)))?;
    Some(crate::util::email_addr::company_label_for(&domain, Some(&anchor)))
}

fn normalize_kind(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "user" => "user".to_string(),
        "contact" => "contact".to_string(),
        "domain" => "domain".to_string(),
        "project" => "project".to_string(),
        _ => "contact".to_string(),
    }
}

fn normalize_domain(raw: Option<&str>) -> Option<String> {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("personal") => Some("personal".to_string()),
        Some("professional") => Some("professional".to_string()),
        _ => None,
    }
}

fn normalize_vigency(raw: Option<&str>) -> Option<String> {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("atemporal") => Some("atemporal".to_string()),
        Some("deciduous") => Some("deciduous".to_string()),
        _ => None,
    }
}

fn is_silly_fact(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }
    const BAD_PREFIXES: &[&str] = &[
        "email was ",
        "email is ",
        "the email was ",
        "the email is ",
        "this email was ",
        "this email is ",
        "email sent ",
        "the email sent ",
        "email from ",
        "email to ",
        "subject is ",
        "subject was ",
        "the subject is ",
        "the subject of ",
        "sender is ",
        "sender was ",
        "recipient is ",
        "sent by ",
        "sent from ",
        "sent to ",
        "received from ",
        "received by ",
        "the message ",
        "this message ",
    ];
    if BAD_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    const BAD_EXACT: &[&str] = &[
        "email exists",
        "the email exists",
        "this is an email",
        "email received",
        "message received",
    ];
    BAD_EXACT.iter().any(|s| lower == *s)
}

use crate::util::text::truncate_utf8;

fn emit_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[cfg(test)]
mod thread_tests {
    use super::*;
    use crate::services::thread_reader::fixtures;

    #[test]
    fn the_prompt_reads_a_replys_new_content_not_its_quoted_history() {
        // Reading the quote made the extractor learn the request's facts
        // again from every reply.
        let db = Arc::new(Database::new_for_testing().unwrap());
        fixtures::seed_quoting_thread(&db);
        let email = db.get_email_by_id("e2").unwrap().expect("e2");
        let prompt = build_prompt(&db, &email).unwrap();
        assert!(prompt.contains(fixtures::REPLY_NEW), "{prompt}");
        assert!(!prompt.contains(fixtures::REQUEST), "{prompt}");
    }
}

#[cfg(test)]
mod invite_tests {
    use super::*;
    use crate::services::thread_reader::fixtures;

    fn seeded() -> Arc<Database> {
        let db = Arc::new(Database::new_for_testing().unwrap());
        fixtures::seed_quoting_thread(&db);
        db
    }

    /// A meeting link in a signature is not an invitation: the email must
    /// still be read for facts/tasks.
    #[test]
    fn a_meeting_link_in_the_body_is_not_an_invite() {
        let db = seeded();
        db.connection()
            .execute(
                "UPDATE email_bodies SET body = 'Please send the budget by Friday.\n--\nMy room: https://zoom.us/j/123456 | https://meet.google.com/abc-defg-hij' WHERE email_id = 'e1'",
                [],
            )
            .unwrap();
        let email = db.get_email_by_id("e1").unwrap().unwrap();
        assert!(!looks_like_calendar_invite(&db, &email));
    }

    #[test]
    fn a_calendar_attachment_marks_an_invite() {
        let db = seeded();
        db.insert_email_attachment_meta("e1", "acct", "att-1", "invite.ics", "text/calendar", 512)
            .unwrap();
        let email = db.get_email_by_id("e1").unwrap().unwrap();
        assert!(looks_like_calendar_invite(&db, &email));
    }
}

#[cfg(test)]
mod process_tests {
    //! An email is only marked fact-extracted after a successful model call
    //! and parse — otherwise it is never retried.
    use super::*;
    use crate::ai::provider::FakeAiProvider;
    use crate::services::thread_reader::fixtures;

    fn setup() -> (Arc<Database>, Arc<FakeAiProvider>, AiService, MemoryConfig) {
        let db = Arc::new(Database::new_for_testing().unwrap());
        fixtures::seed_quoting_thread(&db);
        let fake = Arc::new(FakeAiProvider::new());
        let ai = AiService::with_provider(db.clone(), fake.clone());
        let cfg = MemoryConfig {
            extract_from_self_only: false,
            ..MemoryConfig::default()
        };
        (db, fake, ai, cfg)
    }

    fn is_pending(db: &Database, id: &str) -> bool {
        db.get_memory_unextracted_email_ids("acct", 50, &[], None)
            .unwrap()
            .iter()
            .any(|e| e == id)
    }

    #[tokio::test]
    async fn a_provider_error_leaves_the_email_queued_for_retry() {
        let (db, fake, ai, cfg) = setup();
        fake.fail_completions(Some("model offline"));
        let res = process_email(&db, &ai, "me@example.com", "e1", &cfg).await;
        assert!(res.is_err());
        assert!(is_pending(&db, "e1"));
    }

    #[tokio::test]
    async fn an_unparseable_reply_leaves_the_email_queued_for_retry() {
        let (db, fake, ai, cfg) = setup();
        for reply in ["", "Sorry, I cannot help with that."] {
            fake.push_completion(reply);
            let res = process_email(&db, &ai, "me@example.com", "e1", &cfg).await;
            assert!(res.is_err(), "reply {reply:?}");
            assert!(is_pending(&db, "e1"), "reply {reply:?}");
        }
    }

    #[tokio::test]
    async fn one_malformed_fact_does_not_drop_the_others() {
        let (db, fake, ai, cfg) = setup();
        fake.push_completion(
            r#"{"facts":[
                {"subjectKind":"contact","subjectKey":"ana@example.com","fact":"Ana leads the portal project"},
                {"subjectKind":"contact","fact":42},
                {"subjectKind":"contact","subjectKey":"ana@example.com","fact":"Ana prefers written updates"}
            ]}"#,
        );
        assert!(process_email(&db, &ai, "me@example.com", "e1", &cfg).await.unwrap());
        assert!(!is_pending(&db, "e1"));
        let facts = db
            .get_memory_facts_by_subject("acct", "contact", "ana@example.com")
            .unwrap();
        assert_eq!(facts.len(), 2, "{facts:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_invite_detection() {
        assert!(subject_looks_like_invite("Invitation: Team sync @ 3pm"));
        assert!(subject_looks_like_invite("Invitación: Reunión semanal"));
        assert!(!subject_looks_like_invite("Please send invoice"));
    }

    #[test]
    fn is_silly_fact_rejects_envelope_restatements() {
        assert!(is_silly_fact("Email was sent by alice@acme.com"));
        assert!(is_silly_fact("Subject is Invoice reminder"));
        assert!(!is_silly_fact("Alice prefers morning calls"));
    }

    #[test]
    fn derive_company_tag_picks_most_frequent_non_owner_domain() {
        let tag = derive_company_tag(
            &["alice@acme.com".into(), "bob@acme.com".into(), "carol@other.io".into()],
            &[],
            "me@mine.com",
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }
}
