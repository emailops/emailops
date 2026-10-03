//! Types of the email agent (V035): the user's rules, the runs the agent made
//! on an email or a calendar event, the actions each run took or proposes,
//! and the user's stats panels.

use serde::{Deserialize, Serialize};

/// What wakes the agent up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentTrigger {
    /// A new message arrived in an inbox.
    Email,
    /// A calendar event is about to start.
    Event,
}

impl AgentTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentTrigger::Email => "email",
            AgentTrigger::Event => "event",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "email" => Some(AgentTrigger::Email),
            "event" => Some(AgentTrigger::Event),
            _ => None,
        }
    }
}

/// Something the agent can do. It never sends mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentActionKind {
    /// Write a reply draft, saved locally (not pushed to the provider).
    DraftReply,
    /// Add a task to the Tasks list.
    CreateTask,
    /// Run one of the user's skills on the email or event; its output is a
    /// note in the agent's feed.
    RunSkill,
    MarkRead,
    Archive,
    Star,
}

impl AgentActionKind {
    pub const ALL: [AgentActionKind; 6] = [
        AgentActionKind::DraftReply,
        AgentActionKind::CreateTask,
        AgentActionKind::RunSkill,
        AgentActionKind::MarkRead,
        AgentActionKind::Archive,
        AgentActionKind::Star,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AgentActionKind::DraftReply => "draft_reply",
            AgentActionKind::CreateTask => "create_task",
            AgentActionKind::RunSkill => "run_skill",
            AgentActionKind::MarkRead => "mark_read",
            AgentActionKind::Archive => "archive",
            AgentActionKind::Star => "star",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Changes the mailbox on the provider's server. These always wait for
    /// the user's approval; the others stay local and reversible.
    pub fn touches_mailbox(self) -> bool {
        matches!(
            self,
            AgentActionKind::MarkRead | AgentActionKind::Archive | AgentActionKind::Star
        )
    }

    /// Whether the action makes sense for this trigger: an event has no
    /// message to reply to, mark, archive or star.
    pub fn applies_to(self, trigger: AgentTrigger) -> bool {
        match trigger {
            AgentTrigger::Email => true,
            AgentTrigger::Event => matches!(self, AgentActionKind::CreateTask | AgentActionKind::RunSkill),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentActionStatus {
    /// Waiting for the user's approval.
    Pending,
    Done,
    Failed,
    Rejected,
}

impl AgentActionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentActionStatus::Pending => "pending",
            AgentActionStatus::Done => "done",
            AgentActionStatus::Failed => "failed",
            AgentActionStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(AgentActionStatus::Pending),
            "done" => Some(AgentActionStatus::Done),
            "failed" => Some(AgentActionStatus::Failed),
            "rejected" => Some(AgentActionStatus::Rejected),
            _ => None,
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentRunStatus {
    /// At least one rule matched and its actions were planned.
    Matched,
    /// No rule matched. Kept so the trigger is never evaluated twice; not
    /// shown in the feed.
    NoMatch,
    Failed,
}

impl AgentRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentRunStatus::Matched => "matched",
            AgentRunStatus::NoMatch => "no_match",
            AgentRunStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "matched" => Some(AgentRunStatus::Matched),
            "no_match" => Some(AgentRunStatus::NoMatch),
            "failed" => Some(AgentRunStatus::Failed),
            _ => None,
        }
    }
}

/// A rule: when `match_prompt` describes the email or event, follow
/// `action_prompt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRule {
    pub id: String,
    pub name: String,
    pub trigger: AgentTrigger,
    /// `None` = every account.
    pub account_id: Option<String>,
    pub match_prompt: String,
    pub action_prompt: String,
    /// Every action of this rule waits for approval, local ones included.
    pub always_approve: bool,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What the rule editor sends to create or update a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuleInput {
    pub name: String,
    pub trigger: AgentTrigger,
    #[serde(default)]
    pub account_id: Option<String>,
    pub match_prompt: String,
    pub action_prompt: String,
    #[serde(default)]
    pub always_approve: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// One thing a run did or proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAction {
    pub id: String,
    pub run_id: String,
    /// `None` once the rule was deleted.
    pub rule_id: Option<String>,
    pub rule_name: String,
    pub kind: AgentActionKind,
    /// The model's argument: draft instructions, task title, skill name.
    pub detail: String,
    pub status: AgentActionStatus,
    pub requires_approval: bool,
    /// What the action produced: a draft id, a task id, a skill's output.
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    /// Subject or event title of the run it belongs to.
    pub run_title: String,
}

/// The agent's work on one email or event — one entry of the feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: String,
    pub account_id: String,
    pub trigger: AgentTrigger,
    /// The email id or the calendar event id.
    pub trigger_ref: String,
    /// Subject or event title.
    pub title: String,
    /// Sender of the email, organizer of the event.
    pub sender: String,
    pub status: AgentRunStatus,
    /// The model's analysis, shown as the agent's message.
    pub summary: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub actions: Vec<AgentAction>,
}

/// The time window a stats panel counts over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelWindow {
    /// Since local midnight.
    Today,
    Last7Days,
    Last30Days,
}

impl PanelWindow {
    pub fn as_str(self) -> &'static str {
        match self {
            PanelWindow::Today => "today",
            PanelWindow::Last7Days => "last7_days",
            PanelWindow::Last30Days => "last30_days",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "today" => Some(PanelWindow::Today),
            "last7_days" => Some(PanelWindow::Last7Days),
            "last30_days" => Some(PanelWindow::Last30Days),
            _ => None,
        }
    }
}

/// A counter the user defined with a prompt: the emails it describes that
/// arrived in `window`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPanel {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub window: PanelWindow,
    pub created_at: i64,
    /// Matching emails in the window, computed when listed.
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPanelInput {
    pub title: String,
    pub prompt: String,
    pub window: PanelWindow,
}

/// Preference: the agent is on.
pub const AGENT_ENABLED_PREF: &str = "agent.enabled";
/// Preference: unix seconds the agent was last turned on. Only mail that
/// arrives after it is evaluated, so turning the agent on never sweeps the
/// mailbox's history.
pub const AGENT_SINCE_PREF: &str = "agent.since";
/// Preference: minutes before an event's start the agent looks at it.
pub const AGENT_EVENT_LEAD_PREF: &str = "agent.event_lead_minutes";
