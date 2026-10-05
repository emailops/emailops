//! The email agent view. Thin wrappers over `services::agent`; the work that
//! calls the model (approved actions, panel counts) is queued and reported
//! through `agent-updated` and `app-log`.

use serde::Serialize;
use tauri::State;

use crate::models::agent::{
    AgentAction, AgentPanel, AgentPanelInput, AgentRule, AgentRuleInput, AgentRun, ReviewOutcome,
};
use crate::models::error::Result;
use crate::services::agent::{self, runner};
use crate::services::clock::now_secs;
use crate::AppState;

/// Everything the agent view shows, in one call.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOverview {
    pub enabled: bool,
    pub rules: Vec<AgentRule>,
    pub panels: Vec<AgentPanel>,
    pub feed: Vec<AgentRun>,
    pub actions: Vec<AgentAction>,
}

#[tauri::command]
pub async fn get_agent_overview(state: State<'_, AppState>) -> Result<AgentOverview> {
    let db = &state.db;
    Ok(AgentOverview {
        enabled: agent::is_enabled(db),
        rules: agent::list_rules(db)?,
        panels: agent::list_panels(db, now_secs())?,
        feed: agent::feed(db)?,
        actions: agent::actions(db)?,
    })
}

/// Turn the agent on (it then evaluates mail that arrives from now on) or off.
#[tauri::command]
pub async fn set_agent_enabled(state: State<'_, AppState>, enabled: bool) -> Result<()> {
    agent::set_enabled(&state.db, enabled, now_secs())?;
    crate::services::logger::log(
        "info",
        "agent",
        if enabled { "Email agent on" } else { "Email agent off" },
    );
    Ok(())
}

#[tauri::command]
pub async fn create_agent_rule(state: State<'_, AppState>, input: AgentRuleInput) -> Result<AgentRule> {
    agent::create_rule(&state.db, input, now_secs())
}

#[tauri::command]
pub async fn update_agent_rule(state: State<'_, AppState>, id: String, input: AgentRuleInput) -> Result<AgentRule> {
    agent::update_rule(&state.db, &id, input, now_secs())
}

#[tauri::command]
pub async fn delete_agent_rule(state: State<'_, AppState>, id: String) -> Result<()> {
    agent::delete_rule(&state.db, &id)
}

/// Count the mail already in a panel's window, in the background.
async fn queue_panel_backfill(state: &State<'_, AppState>, panel_id: String) {
    let db = state.db.clone();
    let label = format!("agent:panel:{panel_id}");
    state
        .ai_background
        .submit_priority(&label, async move {
            let provider = match crate::services::ai::AiService::load_provider(&db) {
                Ok(p) => p,
                Err(e) => {
                    crate::services::logger::log("warn", "agent", format!("Panel count skipped: {e}"));
                    return;
                }
            };
            match runner::backfill_panel(&db, provider.as_ref(), &panel_id, now_secs()).await {
                Ok(n) => crate::services::logger::log("success", "agent", format!("Panel counted: {n} email(s)")),
                Err(e) => crate::services::logger::log("error", "agent", format!("Panel count failed: {e}")),
            }
        })
        .await;
}

#[tauri::command]
pub async fn create_agent_panel(state: State<'_, AppState>, input: AgentPanelInput) -> Result<AgentPanel> {
    let panel = agent::create_panel(&state.db, input, now_secs())?;
    queue_panel_backfill(&state, panel.id.clone()).await;
    Ok(panel)
}

#[tauri::command]
pub async fn update_agent_panel(state: State<'_, AppState>, id: String, input: AgentPanelInput) -> Result<()> {
    if agent::update_panel(&state.db, &id, input)? {
        queue_panel_backfill(&state, id).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_agent_panel(state: State<'_, AppState>, id: String) -> Result<()> {
    agent::delete_panel(&state.db, &id)
}

/// Approve a pending action. It runs on the AI queue (a draft needs the
/// model); the view refreshes on `agent-updated`.
#[tauri::command]
pub async fn approve_agent_action(state: State<'_, AppState>, id: String) -> Result<()> {
    let db = state.db.clone();
    let label = format!("agent:action:{id}");
    state
        .ai_queue
        .submit_priority(&label, async move {
            let (provider, effects, skills) = match runner::service_deps(&db) {
                Ok(deps) => deps,
                Err(e) => {
                    crate::services::logger::log("error", "agent", format!("Agent action not run: {e}"));
                    return;
                }
            };
            let deps = runner::AgentDeps {
                provider: provider.as_ref(),
                effects: &effects,
                skills: &skills,
            };
            if let Err(e) = runner::run_action(&db, &deps, &id, now_secs()).await {
                crate::services::logger::log("error", "agent", format!("Agent action failed: {e}"));
            }
        })
        .await;
    Ok(())
}

#[tauri::command]
pub async fn reject_agent_action(state: State<'_, AppState>, id: String) -> Result<()> {
    agent::reject_action(&state.db, &id, now_secs())
}

/// The user sent or discarded a reply draft of the agent from the Agent view.
#[tauri::command]
pub async fn review_agent_draft(state: State<'_, AppState>, id: String, outcome: ReviewOutcome) -> Result<()> {
    agent::review_draft(&state.db, &id, outcome, now_secs())
}
