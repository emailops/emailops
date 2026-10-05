//! The email agent: rules evaluated on new mail and upcoming events, the
//! actions they take or propose, and prompt-defined stats panels.
//!
//! [`planner`] holds every decision (pure); [`runner`] does the I/O. This
//! module is the surface the commands and the sync hooks call.

pub mod planner;
pub mod runner;

use uuid::Uuid;

use crate::db::Database;
use crate::models::agent::{
    AgentAction, AgentPanel, AgentPanelInput, AgentRule, AgentRuleInput, AgentRun, ReviewOutcome, AGENT_ENABLED_PREF,
    AGENT_EVENT_LEAD_PREF, AGENT_SINCE_PREF,
};
use crate::models::error::{AppError, Result};

const DEFAULT_EVENT_LEAD_MINUTES: i64 = 15;
const MAX_PROMPT_CHARS: usize = 2_000;
const MAX_NAME_CHARS: usize = 80;
/// Runs the feed shows.
pub const FEED_LIMIT: i64 = 100;
/// Decided actions the side panel shows under the pending ones.
pub const RECENT_ACTIONS: i64 = 50;

/// Whether the agent is on. Off until the user turns it on.
pub fn is_enabled(db: &Database) -> bool {
    db.get_preference(AGENT_ENABLED_PREF)
        .ok()
        .flatten()
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

/// Turn the agent on or off. Turning it on restarts the clock: only mail
/// that arrives from `now` on is evaluated, never the backlog that came in
/// while it was off.
pub fn set_enabled(db: &Database, enabled: bool, now: i64) -> Result<()> {
    if enabled {
        db.set_preference(AGENT_SINCE_PREF, &now.to_string())?;
    }
    db.set_preference(AGENT_ENABLED_PREF, if enabled { "true" } else { "false" })
}

/// Unix seconds the agent was last turned on.
pub fn enabled_since(db: &Database) -> i64 {
    db.get_preference(AGENT_SINCE_PREF)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(i64::MAX)
}

/// How long before an event's start the agent looks at it.
pub fn event_lead_secs(db: &Database) -> i64 {
    let minutes = db
        .get_preference(AGENT_EVENT_LEAD_PREF)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_EVENT_LEAD_MINUTES);
    minutes.clamp(1, 120) * 60
}

fn required(field: &str, value: &str, max: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::InvalidInput(format!("{field} cannot be empty")));
    }
    if value.chars().count() > max {
        return Err(AppError::InvalidInput(format!(
            "{field} is longer than {max} characters"
        )));
    }
    Ok(value.to_string())
}

fn rule_from_input(db: &Database, id: String, input: AgentRuleInput, created_at: i64, now: i64) -> Result<AgentRule> {
    if let Some(account_id) = input.account_id.as_deref() {
        if db.get_account(account_id)?.is_none() {
            return Err(AppError::NotFound(format!("Account {account_id} not found")));
        }
    }
    Ok(AgentRule {
        id,
        name: required("name", &input.name, MAX_NAME_CHARS)?,
        trigger: input.trigger,
        account_id: input.account_id,
        match_prompt: required("match prompt", &input.match_prompt, MAX_PROMPT_CHARS)?,
        action_prompt: required("action prompt", &input.action_prompt, MAX_PROMPT_CHARS)?,
        always_approve: input.always_approve,
        enabled: input.enabled,
        created_at,
        updated_at: now,
    })
}

pub fn list_rules(db: &Database) -> Result<Vec<AgentRule>> {
    db.list_agent_rules()
}

pub fn create_rule(db: &Database, input: AgentRuleInput, now: i64) -> Result<AgentRule> {
    let rule = rule_from_input(db, Uuid::new_v4().to_string(), input, now, now)?;
    db.insert_agent_rule(&rule)?;
    Ok(rule)
}

pub fn update_rule(db: &Database, id: &str, input: AgentRuleInput, now: i64) -> Result<AgentRule> {
    let existing = db
        .get_agent_rule(id)?
        .ok_or_else(|| AppError::NotFound(format!("Agent rule {id} not found")))?;
    let rule = rule_from_input(db, existing.id, input, existing.created_at, now)?;
    db.update_agent_rule(&rule)?;
    Ok(rule)
}

pub fn delete_rule(db: &Database, id: &str) -> Result<()> {
    if !db.delete_agent_rule(id)? {
        return Err(AppError::NotFound(format!("Agent rule {id} not found")));
    }
    Ok(())
}

fn panel_from_input(id: String, input: AgentPanelInput, created_at: i64) -> Result<AgentPanel> {
    Ok(AgentPanel {
        id,
        title: required("title", &input.title, MAX_NAME_CHARS)?,
        prompt: required("prompt", &input.prompt, MAX_PROMPT_CHARS)?,
        window: input.window,
        created_at,
        count: 0,
    })
}

/// Every panel with its count over its window at `now`.
pub fn list_panels(db: &Database, now: i64) -> Result<Vec<AgentPanel>> {
    let offset = crate::services::clock::utc_offset_secs();
    let mut panels = db.list_agent_panels()?;
    for panel in &mut panels {
        let since = planner::window_start(panel.window, now, offset);
        panel.count = db.count_agent_panel_hits(&panel.id, since)?;
    }
    Ok(panels)
}

/// Create a panel. The caller queues [`runner::backfill_panel`] to count
/// the mail already in its window.
pub fn create_panel(db: &Database, input: AgentPanelInput, now: i64) -> Result<AgentPanel> {
    let panel = panel_from_input(Uuid::new_v4().to_string(), input, now)?;
    db.insert_agent_panel(&panel)?;
    Ok(panel)
}

/// Update a panel. Returns whether its prompt changed — its count then
/// starts over and the caller queues a backfill.
pub fn update_panel(db: &Database, id: &str, input: AgentPanelInput) -> Result<bool> {
    let existing = db
        .get_agent_panel(id)?
        .ok_or_else(|| AppError::NotFound(format!("Agent panel {id} not found")))?;
    let panel = panel_from_input(existing.id, input, existing.created_at)?;
    let prompt_changed = panel.prompt != existing.prompt;
    db.update_agent_panel(&panel)?;
    Ok(prompt_changed)
}

pub fn delete_panel(db: &Database, id: &str) -> Result<()> {
    if !db.delete_agent_panel(id)? {
        return Err(AppError::NotFound(format!("Agent panel {id} not found")));
    }
    Ok(())
}

pub fn feed(db: &Database) -> Result<Vec<AgentRun>> {
    db.list_agent_feed(FEED_LIMIT)
}

pub fn actions(db: &Database) -> Result<Vec<AgentAction>> {
    db.list_agent_actions(RECENT_ACTIONS)
}

/// Reject a pending action; it never runs.
pub fn reject_action(db: &Database, id: &str, now: i64) -> Result<()> {
    if !db.reject_agent_action(id, now)? {
        return Err(AppError::InvalidInput(format!(
            "Agent action {id} is no longer pending"
        )));
    }
    crate::services::events::emit("agent-updated", ());
    Ok(())
}

/// Record that the user sent or discarded a reply draft of the agent.
pub fn review_draft(db: &Database, id: &str, outcome: ReviewOutcome, now: i64) -> Result<()> {
    if !db.review_agent_draft(id, outcome, now)? {
        return Err(AppError::InvalidInput(format!(
            "Agent action {id} is not a reply draft waiting for review"
        )));
    }
    crate::services::events::emit("agent-updated", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::agent::{AgentTrigger, PanelWindow};

    const NOW: i64 = 1_800_000_000;

    fn input() -> AgentRuleInput {
        AgentRuleInput {
            name: "  Support  ".into(),
            trigger: AgentTrigger::Email,
            account_id: None,
            match_prompt: "customer asks for help".into(),
            action_prompt: "draft a reply".into(),
            always_approve: false,
            enabled: true,
        }
    }

    #[test]
    fn the_agent_is_off_until_turned_on_and_only_looks_at_mail_from_then() {
        let db = Database::new_for_testing().unwrap();
        assert!(!is_enabled(&db));
        assert_eq!(enabled_since(&db), i64::MAX);
        set_enabled(&db, true, NOW).unwrap();
        assert!(is_enabled(&db));
        assert_eq!(enabled_since(&db), NOW);
        set_enabled(&db, false, NOW + 10).unwrap();
        set_enabled(&db, true, NOW + 20).unwrap();
        assert_eq!(enabled_since(&db), NOW + 20, "mail that came while off is skipped");
    }

    #[test]
    fn the_event_lead_defaults_to_a_quarter_hour_and_is_bounded() {
        let db = Database::new_for_testing().unwrap();
        assert_eq!(event_lead_secs(&db), 15 * 60);
        db.set_preference(AGENT_EVENT_LEAD_PREF, "500").unwrap();
        assert_eq!(event_lead_secs(&db), 120 * 60);
        db.set_preference(AGENT_EVENT_LEAD_PREF, "0").unwrap();
        assert_eq!(event_lead_secs(&db), 60);
    }

    #[test]
    fn a_rule_is_stored_trimmed_and_keeps_its_creation_time_on_update() {
        let db = Database::new_for_testing().unwrap();
        let rule = create_rule(&db, input(), NOW).unwrap();
        assert_eq!(rule.name, "Support");
        let mut changed = input();
        changed.enabled = false;
        let updated = update_rule(&db, &rule.id, changed, NOW + 5).unwrap();
        assert_eq!((updated.created_at, updated.updated_at), (NOW, NOW + 5));
        assert_eq!(list_rules(&db).unwrap(), vec![updated]);
    }

    #[test]
    fn a_rule_needs_a_name_both_prompts_and_a_real_account() {
        let db = Database::new_for_testing().unwrap();
        for broken in [
            AgentRuleInput {
                name: " ".into(),
                ..input()
            },
            AgentRuleInput {
                match_prompt: "".into(),
                ..input()
            },
            AgentRuleInput {
                action_prompt: "\n".into(),
                ..input()
            },
            AgentRuleInput {
                match_prompt: "x".repeat(2_001),
                ..input()
            },
        ] {
            assert!(matches!(create_rule(&db, broken, NOW), Err(AppError::InvalidInput(_))));
        }
        let unknown = AgentRuleInput {
            account_id: Some("nope".into()),
            ..input()
        };
        assert!(matches!(create_rule(&db, unknown, NOW), Err(AppError::NotFound(_))));
        assert!(list_rules(&db).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_missing_rule_or_panel_is_not_found() {
        let db = Database::new_for_testing().unwrap();
        assert!(matches!(delete_rule(&db, "nope"), Err(AppError::NotFound(_))));
        assert!(matches!(delete_panel(&db, "nope"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn a_panel_update_reports_whether_its_prompt_changed() {
        let db = Database::new_for_testing().unwrap();
        let panel_input = |prompt: &str| AgentPanelInput {
            title: "Support".into(),
            prompt: prompt.into(),
            window: PanelWindow::Today,
        };
        let panel = create_panel(&db, panel_input("support requests"), NOW).unwrap();
        assert!(!update_panel(&db, &panel.id, panel_input("support requests")).unwrap());
        assert!(update_panel(&db, &panel.id, panel_input("complaints")).unwrap());
        assert!(matches!(
            create_panel(&db, panel_input(" "), NOW),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn reviewing_something_that_is_not_a_waiting_draft_is_refused() {
        let db = Database::new_for_testing().unwrap();
        assert!(matches!(
            review_draft(&db, "missing", ReviewOutcome::Sent, NOW),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn rejecting_twice_is_refused() {
        let db = Database::new_for_testing().unwrap();
        assert!(matches!(
            reject_action(&db, "missing", NOW),
            Err(AppError::InvalidInput(_))
        ));
    }
}
