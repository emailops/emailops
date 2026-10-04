//! The agent's executor: evaluates new mail and due events against the
//! rules and panels ([`planner`] decides, this module does the I/O), stores
//! each run with its actions, and runs an action — at once when it needs no
//! approval, or when the user approves it.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use super::planner::{self, Criterion, CriterionTarget, EventContext};
use crate::ai::json_shape::JsonShape;
use crate::ai::provider::{AIProvider, CompletionOptions};
use crate::db::Database;
use crate::models::agent::{
    AgentAction, AgentActionKind, AgentActionStatus, AgentPanel, AgentRule, AgentRun, AgentRunStatus, AgentTrigger,
};
use crate::models::error::{AppError, Result};
use crate::models::{CalendarEvent, CreateTaskRequest, Email, SaveDraftRequest};
use crate::services::emails::{ThreadAction, ThreadRef};
use crate::services::skills::SkillCatalog;

/// Emails one pass evaluates; the rest wait for the next sync.
pub const MAX_EMAILS_PER_PASS: i64 = 25;
/// Emails a new panel counts back over its window.
pub const MAX_PANEL_BACKFILL: i64 = 200;
/// How much of an email body the model reads.
const BODY_MAX_CHARS: usize = 3_000;
/// Messages with an event's attendees shown with the event.
const RECENT_WITH_ATTENDEES: i64 = 5;

/// What the agent does outside its own tables: the mailbox and the drafts.
/// A seam so tests run the executor without a provider account or a model
/// configured in the database.
#[async_trait]
pub trait AgentEffects: Send + Sync {
    /// Write a reply draft to `email_id` following `instructions`; returns
    /// the draft id.
    async fn draft_reply(&self, email_id: &str, instructions: &str) -> Result<String>;
    async fn thread_action(&self, account_id: &str, thread_id: &str, action: ThreadAction) -> Result<()>;
}

/// The production effects: the same services the composer and the inbox use.
pub struct ServiceEffects {
    pub db: Arc<Database>,
}

#[async_trait]
impl AgentEffects for ServiceEffects {
    async fn draft_reply(&self, email_id: &str, instructions: &str) -> Result<String> {
        let email = self
            .db
            .get_email_by_id(email_id)?
            .ok_or_else(|| AppError::NotFound(format!("Email {email_id} not found")))?;
        let instructions = (!instructions.trim().is_empty()).then_some(instructions);
        let generated = crate::services::emails::generate_draft(&self.db, email_id, instructions).await?;
        let subject = if email.subject.to_lowercase().starts_with("re:") {
            email.subject.clone()
        } else {
            format!("Re: {}", email.subject)
        };
        let draft = crate::services::emails::save_draft(
            &self.db,
            &SaveDraftRequest {
                id: None,
                email_id: Some(email_id.to_string()),
                account_id: email.account_id.clone(),
                to_addresses: vec![email.sender_email.clone()],
                cc_addresses: Vec::new(),
                subject,
                body: generated.body,
                body_html: None,
                provider_draft_id: None,
                attachments: None,
            },
        )?;
        Ok(draft.id)
    }

    async fn thread_action(&self, account_id: &str, thread_id: &str, action: ThreadAction) -> Result<()> {
        let report = crate::services::emails::apply_thread_action(
            &self.db,
            &[ThreadRef {
                account_id: account_id.to_string(),
                thread_id: thread_id.to_string(),
            }],
            action,
            None,
        )
        .await;
        match report.failed.into_iter().next() {
            // The failure carries the error already rendered; re-wrapping it
            // must not print "Sync error:" twice.
            Some(failure) => Err(AppError::SyncError(
                failure
                    .message
                    .strip_prefix("Sync error: ")
                    .unwrap_or(&failure.message)
                    .to_string(),
            )),
            None => Ok(()),
        }
    }
}

/// The production dependencies: the configured provider (refused while the
/// AI master switch is off), the real effects and the enabled skills.
pub fn service_deps(db: &Arc<Database>) -> Result<(Arc<dyn AIProvider>, ServiceEffects, SkillCatalog)> {
    let provider = crate::services::ai::AiService::load_provider(db)?;
    let effects = ServiceEffects { db: Arc::clone(db) };
    Ok((provider, effects, crate::services::skills::catalog_for(db)))
}

/// Everything a pass needs besides the database.
pub struct AgentDeps<'a> {
    pub provider: &'a dyn AIProvider,
    pub effects: &'a dyn AgentEffects,
    /// The user's enabled skills (empty when skills are off).
    pub skills: &'a SkillCatalog,
}

fn log(level: &str, message: impl Into<String>) {
    crate::services::logger::log(level, "agent", message);
}

/// Tell the agent view to reload.
fn notify_changed() {
    crate::services::events::emit("agent-updated", ());
}

fn format_time(ts: i64) -> String {
    let local = ts + i64::from(crate::services::clock::utc_offset_secs());
    chrono::DateTime::from_timestamp(local, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// One structured call to the model. A provider error is returned as is (the
/// trigger is retried on the next pass); the reply is returned unparsed.
async fn ask(
    provider: &dyn AIProvider,
    prefix: &str,
    suffix: &str,
    shape: JsonShape,
    max_tokens: u32,
) -> Result<String> {
    let options = CompletionOptions {
        temperature: Some(0.0),
        max_tokens: Some(max_tokens),
        think: Some(false),
        json_shape: Some(shape),
    };
    let started = std::time::Instant::now();
    let result = provider.complete_with_prefix(prefix, suffix, options).await?;
    crate::ai::tracing::driver().record_generation(crate::ai::tracing::GenerationParams {
        trace_name: "agent",
        name: "agent_decision",
        model: &result.model,
        input: suffix,
        output: &result.text,
        prompt_tokens: result.prompt_tokens,
        completion_tokens: result.completion_tokens,
        latency_ms: started.elapsed().as_millis() as u64,
        error: None,
    });
    Ok(result.text)
}

/// The text the model reads for an email.
fn email_context(db: &Database, email: &Email) -> String {
    let body = crate::services::lenses::extractor::body_for_extraction(db, email, BODY_MAX_CHARS);
    let from = if email.sender.is_empty() || email.sender == email.sender_email {
        email.sender_email.clone()
    } else {
        format!("{} <{}>", email.sender, email.sender_email)
    };
    planner::render_email_context(&from, &format_time(email.timestamp), &email.subject, &body)
}

/// The text the model reads for an event, with recent mail from its people.
fn event_context(db: &Database, event: &CalendarEvent) -> Result<String> {
    let attendees: Vec<&str> = event.attendees.iter().map(|a| a.email.as_str()).collect();
    let recent = db
        .agent_recent_with(&event.account_id, &attendees, RECENT_WITH_ATTENDEES)?
        .into_iter()
        .map(|(ts, sender, subject)| format!("{} {sender}: {subject}", format_time(ts)))
        .collect();
    Ok(planner::render_event_context(&EventContext {
        title: &event.title,
        start: &format_time(event.start_time),
        location: &event.location,
        organizer: &event.organizer,
        attendees,
        description: &crate::services::lenses::extractor::clean_and_trim_body(&event.description, BODY_MAX_CHARS),
        recent,
    }))
}

/// The item a run is about, as the model reads it.
fn run_context(db: &Database, run: &AgentRun) -> Result<String> {
    match run.trigger {
        AgentTrigger::Email => {
            let email = db
                .get_email_by_id(&run.trigger_ref)?
                .ok_or_else(|| AppError::NotFound(format!("Email {} not found", run.trigger_ref)))?;
            Ok(email_context(db, &email))
        }
        AgentTrigger::Event => {
            let event = db
                .get_calendar_event(&run.trigger_ref)?
                .ok_or_else(|| AppError::NotFound(format!("Event {} not found", run.trigger_ref)))?;
            event_context(db, &event)
        }
    }
}

/// One email or event to evaluate.
struct Trigger<'a> {
    kind: AgentTrigger,
    account_id: &'a str,
    trigger_ref: &'a str,
    title: &'a str,
    sender: &'a str,
    /// The email's timestamp, for panel windows; `None` for an event.
    email_timestamp: Option<i64>,
    context: String,
}

/// What the model decided about one email or event, before anything is
/// stored or run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The criteria (rules and panels) the model said match.
    pub matched: Vec<Criterion>,
    /// The match reply could not be read.
    pub unreadable: bool,
    /// One entry per matched rule, in criteria order.
    pub rules: Vec<RuleDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDecision {
    pub rule_id: String,
    pub rule_name: String,
    /// `None` when the action reply could not be read.
    pub summary: Option<String>,
    pub actions: Vec<planner::PlannedAction>,
}

/// Ask the model which rules and panels match `context`, then what each
/// matched rule does. No I/O besides the model; the eval harness calls this
/// directly. A provider error is returned as is.
pub async fn decide(
    provider: &dyn AIProvider,
    skills: &SkillCatalog,
    rules: &[AgentRule],
    panels: &[AgentPanel],
    trigger: AgentTrigger,
    account_id: &str,
    context: &str,
) -> Result<Decision> {
    let criteria = planner::plan_criteria(rules, panels, trigger, account_id);
    let mut decision = Decision {
        matched: Vec::new(),
        unreadable: false,
        rules: Vec::new(),
    };
    if criteria.is_empty() {
        return Ok(decision);
    }
    let (prefix, suffix) = planner::match_prompt(&criteria, context);
    let reply = ask(
        provider,
        &prefix,
        &suffix,
        planner::match_shape(&criteria),
        planner::match_max_tokens(&criteria),
    )
    .await?;
    match planner::parse_matches(&reply, &criteria) {
        Some(found) => decision.matched = found.into_iter().cloned().collect(),
        None => {
            decision.unreadable = true;
            return Ok(decision);
        }
    }

    let skill_names: Vec<&str> = skills.names();
    let skill_list: Vec<(&str, &str)> = skills
        .skills
        .iter()
        .map(|s| (s.name.as_str(), s.description.as_str()))
        .collect();
    let offered = planner::offered_kinds(trigger, !skill_names.is_empty());
    for criterion in &decision.matched {
        let CriterionTarget::Rule(rule_id) = &criterion.target else {
            continue;
        };
        let Some(rule) = rules.iter().find(|r| &r.id == rule_id) else {
            continue;
        };
        let (prefix, suffix) = planner::action_prompt(rule, context, &offered, &skill_list);
        let reply = ask(provider, &prefix, &suffix, planner::action_shape(&offered), 700).await?;
        let parsed = planner::parse_action_reply(&reply);
        decision.rules.push(RuleDecision {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            summary: parsed.as_ref().map(|r| r.summary.trim().to_string()),
            actions: parsed
                .map(|r| planner::plan_actions(&r, rule, &offered, &skill_names))
                .unwrap_or_default(),
        });
    }
    Ok(decision)
}

/// Evaluate one trigger and store its run; run the actions that need no
/// approval. Returns whether a rule matched. A provider error is returned
/// before anything is stored, so the trigger is tried again next pass.
async fn evaluate(
    db: &Arc<Database>,
    deps: &AgentDeps<'_>,
    trigger: Trigger<'_>,
    rules: &[AgentRule],
    panels: &[AgentPanel],
    now: i64,
) -> Result<bool> {
    let decision = decide(
        deps.provider,
        deps.skills,
        rules,
        panels,
        trigger.kind,
        trigger.account_id,
        &trigger.context,
    )
    .await?;

    for criterion in &decision.matched {
        if let (CriterionTarget::Panel(panel_id), Some(ts)) = (&criterion.target, trigger.email_timestamp) {
            db.add_agent_panel_hit(panel_id, trigger.trigger_ref, ts)?;
        }
    }

    let id = Uuid::new_v4().to_string();
    let actions: Vec<AgentAction> = decision
        .rules
        .iter()
        .flat_map(|r| {
            r.actions.iter().map(|planned| AgentAction {
                id: Uuid::new_v4().to_string(),
                run_id: id.clone(),
                rule_id: Some(r.rule_id.clone()),
                rule_name: r.rule_name.clone(),
                kind: planned.kind,
                detail: planned.detail.clone(),
                status: AgentActionStatus::Pending,
                requires_approval: planned.requires_approval,
                result: None,
                error: None,
                created_at: now,
                decided_at: None,
                run_title: trigger.title.to_string(),
            })
        })
        .collect();
    let unreadable_rule = decision.rules.iter().find(|r| r.summary.is_none());
    let (status, error) = if decision.unreadable {
        (
            AgentRunStatus::Failed,
            Some("The model's answer could not be read.".to_string()),
        )
    } else if decision.rules.is_empty() {
        (AgentRunStatus::NoMatch, None)
    } else {
        (
            AgentRunStatus::Matched,
            unreadable_rule.map(|r| format!("The model's answer for rule '{}' could not be read.", r.rule_name)),
        )
    };
    let summary = match decision.rules.as_slice() {
        [only] => only.summary.clone().unwrap_or_default(),
        many => many
            .iter()
            .map(|r| format!("{}: {}", r.rule_name, r.summary.as_deref().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    let run = AgentRun {
        id,
        account_id: trigger.account_id.to_string(),
        trigger: trigger.kind,
        trigger_ref: trigger.trigger_ref.to_string(),
        title: trigger.title.to_string(),
        sender: trigger.sender.to_string(),
        status,
        summary,
        error,
        created_at: now,
        actions,
    };

    if !db.insert_agent_run(&run)? {
        return Ok(false);
    }
    for action in run.actions.iter().filter(|a| !a.requires_approval) {
        run_action(db, deps, &action.id, now).await?;
    }
    if run.status != AgentRunStatus::NoMatch {
        notify_changed();
    }
    Ok(run.status == AgentRunStatus::Matched)
}

/// Evaluate `account_id`'s new mail since the agent was turned on. Returns
/// how many emails matched a rule.
pub async fn process_new_emails(db: &Arc<Database>, deps: &AgentDeps<'_>, account_id: &str, now: i64) -> Result<usize> {
    if !super::is_enabled(db) {
        return Ok(0);
    }
    let rules = db.list_agent_rules()?;
    let panels = db.list_agent_panels()?;
    let has_email_rules = rules.iter().any(|r| r.enabled && r.trigger == AgentTrigger::Email);
    if !has_email_rules && panels.is_empty() {
        return Ok(0);
    }
    let since = super::enabled_since(db);
    let mut matched = 0;
    for email_id in db.agent_candidate_emails(account_id, since, MAX_EMAILS_PER_PASS)? {
        if crate::services::task_queue::cancel_requested() {
            log("info", "Agent stopped by the user");
            break;
        }
        let Some(email) = db.get_email_by_id(&email_id)? else {
            continue;
        };
        let trigger = Trigger {
            kind: AgentTrigger::Email,
            account_id: &email.account_id,
            trigger_ref: &email.id,
            title: &email.subject,
            sender: if email.sender.is_empty() {
                &email.sender_email
            } else {
                &email.sender
            },
            email_timestamp: Some(email.timestamp),
            context: email_context(db, &email),
        };
        if evaluate(db, deps, trigger, &rules, &panels, now).await? {
            matched += 1;
        }
    }
    Ok(matched)
}

/// Evaluate the events of the calendar-enabled accounts that start within
/// the lead time. Returns how many matched a rule.
pub async fn process_due_events(db: &Arc<Database>, deps: &AgentDeps<'_>, now: i64) -> Result<usize> {
    if !super::is_enabled(db) {
        return Ok(0);
    }
    let rules = db.list_agent_rules()?;
    if !rules.iter().any(|r| r.enabled && r.trigger == AgentTrigger::Event) {
        return Ok(0);
    }
    let lead = super::event_lead_secs(db);
    let mut matched = 0;
    for account in db.list_accounts()?.into_iter().filter(|a| a.enabled) {
        if !db.calendar_enabled(&account.id)? {
            continue;
        }
        let events = db.list_visible_calendar_events(&account.id, now, now + lead + 60)?;
        for event in planner::due_events(&events, now, lead) {
            if db.agent_run_exists(AgentTrigger::Event, &event.id)? {
                continue;
            }
            let trigger = Trigger {
                kind: AgentTrigger::Event,
                account_id: &event.account_id,
                trigger_ref: &event.id,
                title: &event.title,
                sender: &event.organizer,
                email_timestamp: None,
                context: event_context(db, event)?,
            };
            if evaluate(db, deps, trigger, &rules, &[], now).await? {
                matched += 1;
            }
        }
    }
    Ok(matched)
}

/// Count a new (or re-prompted) panel over its window: check up to
/// [`MAX_PANEL_BACKFILL`] inbox emails against it. Returns the hits.
pub async fn backfill_panel(db: &Arc<Database>, provider: &dyn AIProvider, panel_id: &str, now: i64) -> Result<usize> {
    let Some(panel) = db.get_agent_panel(panel_id)? else {
        return Ok(0);
    };
    let since = planner::window_start(panel.window, now, crate::services::clock::utc_offset_secs());
    let criteria = planner::plan_criteria(&[], std::slice::from_ref(&panel), AgentTrigger::Email, "");
    let mut hits = 0;
    for email_id in db.agent_panel_window_emails(since, MAX_PANEL_BACKFILL)? {
        if crate::services::task_queue::cancel_requested() {
            break;
        }
        let Some(email) = db.get_email_by_id(&email_id)? else {
            continue;
        };
        let (prefix, suffix) = planner::match_prompt(&criteria, &email_context(db, &email));
        let reply = ask(
            provider,
            &prefix,
            &suffix,
            planner::match_shape(&criteria),
            planner::match_max_tokens(&criteria),
        )
        .await?;
        if planner::parse_matches(&reply, &criteria).is_some_and(|m| !m.is_empty()) {
            db.add_agent_panel_hit(&panel.id, &email.id, email.timestamp)?;
            hits += 1;
        }
    }
    notify_changed();
    Ok(hits)
}

const SKILL_INSTRUCTIONS: &str = "You are an email assistant. Follow the skill below on the email or calendar \
event that comes after it. Reply in plain text, in the language of the item, without preamble.\n\n";

/// What an action does; returns what it produced.
async fn execute(
    db: &Arc<Database>,
    deps: &AgentDeps<'_>,
    action: &AgentAction,
    run: &AgentRun,
) -> Result<Option<String>> {
    let email = || -> Result<Email> {
        db.get_email_by_id(&run.trigger_ref)?
            .ok_or_else(|| AppError::NotFound(format!("Email {} not found", run.trigger_ref)))
    };
    let thread_action = |kind: AgentActionKind| match kind {
        AgentActionKind::MarkRead => Some(ThreadAction::MarkRead),
        AgentActionKind::Archive => Some(ThreadAction::Archive),
        AgentActionKind::Star => Some(ThreadAction::Star),
        _ => None,
    };
    match action.kind {
        AgentActionKind::DraftReply => Ok(Some(deps.effects.draft_reply(&run.trigger_ref, &action.detail).await?)),
        AgentActionKind::CreateTask => {
            let source_email = (run.trigger == AgentTrigger::Email).then(email).transpose()?;
            let task = crate::services::tasks::create_task(
                db,
                CreateTaskRequest {
                    account_id: run.account_id.clone(),
                    title: action.detail.clone(),
                    detail: Some(run.title.clone()),
                    priority: None,
                    due_at: None,
                    source_email_id: source_email.as_ref().map(|e| e.id.clone()),
                    source_thread_id: source_email.as_ref().map(|e| e.thread_id.clone()),
                    source: Some("agent".to_string()),
                    company: None,
                },
            )?;
            Ok(Some(task.id))
        }
        AgentActionKind::RunSkill => {
            let skill = deps
                .skills
                .get(&action.detail)
                .ok_or_else(|| AppError::NotFound(format!("Skill {} not found", action.detail)))?;
            let prefix = format!(
                "{SKILL_INSTRUCTIONS}{}\n\n",
                crate::services::skills::render_skill_block(skill)
            );
            let options = CompletionOptions {
                temperature: Some(0.3),
                max_tokens: Some(700),
                think: Some(false),
                json_shape: None,
            };
            let reply = deps
                .provider
                .complete_with_prefix(&prefix, &run_context(db, run)?, options)
                .await?;
            Ok(Some(reply.text.trim().to_string()))
        }
        kind => {
            let thread = thread_action(kind)
                .ok_or_else(|| AppError::InvalidInput(format!("unknown agent action {}", kind.as_str())))?;
            let email = email()?;
            deps.effects
                .thread_action(&email.account_id, &email.thread_id, thread)
                .await?;
            Ok(None)
        }
    }
}

/// Run a pending action once: claim it, execute it, record the outcome.
/// Returns the action as stored afterwards. An action that is no longer
/// pending (already run, rejected) is returned unchanged.
pub async fn run_action(db: &Arc<Database>, deps: &AgentDeps<'_>, action_id: &str, now: i64) -> Result<AgentAction> {
    let action = db
        .get_agent_action(action_id)?
        .ok_or_else(|| AppError::NotFound(format!("Agent action {action_id} not found")))?;
    if !db.claim_agent_action(action_id, now)? {
        return Ok(action);
    }
    let run = db
        .get_agent_run(&action.run_id)?
        .ok_or_else(|| AppError::NotFound(format!("Agent run {} not found", action.run_id)))?;
    match execute(db, deps, &action, &run).await {
        Ok(result) => {
            db.finish_agent_action(action_id, Ok(result.as_deref()))?;
            log(
                "success",
                format!("Agent: {} done for '{}'", action.kind.as_str(), run.title),
            );
        }
        Err(e) => {
            db.finish_agent_action(action_id, Err(&e.to_string()))?;
            log(
                "error",
                format!("Agent: {} failed for '{}': {e}", action.kind.as_str(), run.title),
            );
        }
    }
    notify_changed();
    db.get_agent_action(action_id)?
        .ok_or_else(|| AppError::NotFound(format!("Agent action {action_id} not found")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::FakeAiProvider;
    use crate::models::agent::{AgentRuleInput, PanelWindow};
    use crate::services::skills::Skill;
    use std::sync::Mutex;

    const NOW: i64 = 1_800_000_000;

    #[derive(Default)]
    struct FakeEffects {
        drafts: Mutex<Vec<(String, String)>>,
        thread_actions: Mutex<Vec<(String, String, ThreadAction)>>,
        fail_thread_actions: bool,
    }

    #[async_trait]
    impl AgentEffects for FakeEffects {
        async fn draft_reply(&self, email_id: &str, instructions: &str) -> Result<String> {
            self.drafts
                .lock()
                .unwrap()
                .push((email_id.to_string(), instructions.to_string()));
            Ok("draft-1".to_string())
        }

        async fn thread_action(&self, account_id: &str, thread_id: &str, action: ThreadAction) -> Result<()> {
            if self.fail_thread_actions {
                return Err(AppError::SyncError("provider offline".into()));
            }
            self.thread_actions
                .lock()
                .unwrap()
                .push((account_id.to_string(), thread_id.to_string(), action));
            Ok(())
        }
    }

    fn db() -> Arc<Database> {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc-1");
        super::super::set_enabled(&db, true, NOW - 3_600).unwrap();
        db
    }

    fn seed_email(db: &Database, id: &str, timestamp: i64) {
        db.connection()
            .execute(
                "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                                     recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox,
                                     created_at)
                 VALUES (?1, 'acc-1', ?2, 'Cannot log in', 'Ana', 'ana@example.com', 'example.com', '[]', '[]',
                         '', ?3, 0, 'primary', 'inbox', 0)",
                rusqlite::params![id, format!("thread-{id}"), timestamp],
            )
            .unwrap();
    }

    fn add_rule(db: &Database, trigger: AgentTrigger, always_approve: bool) -> AgentRule {
        super::super::create_rule(
            db,
            AgentRuleInput {
                name: "Support".into(),
                trigger,
                account_id: None,
                match_prompt: "a customer asks for help".into(),
                action_prompt: "draft a reply and archive".into(),
                always_approve,
                enabled: true,
            },
            NOW - 10,
        )
        .unwrap()
    }

    fn deps<'a>(provider: &'a FakeAiProvider, effects: &'a FakeEffects, skills: &'a SkillCatalog) -> AgentDeps<'a> {
        AgentDeps {
            provider,
            effects,
            skills,
        }
    }

    #[tokio::test]
    async fn a_matching_email_runs_its_local_actions_and_queues_mailbox_changes() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "asks for help", "answer": "yes"}}"#);
        ai.push_completion(
            r#"{"summary": "Ana cannot log in.", "actions": [
                {"action": "draft_reply", "detail": "Offer a password reset"},
                {"action": "archive", "detail": ""}]}"#,
        );
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();

        let matched = process_new_emails(&db, &deps(&ai, &effects, &skills), "acc-1", NOW)
            .await
            .unwrap();

        assert_eq!(matched, 1);
        let feed = db.list_agent_feed(10).unwrap();
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].summary, "Ana cannot log in.");
        assert_eq!(feed[0].title, "Cannot log in");
        let actions: Vec<_> = feed[0]
            .actions
            .iter()
            .map(|a| (a.kind, a.status, a.result.clone()))
            .collect();
        assert_eq!(
            actions,
            vec![
                (
                    AgentActionKind::DraftReply,
                    AgentActionStatus::Done,
                    Some("draft-1".to_string())
                ),
                (AgentActionKind::Archive, AgentActionStatus::Pending, None),
            ]
        );
        assert_eq!(
            effects.drafts.lock().unwrap().clone(),
            vec![("e1".to_string(), "Offer a password reset".to_string())]
        );
        assert!(
            effects.thread_actions.lock().unwrap().is_empty(),
            "archive waits for approval"
        );
    }

    #[tokio::test]
    async fn an_email_no_rule_matches_is_evaluated_once_and_stays_out_of_the_feed() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "not support", "answer": "no"}}"#);
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();
        let d = deps(&ai, &effects, &skills);

        assert_eq!(process_new_emails(&db, &d, "acc-1", NOW).await.unwrap(), 0);
        assert_eq!(process_new_emails(&db, &d, "acc-1", NOW).await.unwrap(), 0);

        assert_eq!(ai.completion_calls().len(), 1, "never asked twice about the same email");
        assert!(db.list_agent_feed(10).unwrap().is_empty());
        assert!(db.agent_run_exists(AgentTrigger::Email, "e1").unwrap());
    }

    #[tokio::test]
    async fn mail_from_before_the_agent_was_turned_on_or_while_it_is_off_is_ignored() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "history", NOW - 7_200);
        let ai = FakeAiProvider::new();
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();
        let d = deps(&ai, &effects, &skills);
        process_new_emails(&db, &d, "acc-1", NOW).await.unwrap();

        super::super::set_enabled(&db, false, NOW).unwrap();
        seed_email(&db, "new", NOW - 1);
        process_new_emails(&db, &d, "acc-1", NOW).await.unwrap();

        assert!(ai.completion_calls().is_empty());
    }

    #[tokio::test]
    async fn a_provider_failure_leaves_the_email_for_the_next_pass() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.fail_completions(Some("model not loaded"));
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();

        assert!(process_new_emails(&db, &deps(&ai, &effects, &skills), "acc-1", NOW)
            .await
            .is_err());
        assert!(!db.agent_run_exists(AgentTrigger::Email, "e1").unwrap());
    }

    #[tokio::test]
    async fn an_unreadable_match_reply_is_shown_as_a_failed_run() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion("I think R1 applies");
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();

        process_new_emails(&db, &deps(&ai, &effects, &skills), "acc-1", NOW)
            .await
            .unwrap();

        let feed = db.list_agent_feed(10).unwrap();
        assert_eq!(feed[0].status, AgentRunStatus::Failed);
        assert!(feed[0].error.is_some());
    }

    #[tokio::test]
    async fn a_panel_match_counts_the_email_without_a_feed_entry() {
        let db = db();
        let panel = super::super::create_panel(
            &db,
            crate::models::agent::AgentPanelInput {
                title: "Support today".into(),
                prompt: "support requests".into(),
                window: PanelWindow::Last7Days,
            },
            NOW - 10,
        )
        .unwrap();
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"P1": {"reason": "support", "answer": "yes"}}"#);
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();

        process_new_emails(&db, &deps(&ai, &effects, &skills), "acc-1", NOW)
            .await
            .unwrap();

        assert_eq!(db.count_agent_panel_hits(&panel.id, 0).unwrap(), 1);
        assert!(db.list_agent_feed(10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn approving_a_mailbox_action_applies_it_once() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "asks for help", "answer": "yes"}}"#);
        ai.push_completion(r#"{"summary": "s", "actions": [{"action": "star", "detail": ""}]}"#);
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();
        let d = deps(&ai, &effects, &skills);
        process_new_emails(&db, &d, "acc-1", NOW).await.unwrap();
        let pending = db.list_agent_actions(10).unwrap()[0].clone();

        let done = run_action(&db, &d, &pending.id, NOW + 5).await.unwrap();
        run_action(&db, &d, &pending.id, NOW + 6).await.unwrap();

        assert_eq!(done.status, AgentActionStatus::Done);
        assert_eq!(done.decided_at, Some(NOW + 5));
        assert_eq!(
            effects.thread_actions.lock().unwrap().clone(),
            vec![("acc-1".to_string(), "thread-e1".to_string(), ThreadAction::Star)]
        );
    }

    #[tokio::test]
    async fn a_failing_action_is_recorded_with_its_error() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "asks for help", "answer": "yes"}}"#);
        ai.push_completion(r#"{"summary": "s", "actions": [{"action": "mark_read", "detail": ""}]}"#);
        let effects = FakeEffects {
            fail_thread_actions: true,
            ..Default::default()
        };
        let skills = SkillCatalog::default();
        let d = deps(&ai, &effects, &skills);
        process_new_emails(&db, &d, "acc-1", NOW).await.unwrap();
        let pending = db.list_agent_actions(10).unwrap()[0].clone();

        let failed = run_action(&db, &d, &pending.id, NOW).await.unwrap();

        assert_eq!(failed.status, AgentActionStatus::Failed);
        assert!(failed.error.unwrap().contains("provider offline"));
    }

    #[tokio::test]
    async fn an_always_ask_rule_creates_the_task_only_once_approved() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, true);
        seed_email(&db, "e1", NOW - 60);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "asks for help", "answer": "yes"}}"#);
        ai.push_completion(r#"{"summary": "s", "actions": [{"action": "create_task", "detail": "Call Ana"}]}"#);
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();
        let d = deps(&ai, &effects, &skills);
        process_new_emails(&db, &d, "acc-1", NOW).await.unwrap();
        assert!(db.list_pending_tasks("acc-1", None, None, 10).unwrap().is_empty());

        let pending = db.list_agent_actions(10).unwrap()[0].clone();
        let done = run_action(&db, &d, &pending.id, NOW).await.unwrap();

        let tasks = db.list_pending_tasks("acc-1", None, None, 10).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Call Ana");
        assert_eq!(tasks[0].source, "agent");
        assert_eq!(tasks[0].source_email_id.as_deref(), Some("e1"));
        assert_eq!(done.result, Some(tasks[0].id.clone()));
    }

    #[tokio::test]
    async fn a_skill_action_stores_the_skill_output() {
        let db = db();
        add_rule(&db, AgentTrigger::Email, false);
        seed_email(&db, "e1", NOW - 60);
        let skills = SkillCatalog {
            skills: vec![Skill {
                name: "triage".into(),
                description: "Sort support mail".into(),
                body: "Say which product the email is about.".into(),
                path: Default::default(),
                files: vec![],
            }],
            errors: vec![],
        };
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "asks for help", "answer": "yes"}}"#);
        ai.push_completion(r#"{"summary": "s", "actions": [{"action": "run_skill", "detail": "triage"}]}"#);
        ai.push_completion("  The email is about the login page.  ");
        let effects = FakeEffects::default();

        process_new_emails(&db, &deps(&ai, &effects, &skills), "acc-1", NOW)
            .await
            .unwrap();

        let action = db.list_agent_actions(10).unwrap()[0].clone();
        assert_eq!(action.status, AgentActionStatus::Done);
        assert_eq!(action.result.as_deref(), Some("The email is about the login page."));
        let (prefix, suffix) = ai.prefix_completion_calls()[2].clone();
        assert!(prefix.contains("Say which product the email is about."));
        assert!(suffix.contains("Cannot log in"));
    }

    fn seed_event(db: &Database, id: &str, start: i64) {
        db.upsert_calendar_events(&[CalendarEvent {
            id: id.into(),
            account_id: "acc-1".into(),
            provider_event_id: id.into(),
            calendar_id: "primary".into(),
            title: "Kickoff with Ana".into(),
            description: "Agenda".into(),
            location: String::new(),
            start_time: start,
            end_time: start + 1_800,
            is_all_day: false,
            timezone: String::new(),
            organizer: "boss@example.com".into(),
            attendees: vec![crate::models::CalendarAttendee {
                email: "ana@example.com".into(),
                response: "accepted".into(),
            }],
            meeting_link: None,
            meeting_platform: None,
            status: "confirmed".into(),
            html_link: None,
            notified_at: None,
            recurring_event_id: None,
            created_at: 0,
            updated_at: 0,
        }])
        .unwrap();
    }

    #[tokio::test]
    async fn a_due_event_is_evaluated_once_with_recent_mail_from_its_attendees() {
        let db = db();
        add_rule(&db, AgentTrigger::Event, false);
        seed_email(&db, "e1", NOW - 60);
        seed_event(&db, "ev-soon", NOW + 300);
        seed_event(&db, "ev-later", NOW + 7_200);
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"R1": {"reason": "asks for help", "answer": "yes"}}"#);
        ai.push_completion(r#"{"summary": "Kickoff in 5 minutes", "actions": [{"action": "create_task", "detail": "Prepare kickoff"}]}"#);
        let effects = FakeEffects::default();
        let skills = SkillCatalog::default();
        let d = deps(&ai, &effects, &skills);

        assert_eq!(process_due_events(&db, &d, NOW).await.unwrap(), 1);
        assert_eq!(process_due_events(&db, &d, NOW + 60).await.unwrap(), 0);

        let feed = db.list_agent_feed(10).unwrap();
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].trigger_ref, "ev-soon");
        assert_eq!(feed[0].actions[0].status, AgentActionStatus::Done);
        let (_, match_suffix) = ai.prefix_completion_calls()[0].clone();
        assert!(match_suffix.contains("Kickoff with Ana"));
        assert!(match_suffix.contains("ana@example.com: Cannot log in"));
        assert_eq!(ai.completion_calls().len(), 2);
    }

    #[tokio::test]
    async fn a_new_panel_counts_the_emails_of_its_window() {
        let db = db();
        seed_email(&db, "in-window", NOW - 60);
        seed_email(&db, "too-old", NOW - 30 * 86_400);
        seed_email(&db, "not-support", NOW - 120);
        let panel = super::super::create_panel(
            &db,
            crate::models::agent::AgentPanelInput {
                title: "Support".into(),
                prompt: "support requests".into(),
                window: PanelWindow::Last7Days,
            },
            NOW,
        )
        .unwrap();
        let ai = FakeAiProvider::new();
        ai.push_completion(r#"{"P1": {"reason": "support", "answer": "yes"}}"#);
        ai.push_completion(r#"{"R1": {"reason": "not support", "answer": "no"}}"#);

        assert_eq!(backfill_panel(&db, &ai, &panel.id, NOW).await.unwrap(), 1);

        assert_eq!(ai.completion_calls().len(), 2, "the old email is outside the window");
        assert_eq!(db.count_agent_panel_hits(&panel.id, 0).unwrap(), 1);
    }
}
