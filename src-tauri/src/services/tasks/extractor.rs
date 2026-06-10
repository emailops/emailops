//! Per-email task extraction.
//!
//! This pipeline owns pending-task creation and thread follow-up state. It is
//! intentionally independent from memory fact extraction: it reads/writes
//! the `tasks` pipeline status, uses `TaskConfig`, and never consumes the
//! memory-fact backlog.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;
use tauri::AppHandle;
use uuid::Uuid;

use crate::ai::provider::CompletionOptions;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Email, PendingTask, ThreadState};
use crate::services::ai::AiService;
use crate::services::tasks::config::TaskConfig;
use crate::util::text::truncate_utf8;

const MAX_BODY_CHARS: usize = 1500;

pub async fn extract_new_emails(db: &Arc<Database>, app: &AppHandle, account_id: &str) -> Result<u32> {
    if !db.is_ai_enabled()? {
        emit_log(
            app,
            "info",
            "tasks",
            "Skipped: AI is disabled in settings (master switch off)",
        );
        return Ok(0);
    }
    let cfg = crate::services::tasks::config::get_config(db)?;
    if !cfg.enabled {
        emit_log(app, "info", "tasks", "Skipped: task extraction is disabled");
        return Ok(0);
    }
    extract_batch(db, app, account_id, &cfg, None).await
}

pub async fn extract_batch(
    db: &Arc<Database>,
    app: &AppHandle,
    account_id: &str,
    cfg: &TaskConfig,
    cancel: Option<&AtomicBool>,
) -> Result<u32> {
    if !cfg.enabled {
        return Ok(0);
    }

    let min_ts = cfg.backfill_min_timestamp(Utc::now().timestamp());
    let ids = db.get_task_unextracted_email_ids(account_id, 50, &cfg.categories, min_ts)?;
    if ids.is_empty() {
        return Ok(0);
    }

    let owner_email = match db.get_account(account_id)? {
        Some(a) => a.email.to_ascii_lowercase(),
        None => {
            emit_log(
                app,
                "warn",
                "tasks",
                &format!("extractor: account {account_id} not found; skipping"),
            );
            return Ok(0);
        }
    };

    emit_log(
        app,
        "info",
        "tasks",
        &format!("Extracting tasks from {} new emails", ids.len()),
    );

    let ai = match AiService::new(db.clone()) {
        Ok(svc) => svc,
        Err(e) => {
            emit_log(
                app,
                "warn",
                "tasks",
                &format!("task extractor disabled (no AI provider): {e}"),
            );
            return run_heuristic_only(db, app, &owner_email, &ids, cfg).await;
        }
    };

    let mut ok = 0;
    for email_id in &ids {
        if let Some(c) = cancel {
            if c.load(Ordering::SeqCst) {
                emit_log(app, "info", "tasks", "Task extraction cancelled mid-batch");
                break;
            }
        }
        match process_email(db, app, &ai, &owner_email, email_id, cfg).await {
            Ok(true) => ok += 1,
            Ok(false) => {}
            Err(e) => emit_log(app, "warn", "tasks", &format!("task extractor skipped {email_id}: {e}")),
        }
    }

    emit_log(
        app,
        "success",
        "tasks",
        &format!("Extracted tasks for {ok}/{} emails", ids.len()),
    );
    Ok(ok)
}

async fn run_heuristic_only(
    db: &Arc<Database>,
    app: &AppHandle,
    owner_email: &str,
    email_ids: &[String],
    cfg: &TaskConfig,
) -> Result<u32> {
    let mut ok = 0;
    for id in email_ids {
        let Some(email) = db.get_email(id)? else {
            continue;
        };
        if let Err(e) = apply_heuristic_thread_update(db, &email, owner_email) {
            emit_log(app, "warn", "tasks", &format!("heuristic update failed for {id}: {e}"));
            continue;
        }
        if cfg.is_sender_excluded(&email.sender_email) {
            db.mark_tasks_extracted(id, Utc::now().timestamp())?;
            continue;
        }
        db.mark_tasks_extracted(id, Utc::now().timestamp())?;
        ok += 1;
    }
    Ok(ok)
}

async fn process_email(
    db: &Arc<Database>,
    app: &AppHandle,
    ai: &AiService,
    owner_email: &str,
    email_id: &str,
    cfg: &TaskConfig,
) -> Result<bool> {
    let Some(email) = db.get_email(email_id)? else {
        return Ok(true);
    };

    if let Err(e) = apply_heuristic_thread_update(db, &email, owner_email) {
        emit_log(
            app,
            "warn",
            "tasks",
            &format!("heuristic update failed for {email_id}: {e}"),
        );
    }

    let is_self_authored = !owner_email.is_empty() && email.sender_email.eq_ignore_ascii_case(owner_email);
    let email_tags = db.get_email_tags(email_id).unwrap_or_default();
    let tag_values: Vec<&str> = email_tags.iter().map(|t| t.tag_value.as_str()).collect();
    let tag_skip = !cfg.excluded_tags.is_empty() && cfg.is_tag_excluded(tag_values.iter().copied());
    let skip = cfg.is_sender_excluded(&email.sender_email)
        || tag_skip
        || looks_like_calendar_invite(db, &email)
        || (cfg.extract_from_self_only && !is_self_authored);

    if skip {
        db.mark_tasks_extracted(email_id, Utc::now().timestamp())?;
        return Ok(false);
    }

    match run_llm_extraction(db, ai, &email, cfg).await {
        Ok(extracted) => write_extraction(
            db,
            &email,
            &extracted,
            derive_company_tag(&email.recipients, &email.cc, owner_email).as_deref(),
        )?,
        Err(e) => {
            emit_log(
                app,
                "debug",
                "tasks",
                &format!("LLM task extraction skipped for {email_id}: {e}"),
            );
        }
    }
    db.mark_tasks_extracted(email_id, Utc::now().timestamp())?;
    Ok(true)
}

#[derive(Debug, Default, Deserialize, serde::Serialize, Clone)]
pub struct ExtractedTasksPayload {
    #[serde(default)]
    pub tasks: Vec<ExtractedTask>,
    #[serde(default, alias = "threadSummary")]
    pub thread_summary: Option<String>,
    #[serde(default)]
    pub commitment: Option<String>,
    #[serde(default, alias = "deadlineIso")]
    pub deadline_iso: Option<String>,
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct ExtractedTask {
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default, alias = "dueAtIso", alias = "dueIso")]
    pub due_at_iso: Option<String>,
}

pub async fn extract_for_eval(db: &Arc<Database>, ai: &AiService, email: &Email) -> Result<ExtractedTasksPayload> {
    let cfg = crate::services::tasks::config::get_config(db)?;
    run_llm_extraction(db, ai, email, &cfg).await
}

async fn run_llm_extraction(
    db: &Arc<Database>,
    ai: &AiService,
    email: &Email,
    cfg: &TaskConfig,
) -> Result<ExtractedTasksPayload> {
    let existing_titles: Vec<String> = db
        .list_pending_tasks(&email.account_id, Some("open"), None, 50)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.source_thread_id.as_deref() == Some(email.thread_id.as_str()))
        .take(8)
        .map(|t| t.title)
        .collect();
    let prompt = build_prompt(db, email, cfg, &existing_titles)?;
    let res = ai
        .complete(
            &prompt,
            "tasks_extraction",
            Some(CompletionOptions {
                temperature: Some(0.1),
                max_tokens: Some(500),
                think: None,
            }),
        )
        .await?;
    let mut payload: ExtractedTasksPayload = parse_json_subset(&res);
    if cfg.max_tasks_per_email > 0 && (payload.tasks.len() as i32) > cfg.max_tasks_per_email {
        payload.tasks.truncate(cfg.max_tasks_per_email as usize);
    }
    Ok(payload)
}

fn build_prompt(
    db: &Arc<Database>,
    email: &Email,
    cfg: &TaskConfig,
    existing_thread_tasks: &[String],
) -> Result<String> {
    let body = db.get_email_body(&email.id).unwrap_or_default();
    let body_trimmed = truncate_utf8(&body, MAX_BODY_CHARS);
    let snippet = if body_trimmed.is_empty() {
        email.snippet.as_str()
    } else {
        body_trimmed
    };
    let max_tasks_clause = if cfg.max_tasks_per_email > 0 {
        format!(
            "- Emit AT MOST {n} task{plural} — pick the single most important action if there are several.\n",
            n = cfg.max_tasks_per_email,
            plural = if cfg.max_tasks_per_email == 1 { "" } else { "s" },
        )
    } else {
        String::new()
    };
    let dedup_block = if existing_thread_tasks.is_empty() {
        String::new()
    } else {
        let mut block = String::from(
            "\nThis email belongs to a thread that already has these open tasks (do NOT duplicate them):\n",
        );
        for t in existing_thread_tasks {
            block.push_str("- ");
            block.push_str(t);
            block.push('\n');
        }
        block
    };
    let language = crate::services::i18n::resolve_ai_language(db)?;
    let language_clause = format!(
        "\nOutput language: write ALL natural-language fields (task titles, task details, threadSummary, commitment) in {lang}. Keep structural values like priority and ISO dates exactly as specified.\n",
        lang = language.english_name(),
    );
    let template = if db.get_preference("prompt.tasks.extract")?.is_some() {
        crate::services::prompts::get_template(db, "tasks.extract")?
    } else if db.get_preference("prompt.memory.extract_tasks")?.is_some() {
        crate::services::prompts::get_template(db, "memory.extract_tasks")?
    } else {
        crate::services::prompts::get_template(db, "tasks.extract")?
    };
    let mut vars: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    vars.insert("today", Utc::now().format("%Y-%m-%d").to_string());
    vars.insert("language_clause", language_clause);
    vars.insert("max_tasks_clause", max_tasks_clause);
    vars.insert("dedup_block", dedup_block);
    vars.insert("sender", email.sender.clone());
    vars.insert("sender_email", email.sender_email.clone());
    vars.insert("subject", email.subject.clone());
    vars.insert("snippet", snippet.to_string());
    Ok(crate::services::prompts::render(&template, &vars))
}

fn write_extraction(
    db: &Arc<Database>,
    email: &Email,
    extracted: &ExtractedTasksPayload,
    company: Option<&str>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    // Email/thread summarization is intentionally disabled for the time
    // being to reduce AI processing on large mailboxes. The LLM may still
    // return a `thread_summary` field, but we do not persist it; only the
    // commitment + deadline (which drive Tasks / "waiting on" state) are
    // written to `thread_states`. Re-enable by restoring the
    // `state.summary = pick_non_empty(...)` line below.
    if extracted.commitment.is_some() || extracted.deadline_iso.is_some() {
        let mut state = db
            .get_thread_state(&email.account_id, &email.thread_id)?
            .unwrap_or_else(|| empty_thread_state(email, now));
        state.commitment = pick_non_empty(&extracted.commitment).or(state.commitment);
        if let Some(ts) = extracted.deadline_iso.as_deref().and_then(parse_iso_ts) {
            state.deadline_at = Some(ts);
        }
        state.updated_at = now;
        db.upsert_thread_state(&state)?;
    }

    for task in &extracted.tasks {
        let title = task.title.trim();
        if title.is_empty() {
            continue;
        }
        let priority = task
            .priority
            .as_deref()
            .map(|p| match p.to_ascii_lowercase().as_str() {
                "low" | "normal" | "high" => p.to_ascii_lowercase(),
                _ => "normal".to_string(),
            })
            .unwrap_or_else(|| "normal".to_string());
        let row = PendingTask {
            id: Uuid::new_v4().to_string(),
            account_id: email.account_id.clone(),
            title: title.to_string(),
            detail: task.detail.clone().filter(|s| !s.trim().is_empty()),
            source: "extracted".to_string(),
            source_email_id: Some(email.id.clone()),
            source_thread_id: Some(email.thread_id.clone()),
            assignee: "me".to_string(),
            status: "open".to_string(),
            priority,
            due_at: task.due_at_iso.as_deref().and_then(parse_iso_ts),
            completed_at: None,
            company: company.map(|c| c.to_string()),
            created_at: now,
            updated_at: now,
        };
        db.insert_pending_task(&row)?;
    }
    Ok(())
}

fn apply_heuristic_thread_update(db: &Arc<Database>, email: &Email, owner_email: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    let sender_is_owner = email.sender_email.eq_ignore_ascii_case(owner_email);
    let current = db.get_thread_state(&email.account_id, &email.thread_id)?;
    let awaiting = if sender_is_owner { "them" } else { "user" };
    let mut participants: Vec<String> = current.as_ref().map(|s| s.participants.clone()).unwrap_or_default();
    add_unique(&mut participants, &email.sender_email);
    for r in &email.recipients {
        add_unique(&mut participants, r);
    }
    for r in &email.cc {
        add_unique(&mut participants, r);
    }
    let last_inbound_at = if sender_is_owner {
        current.as_ref().and_then(|s| s.last_inbound_at)
    } else {
        Some(
            email
                .timestamp
                .max(current.as_ref().and_then(|s| s.last_inbound_at).unwrap_or(0)),
        )
    };
    let last_outbound_at = if sender_is_owner {
        Some(
            email
                .timestamp
                .max(current.as_ref().and_then(|s| s.last_outbound_at).unwrap_or(0)),
        )
    } else {
        current.as_ref().and_then(|s| s.last_outbound_at)
    };
    db.upsert_thread_state(&ThreadState {
        account_id: email.account_id.clone(),
        thread_id: email.thread_id.clone(),
        awaiting: awaiting.to_string(),
        last_inbound_at,
        last_outbound_at,
        last_touched_at: now,
        summary: current.as_ref().and_then(|s| s.summary.clone()),
        commitment: current.as_ref().and_then(|s| s.commitment.clone()),
        deadline_at: current.as_ref().and_then(|s| s.deadline_at),
        participants,
        updated_at: now,
    })
}

fn add_unique(list: &mut Vec<String>, email: &str) {
    let normalized = email.trim().to_ascii_lowercase();
    if !normalized.is_empty() && !list.iter().any(|e| e.eq_ignore_ascii_case(&normalized)) {
        list.push(normalized);
    }
}

fn looks_like_calendar_invite(db: &Arc<Database>, email: &Email) -> bool {
    if subject_looks_like_invite(&email.subject) || sender_is_calendar_notifier(&email.sender_email) {
        return true;
    }
    if let Ok(body) = db.get_email_body(&email.id) {
        return body_looks_like_invite(body.get(..body.len().min(8192)).unwrap_or(&body));
    }
    false
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

fn body_looks_like_invite(body: &str) -> bool {
    if body.contains("BEGIN:VCALENDAR")
        || body.contains("text/calendar")
        || body.contains("METHOD:REQUEST")
        || body.contains("METHOD:CANCEL")
        || body.contains("METHOD:REPLY")
    {
        return true;
    }
    let lower = body.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "method=request",
        "microsoft teams meeting",
        "teams.microsoft.com/l/meetup-join",
        "teams.live.com/meet",
        "outlook.office.com/owa/calendar",
        "outlook.office365.com/owa/calendar",
        "meet.google.com/",
        "zoom.us/j/",
        "zoomgov.com/j/",
        "join microsoft teams meeting",
    ];
    NEEDLES.iter().any(|n| lower.contains(n))
}

pub(super) fn derive_company_tag(recipients: &[String], cc: &[String], owner_email: &str) -> Option<String> {
    let owner_domain = owner_email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase());
    // domain -> (count, lex-smallest contributing address)
    let mut buckets: std::collections::HashMap<String, (u32, String)> = std::collections::HashMap::new();
    for addr in recipients.iter().chain(cc.iter()) {
        let (_, addr_only) = crate::util::email_addr::split_name_addr(addr);
        let addr_only = if addr_only.is_empty() {
            addr.trim().to_ascii_lowercase()
        } else {
            addr_only
        };
        let Some((_, domain)) = addr_only.rsplit_once('@') else {
            continue;
        };
        if owner_domain.as_deref() == Some(domain) {
            continue;
        }
        let domain = domain.to_string();
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

fn empty_thread_state(email: &Email, now: i64) -> ThreadState {
    ThreadState {
        account_id: email.account_id.clone(),
        thread_id: email.thread_id.clone(),
        awaiting: "unknown".to_string(),
        last_inbound_at: None,
        last_outbound_at: None,
        last_touched_at: now,
        summary: None,
        commitment: None,
        deadline_at: None,
        participants: Vec::new(),
        updated_at: now,
    }
}

fn pick_non_empty(s: &Option<String>) -> Option<String> {
    s.as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("null"))
}

fn parse_json_subset<T: Default + for<'de> Deserialize<'de>>(raw: &str) -> T {
    serde_json::from_str::<T>(&extract_json(raw)).unwrap_or_default()
}

fn extract_json(text: &str) -> String {
    let cleaned = if text.contains("```") {
        text.lines()
            .filter(|l| !l.trim().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };
    if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
        if end >= start {
            return cleaned[start..=end].to_string();
        }
    }
    cleaned.trim().to_string()
}

fn parse_iso_ts(raw: &str) -> Option<i64> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("null") {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.timestamp());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|ndt| ndt.and_utc().timestamp());
    }
    None
}

fn emit_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_company_tag_picks_non_owner_domain() {
        let tag = derive_company_tag(
            &["alice@acme.com".into(), "bob@acme.com".into(), "me@mine.com".into()],
            &[],
            "me@mine.com",
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn parse_iso_handles_date_and_datetime() {
        assert!(parse_iso_ts("2026-04-23").is_some());
        assert!(parse_iso_ts("2026-04-23T12:00:00Z").is_some());
        assert!(parse_iso_ts("").is_none());
    }
}
