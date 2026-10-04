//! The agent's decisions, with no I/O: which criteria an email or event is
//! checked against, the two prompts and the JSON shapes their replies must
//! take, how a reply becomes a list of actions, which actions wait for the
//! user, which events are due, and where a panel's window starts.

use crate::ai::json_shape::JsonShape;
use crate::models::agent::{AgentActionKind, AgentPanel, AgentRule, AgentTrigger, PanelWindow};
use crate::models::CalendarEvent;

/// Most actions one rule may take on one email or event.
pub const MAX_ACTIONS_PER_RULE: usize = 3;
const SUMMARY_MAX_CHARS: usize = 600;
const DETAIL_MAX_CHARS: usize = 400;

/// What a criterion decides when it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CriterionTarget {
    Rule(String),
    Panel(String),
}

/// One yes/no question the match call answers about an email or event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Criterion {
    /// Short label the model answers with (`R1`, `P2`): ids would cost tokens
    /// and invite typos.
    pub key: String,
    pub target: CriterionTarget,
    pub text: String,
}

/// The criteria to check a trigger of `account_id` against: the enabled rules
/// of that trigger that cover the account and, for an email, every panel.
pub fn plan_criteria(
    rules: &[AgentRule],
    panels: &[AgentPanel],
    trigger: AgentTrigger,
    account_id: &str,
) -> Vec<Criterion> {
    let rules = rules.iter().filter(|r| {
        r.enabled
            && r.trigger == trigger
            && r.account_id.as_deref().is_none_or(|a| a == account_id)
            && !r.match_prompt.trim().is_empty()
    });
    let mut criteria: Vec<Criterion> = rules
        .enumerate()
        .map(|(i, r)| Criterion {
            key: format!("R{}", i + 1),
            target: CriterionTarget::Rule(r.id.clone()),
            text: r.match_prompt.trim().to_string(),
        })
        .collect();
    if trigger == AgentTrigger::Email {
        let panels = panels.iter().filter(|p| !p.prompt.trim().is_empty());
        criteria.extend(panels.enumerate().map(|(i, p)| Criterion {
            key: format!("P{}", i + 1),
            target: CriterionTarget::Panel(p.id.clone()),
            text: p.prompt.trim().to_string(),
        }));
    }
    criteria
}

/// The email as the model reads it.
pub fn render_email_context(from: &str, date: &str, subject: &str, body: &str) -> String {
    format!("EMAIL\nFrom: {from}\nDate: {date}\nSubject: {subject}\n\n{body}")
}

/// The upcoming event as the model reads it, with the latest messages
/// exchanged with its attendees (`recent`: one line each).
pub struct EventContext<'a> {
    pub title: &'a str,
    pub start: &'a str,
    pub location: &'a str,
    pub organizer: &'a str,
    pub attendees: Vec<&'a str>,
    pub description: &'a str,
    pub recent: Vec<String>,
}

pub fn render_event_context(event: &EventContext<'_>) -> String {
    let mut out = format!(
        "CALENDAR EVENT\nTitle: {}\nStarts: {}\nLocation: {}\nOrganizer: {}\nAttendees: {}\n\n{}",
        event.title,
        event.start,
        event.location,
        event.organizer,
        event.attendees.join(", "),
        event.description
    );
    if !event.recent.is_empty() {
        out.push_str("\n\nRecent messages with the attendees:\n");
        out.push_str(&event.recent.join("\n"));
    }
    out
}

const MATCH_INSTRUCTIONS: &str = "You sort incoming items for an email assistant. \
You get numbered criteria and one email or calendar event. Decide each criterion on its own: \
several can describe the same item, or none. Judge by meaning, not by exact words.\n\
Reply with JSON only: one field per criterion key, in order, each with a short reason \
and then the answer, \"yes\" or \"no\". \
Example: {\"R1\": {\"reason\": \"a newsletter, nobody asks for help\", \"answer\": \"no\"}, \
\"P1\": {\"reason\": \"an invoice from a supplier\", \"answer\": \"yes\"}}\n\n";

/// The match call: a fixed instruction prefix (kept decoded between calls by
/// backends with a prefix cache) and the per-trigger suffix with the criteria
/// and the email or event.
pub fn match_prompt(criteria: &[Criterion], context: &str) -> (String, String) {
    let list: Vec<String> = criteria.iter().map(|c| format!("{}: {}", c.key, c.text)).collect();
    let suffix = format!("CRITERIA\n{}\n\n{context}\n\nJSON:", list.join("\n"));
    (MATCH_INSTRUCTIONS.to_string(), suffix)
}

/// `{"R1": {"reason": "…", "answer": "yes"|"no"}, …}` — one explicit answer
/// per criterion, so a small model cannot stop at the first one that fits, and
/// a short reason written before it (the 4B model answered "no" to a rule that
/// plainly matched when a panel beside it already said "yes"; with the reason
/// first it judges each one).
pub fn match_shape(criteria: &[Criterion]) -> JsonShape {
    JsonShape::object(
        criteria
            .iter()
            .map(|c| {
                (
                    c.key.as_str(),
                    JsonShape::object(vec![
                        ("reason", JsonShape::String { max_len: 100 }),
                        ("answer", JsonShape::one_of(&["yes", "no"])),
                    ]),
                )
            })
            .collect(),
    )
}

/// Tokens the match reply needs: a short reason and an answer per criterion.
pub fn match_max_tokens(criteria: &[Criterion]) -> u32 {
    32 + 50 * criteria.len() as u32
}

/// The outermost `{…}` of a reply that may carry prose or code fences.
fn json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    (end > start).then(|| &raw[start..=end])
}

/// The criteria whose `answer` is "yes", in criteria order. A criterion
/// the reply leaves out does not match. `None` when the reply is not a JSON
/// object.
pub fn parse_matches<'a>(raw: &str, criteria: &'a [Criterion]) -> Option<Vec<&'a Criterion>> {
    let reply: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json_object(raw)?).ok()?;
    Some(
        criteria
            .iter()
            .filter(|c| {
                reply
                    .get(&c.key)
                    .and_then(|v| v.get("answer"))
                    .and_then(|a| a.as_str())
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"))
            })
            .collect(),
    )
}

/// The action kinds a rule may use for this trigger. `run_skill` only when
/// the user has skills to run.
pub fn offered_kinds(trigger: AgentTrigger, has_skills: bool) -> Vec<AgentActionKind> {
    AgentActionKind::ALL
        .into_iter()
        .filter(|k| k.applies_to(trigger))
        .filter(|k| has_skills || *k != AgentActionKind::RunSkill)
        .collect()
}

fn kind_help(kind: AgentActionKind) -> &'static str {
    match kind {
        AgentActionKind::DraftReply => "write a reply draft; detail = what the reply must say",
        AgentActionKind::CreateTask => "add a task; detail = the task title",
        AgentActionKind::RunSkill => "run one of the skills below; detail = the skill name",
        AgentActionKind::MarkRead => "mark the email as read; detail = \"\"",
        AgentActionKind::Archive => "archive the conversation; detail = \"\"",
        AgentActionKind::Star => "star the conversation; detail = \"\"",
    }
}

const ACTION_INSTRUCTIONS: &str = "You are an email assistant acting on the user's rule. \
You get the rule's instructions, the actions you may take and one email or calendar event. \
Take only the actions the instructions ask for; none is a valid choice. You never send email.\n\
Reply with JSON only: {\"summary\": \"two or three sentences: what the item is about and which \
actions you chose — say you propose them, the user approves archiving, starring and marking as read\", \"actions\": [{\"action\": \"<action>\", \"detail\": \"<detail>\"}]}. \
Write the summary and the details in the language of the rule's instructions.\n\n";

/// The action call for one matched rule. `skills` lists `(name, description)`.
pub fn action_prompt(
    rule: &AgentRule,
    context: &str,
    kinds: &[AgentActionKind],
    skills: &[(&str, &str)],
) -> (String, String) {
    let actions: Vec<String> = kinds
        .iter()
        .map(|k| format!("- {}: {}", k.as_str(), kind_help(*k)))
        .collect();
    let mut suffix = format!(
        "RULE: {}\nINSTRUCTIONS\n{}\n\nACTIONS\n{}\n",
        rule.name,
        rule.action_prompt.trim(),
        actions.join("\n")
    );
    if kinds.contains(&AgentActionKind::RunSkill) && !skills.is_empty() {
        let list: Vec<String> = skills.iter().map(|(n, d)| format!("- {n}: {d}")).collect();
        suffix.push_str(&format!("\nSKILLS\n{}\n", list.join("\n")));
    }
    suffix.push_str(&format!("\n{context}\n\nJSON:"));
    (ACTION_INSTRUCTIONS.to_string(), suffix)
}

/// `{"summary": "...", "actions": [{"action": <kind>, "detail": "..."}]}`.
pub fn action_shape(kinds: &[AgentActionKind]) -> JsonShape {
    let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    JsonShape::object(vec![
        (
            "summary",
            JsonShape::String {
                max_len: SUMMARY_MAX_CHARS,
            },
        ),
        (
            "actions",
            JsonShape::array(
                JsonShape::object(vec![
                    ("action", JsonShape::one_of(&names)),
                    (
                        "detail",
                        JsonShape::String {
                            max_len: DETAIL_MAX_CHARS,
                        },
                    ),
                ]),
                0,
                MAX_ACTIONS_PER_RULE,
            ),
        ),
    ])
}

/// The action call's reply as written, before validation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ActionReply {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub actions: Vec<RawAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RawAction {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub detail: String,
}

pub fn parse_action_reply(raw: &str) -> Option<ActionReply> {
    serde_json::from_str(json_object(raw)?).ok()
}

/// An action ready to store: run now, or wait for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub kind: AgentActionKind,
    pub detail: String,
    pub requires_approval: bool,
}

/// Turn the reply into actions: only offered kinds, a skill that exists, a
/// task with a title; no duplicates; at most [`MAX_ACTIONS_PER_RULE`].
pub fn plan_actions(
    reply: &ActionReply,
    rule: &AgentRule,
    offered: &[AgentActionKind],
    skill_names: &[&str],
) -> Vec<PlannedAction> {
    let mut planned: Vec<PlannedAction> = Vec::new();
    for raw in &reply.actions {
        let Some(kind) = AgentActionKind::parse(raw.action.trim()) else {
            continue;
        };
        let detail = raw.detail.trim().to_string();
        let usable = offered.contains(&kind)
            && match kind {
                AgentActionKind::CreateTask => !detail.is_empty(),
                AgentActionKind::RunSkill => skill_names.contains(&detail.as_str()),
                _ => true,
            };
        if !usable || planned.iter().any(|p| p.kind == kind) {
            continue;
        }
        planned.push(PlannedAction {
            kind,
            detail,
            requires_approval: requires_approval(kind, rule.always_approve),
        });
        if planned.len() == MAX_ACTIONS_PER_RULE {
            break;
        }
    }
    planned
}

/// Whether an action waits for the user's approval.
pub fn requires_approval(kind: AgentActionKind, always_approve: bool) -> bool {
    always_approve || kind.touches_mailbox()
}

/// Events the agent looks at now: timed, not cancelled, starting within
/// `lead_secs` from `now`.
pub fn due_events(events: &[CalendarEvent], now: i64, lead_secs: i64) -> Vec<&CalendarEvent> {
    events
        .iter()
        .filter(|e| !e.is_all_day && e.status != "cancelled")
        .filter(|e| e.start_time >= now && e.start_time <= now + lead_secs)
        .collect()
}

/// First second a panel counts, for a user whose clock is `utc_offset_secs`
/// ahead of UTC.
pub fn window_start(window: PanelWindow, now: i64, utc_offset_secs: i32) -> i64 {
    const DAY: i64 = 86_400;
    match window {
        PanelWindow::Today => {
            let local = now + i64::from(utc_offset_secs);
            local - local.rem_euclid(DAY) - i64::from(utc_offset_secs)
        }
        PanelWindow::Last7Days => now - 7 * DAY,
        PanelWindow::Last30Days => now - 30 * DAY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, trigger: AgentTrigger, account: Option<&str>, enabled: bool) -> AgentRule {
        AgentRule {
            id: id.into(),
            name: format!("rule {id}"),
            trigger,
            account_id: account.map(Into::into),
            match_prompt: format!("matches {id}"),
            action_prompt: "draft a polite reply".into(),
            always_approve: false,
            enabled,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn panel(id: &str) -> AgentPanel {
        AgentPanel {
            id: id.into(),
            title: format!("panel {id}"),
            prompt: format!("counts {id}"),
            window: PanelWindow::Today,
            created_at: 0,
            count: 0,
        }
    }

    #[test]
    fn an_email_is_checked_against_enabled_email_rules_of_its_account_and_every_panel() {
        let rules = [
            rule("all", AgentTrigger::Email, None, true),
            rule("mine", AgentTrigger::Email, Some("acc-1"), true),
            rule("other", AgentTrigger::Email, Some("acc-2"), true),
            rule("off", AgentTrigger::Email, None, false),
            rule("event", AgentTrigger::Event, None, true),
        ];
        let criteria = plan_criteria(&rules, &[panel("p")], AgentTrigger::Email, "acc-1");
        let targets: Vec<_> = criteria.iter().map(|c| (c.key.as_str(), c.target.clone())).collect();
        assert_eq!(
            targets,
            vec![
                ("R1", CriterionTarget::Rule("all".into())),
                ("R2", CriterionTarget::Rule("mine".into())),
                ("P1", CriterionTarget::Panel("p".into())),
            ]
        );
        assert_eq!(criteria[0].text, "matches all");
        assert_eq!(criteria[2].text, "counts p");
    }

    #[test]
    fn an_event_is_checked_against_event_rules_only() {
        let rules = [
            rule("mail", AgentTrigger::Email, None, true),
            rule("meet", AgentTrigger::Event, None, true),
        ];
        let criteria = plan_criteria(&rules, &[panel("p")], AgentTrigger::Event, "acc-1");
        assert_eq!(criteria.len(), 1);
        assert_eq!(criteria[0].target, CriterionTarget::Rule("meet".into()));
    }

    fn criteria() -> Vec<Criterion> {
        plan_criteria(
            &[
                rule("a", AgentTrigger::Email, None, true),
                rule("b", AgentTrigger::Email, None, true),
            ],
            &[panel("p")],
            AgentTrigger::Email,
            "acc-1",
        )
    }

    fn keys(found: &[&Criterion]) -> Vec<String> {
        found.iter().map(|c| c.key.clone()).collect()
    }

    #[test]
    fn the_email_context_carries_sender_date_subject_and_body() {
        let text = render_email_context(
            "Ana <ana@example.com>",
            "2026-10-03",
            "Login fails",
            "I cannot sign in.",
        );
        for part in [
            "Ana <ana@example.com>",
            "2026-10-03",
            "Login fails",
            "I cannot sign in.",
        ] {
            assert!(text.contains(part), "{part} missing from {text}");
        }
    }

    #[test]
    fn the_event_context_lists_attendees_and_recent_messages() {
        let text = render_event_context(&EventContext {
            title: "Quarterly review",
            start: "2026-10-03 10:00",
            location: "Room 2",
            organizer: "boss@example.com",
            attendees: vec!["ana@example.com", "ben@example.com"],
            description: "Bring the numbers",
            recent: vec!["2026-10-01 ana@example.com: Numbers attached".into()],
        });
        for part in [
            "Quarterly review",
            "2026-10-03 10:00",
            "Room 2",
            "boss@example.com",
            "ana@example.com, ben@example.com",
            "Bring the numbers",
            "Numbers attached",
        ] {
            assert!(text.contains(part), "{part} missing from {text}");
        }
    }

    #[test]
    fn the_match_prefix_is_the_same_for_every_trigger_and_the_suffix_lists_each_criterion() {
        let c = criteria();
        let (prefix_a, suffix_a) = match_prompt(&c, "email one");
        let (prefix_b, _) = match_prompt(&c[..1], "email two");
        assert_eq!(prefix_a, prefix_b, "the cached prefix must not vary");
        assert!(!prefix_a.is_empty());
        assert!(suffix_a.contains("R1: matches a"));
        assert!(
            prefix_a.contains("\"yes\""),
            "the instructions ask for a yes/no per criterion"
        );
        assert!(suffix_a.contains("P1: counts p"));
        assert!(suffix_a.contains("email one"));
    }

    #[test]
    fn the_match_shape_asks_yes_or_no_for_every_criterion_in_order() {
        let shape = match_shape(&criteria());
        let answer = || {
            JsonShape::object(vec![
                ("reason", JsonShape::String { max_len: 100 }),
                ("answer", JsonShape::one_of(&["yes", "no"])),
            ])
        };
        assert_eq!(
            shape,
            JsonShape::object(vec![("R1", answer()), ("R2", answer()), ("P1", answer())])
        );
    }

    #[test]
    fn the_criteria_answered_yes_match_in_criteria_order() {
        let c = criteria();
        let found = parse_matches(
            r#"{"P1": {"reason": "a", "answer": "yes"}, "R1": {"reason": "b", "answer": "Yes"},
                "R2": {"reason": "c", "answer": "no"}, "R9": {"reason": "d", "answer": "yes"}}"#,
            &c,
        )
        .unwrap();
        assert_eq!(keys(&found), vec!["R1", "P1"]);
    }

    #[test]
    fn a_match_reply_wrapped_in_prose_or_fences_still_parses() {
        let c = criteria();
        let found = parse_matches(
            "Sure!\n```json\n{\"R1\": {\"answer\": \"no\"}, \"R2\": {\"answer\": \"yes\"}}\n```",
            &c,
        )
        .unwrap();
        assert_eq!(keys(&found), vec!["R2"]);
        assert!(parse_matches(
            r#"{"R1": {"answer": "no"}, "R2": {"answer": "no"}, "P1": {"answer": "no"}}"#,
            &c
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn a_criterion_left_out_or_without_an_answer_does_not_match() {
        let c = criteria();
        let found = parse_matches(r#"{"R2": {"answer": "yes"}, "R1": "yes"}"#, &c).unwrap();
        assert_eq!(keys(&found), vec!["R2"]);
    }

    #[test]
    fn a_match_reply_that_is_not_json_is_none() {
        assert!(parse_matches("R1 matches", &criteria()).is_none());
    }

    #[test]
    fn an_email_rule_is_offered_every_kind_and_an_event_rule_only_tasks_and_skills() {
        use AgentActionKind::*;
        assert_eq!(
            offered_kinds(AgentTrigger::Email, true),
            vec![DraftReply, CreateTask, RunSkill, MarkRead, Archive, Star]
        );
        assert_eq!(offered_kinds(AgentTrigger::Event, true), vec![CreateTask, RunSkill]);
    }

    #[test]
    fn run_skill_is_not_offered_without_skills() {
        assert!(!offered_kinds(AgentTrigger::Email, false).contains(&AgentActionKind::RunSkill));
        assert_eq!(
            offered_kinds(AgentTrigger::Event, false),
            vec![AgentActionKind::CreateTask]
        );
    }

    #[test]
    fn the_action_prompt_carries_the_rule_the_offered_actions_and_the_skills() {
        let r = rule("a", AgentTrigger::Email, None, true);
        let kinds = offered_kinds(AgentTrigger::Email, true);
        let (prefix, suffix) = action_prompt(&r, "the email", &kinds, &[("triage", "Sort support mail")]);
        let (prefix_other, _) = action_prompt(&r, "another", &kinds[..1], &[]);
        assert_eq!(prefix, prefix_other, "the cached prefix must not vary");
        assert!(suffix.contains("draft a polite reply"));
        assert!(suffix.contains("the email"));
        assert!(suffix.contains("draft_reply"));
        assert!(suffix.contains("triage: Sort support mail"));
        let (_, without_skills) = action_prompt(&r, "x", &offered_kinds(AgentTrigger::Email, false), &[]);
        assert!(!without_skills.contains("run_skill"));
    }

    #[test]
    fn the_action_shape_names_only_offered_kinds() {
        let shape = action_shape(&[AgentActionKind::CreateTask]);
        assert_eq!(
            shape,
            JsonShape::object(vec![
                ("summary", JsonShape::String { max_len: 600 }),
                (
                    "actions",
                    JsonShape::array(
                        JsonShape::object(vec![
                            ("action", JsonShape::one_of(&["create_task"])),
                            ("detail", JsonShape::String { max_len: 400 }),
                        ]),
                        0,
                        MAX_ACTIONS_PER_RULE
                    )
                ),
            ])
        );
    }

    #[test]
    fn an_action_reply_parses_with_missing_fields_defaulted() {
        let reply =
            parse_action_reply(r#"{"summary": "Support request", "actions": [{"action": "mark_read"}]}"#).unwrap();
        assert_eq!(reply.summary, "Support request");
        assert_eq!(reply.actions[0].action, "mark_read");
        assert_eq!(reply.actions[0].detail, "");
        assert!(parse_action_reply("no json here").is_none());
    }

    fn reply(actions: &[(&str, &str)]) -> ActionReply {
        ActionReply {
            summary: "s".into(),
            actions: actions
                .iter()
                .map(|(a, d)| RawAction {
                    action: (*a).into(),
                    detail: (*d).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn local_actions_run_now_and_mailbox_changes_wait_for_approval() {
        let r = rule("a", AgentTrigger::Email, None, true);
        let planned = plan_actions(
            &reply(&[("draft_reply", "Say thanks"), ("archive", "")]),
            &r,
            &offered_kinds(AgentTrigger::Email, false),
            &[],
        );
        assert_eq!(
            planned,
            vec![
                PlannedAction {
                    kind: AgentActionKind::DraftReply,
                    detail: "Say thanks".into(),
                    requires_approval: false
                },
                PlannedAction {
                    kind: AgentActionKind::Archive,
                    detail: String::new(),
                    requires_approval: true
                },
            ]
        );
    }

    #[test]
    fn a_rule_that_always_asks_makes_every_action_wait() {
        let mut r = rule("a", AgentTrigger::Email, None, true);
        r.always_approve = true;
        let planned = plan_actions(
            &reply(&[("create_task", "Call back")]),
            &r,
            &offered_kinds(AgentTrigger::Email, false),
            &[],
        );
        assert!(planned[0].requires_approval);
    }

    #[test]
    fn actions_not_offered_unknown_or_incomplete_are_dropped() {
        let r = rule("a", AgentTrigger::Event, None, true);
        let planned = plan_actions(
            &reply(&[
                ("draft_reply", "not for events"),
                ("send_email", "never"),
                ("create_task", "   "),
                ("run_skill", "missing-skill"),
                ("run_skill", "briefing"),
            ]),
            &r,
            &offered_kinds(AgentTrigger::Event, true),
            &["briefing"],
        );
        assert_eq!(
            planned,
            vec![PlannedAction {
                kind: AgentActionKind::RunSkill,
                detail: "briefing".into(),
                requires_approval: false
            }]
        );
    }

    #[test]
    fn a_repeated_action_is_kept_once_and_the_list_is_capped() {
        let r = rule("a", AgentTrigger::Email, None, true);
        let planned = plan_actions(
            &reply(&[
                ("mark_read", ""),
                ("mark_read", ""),
                ("star", ""),
                ("create_task", "One"),
                ("create_task", "Two"),
            ]),
            &r,
            &offered_kinds(AgentTrigger::Email, false),
            &[],
        );
        let kinds: Vec<_> = planned.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AgentActionKind::MarkRead,
                AgentActionKind::Star,
                AgentActionKind::CreateTask
            ]
        );
    }

    #[test]
    fn only_mailbox_changes_or_an_always_ask_rule_need_approval() {
        use AgentActionKind::*;
        for kind in [DraftReply, CreateTask, RunSkill] {
            assert!(!requires_approval(kind, false), "{kind:?}");
            assert!(requires_approval(kind, true), "{kind:?}");
        }
        for kind in [MarkRead, Archive, Star] {
            assert!(requires_approval(kind, false), "{kind:?}");
        }
    }

    fn event(id: &str, start: i64) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            account_id: "acc-1".into(),
            provider_event_id: id.into(),
            calendar_id: "primary".into(),
            title: id.into(),
            description: String::new(),
            location: String::new(),
            start_time: start,
            end_time: start + 1800,
            is_all_day: false,
            timezone: String::new(),
            organizer: String::new(),
            attendees: vec![],
            meeting_link: None,
            meeting_platform: None,
            status: "confirmed".into(),
            html_link: None,
            notified_at: None,
            recurring_event_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn an_event_is_due_from_lead_time_before_its_start_until_it_starts() {
        let now = 10_000;
        let mut all_day = event("all-day", now + 60);
        all_day.is_all_day = true;
        let mut cancelled = event("cancelled", now + 60);
        cancelled.status = "cancelled".into();
        let events = [
            event("started", now - 1),
            event("now", now),
            event("soon", now + 600),
            event("later", now + 601),
            all_day,
            cancelled,
        ];
        let due: Vec<_> = due_events(&events, now, 600).iter().map(|e| e.id.clone()).collect();
        assert_eq!(due, vec!["now", "soon"]);
    }

    #[test]
    fn a_panel_window_starts_at_local_midnight_or_n_days_back() {
        // 2026-10-03 10:00 UTC; Madrid is UTC+2 → local midnight is 22:00 UTC the day before.
        let now = 1_791_021_600;
        let midnight_utc = 1_790_985_600;
        assert_eq!(window_start(PanelWindow::Today, now, 0), midnight_utc);
        assert_eq!(window_start(PanelWindow::Today, now, 7200), midnight_utc - 7200);
        assert_eq!(window_start(PanelWindow::Last7Days, now, 7200), now - 7 * 86_400);
        assert_eq!(window_start(PanelWindow::Last30Days, now, 0), now - 30 * 86_400);
    }

    #[test]
    fn a_rule_with_a_blank_match_prompt_is_never_checked() {
        let mut blank = rule("blank", AgentTrigger::Email, None, true);
        blank.match_prompt = "   ".into();
        assert!(plan_criteria(&[blank], &[], AgentTrigger::Email, "acc-1").is_empty());
    }
}
