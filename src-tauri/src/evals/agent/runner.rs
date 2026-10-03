// Agent-decision runner: for each synthetic case, build the rules, panels and
// skills it names, render the email or event exactly as the agent does, call
// `decide` on the configured local model and score the result.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Deserialize;

use crate::db::Database;
use crate::evals::json_report::{ItemResult, JsonRunReport};
use crate::evals::{EvalError, EvalResult};
use crate::models::agent::{AgentPanel, AgentRule, AgentTrigger, PanelWindow};
use crate::services::agent::planner::{render_email_context, render_event_context, EventContext};
use crate::services::agent::runner::{decide, Decision};
use crate::services::skills::{Skill, SkillCatalog};

#[derive(Debug, Clone)]
pub struct AgentRunnerConfig {
    pub only_case: Option<String>,
    pub provider: String,
    pub model: String,
    pub out_dir: PathBuf,
    pub cases_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseRule {
    pub name: String,
    #[serde(rename = "match")]
    pub match_prompt: String,
    pub action: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseSkill {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseEmail {
    pub from: String,
    pub date: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseEvent {
    pub title: String,
    pub start: String,
    #[serde(default)]
    pub location: String,
    pub organizer: String,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub recent: Vec<String>,
}

/// One case: rules (keyed R1, R2… in order), panels (P1, P2…), an email or an
/// event, and the expected decision.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentCase {
    pub id: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub rules: Vec<CaseRule>,
    #[serde(default)]
    pub panels: Vec<String>,
    #[serde(default)]
    pub skills: Vec<CaseSkill>,
    #[serde(default)]
    pub email: Option<CaseEmail>,
    #[serde(default)]
    pub event: Option<CaseEvent>,
    /// The exact set of matching keys.
    pub expect_matches: Vec<String>,
    /// Action kinds (`draft_reply`, …) that must be among the planned ones.
    #[serde(default)]
    pub expect_actions: Vec<String>,
    /// Action kinds that must not be planned.
    #[serde(default)]
    pub expect_no_actions: Vec<String>,
}

pub fn load_cases(path: &Path) -> EvalResult<Vec<AgentCase>> {
    let text = std::fs::read_to_string(path)?;
    let cases: Vec<AgentCase> = serde_yaml::from_str(&text)?;
    for case in &cases {
        if case.email.is_some() == case.event.is_some() {
            return Err(EvalError::Config(format!(
                "case `{}` needs exactly one of `email` or `event`",
                case.id
            )));
        }
    }
    Ok(cases)
}

fn trigger_of(case: &AgentCase) -> AgentTrigger {
    if case.email.is_some() {
        AgentTrigger::Email
    } else {
        AgentTrigger::Event
    }
}

fn rules_of(case: &AgentCase) -> Vec<AgentRule> {
    case.rules
        .iter()
        .enumerate()
        .map(|(i, r)| AgentRule {
            id: format!("rule-{}", i + 1),
            name: r.name.clone(),
            trigger: trigger_of(case),
            account_id: None,
            match_prompt: r.match_prompt.clone(),
            action_prompt: r.action.clone(),
            always_approve: false,
            enabled: true,
            created_at: 0,
            updated_at: 0,
        })
        .collect()
}

fn panels_of(case: &AgentCase) -> Vec<AgentPanel> {
    case.panels
        .iter()
        .enumerate()
        .map(|(i, prompt)| AgentPanel {
            id: format!("panel-{}", i + 1),
            title: prompt.clone(),
            prompt: prompt.clone(),
            window: PanelWindow::Today,
            created_at: 0,
            count: 0,
        })
        .collect()
}

fn skills_of(case: &AgentCase) -> SkillCatalog {
    SkillCatalog {
        skills: case
            .skills
            .iter()
            .map(|s| Skill {
                name: s.name.clone(),
                description: s.description.clone(),
                body: s.description.clone(),
                path: PathBuf::new(),
                files: Vec::new(),
            })
            .collect(),
        errors: Vec::new(),
    }
}

fn context_of(case: &AgentCase) -> String {
    match (&case.email, &case.event) {
        (Some(e), _) => render_email_context(&e.from, &e.date, &e.subject, &e.body),
        (_, Some(ev)) => render_event_context(&EventContext {
            title: &ev.title,
            start: &ev.start,
            location: &ev.location,
            organizer: &ev.organizer,
            attendees: ev.attendees.iter().map(String::as_str).collect(),
            description: &ev.description,
            recent: ev.recent.clone(),
        }),
        (None, None) => String::new(),
    }
}

/// The failed checks of one case; empty = passed.
pub fn score(case: &AgentCase, decision: &Decision) -> Vec<String> {
    let mut failures = Vec::new();
    if decision.unreadable {
        failures.push("the match reply could not be read".to_string());
    }
    let got: BTreeSet<&str> = decision.matched.iter().map(|c| c.key.as_str()).collect();
    let want: BTreeSet<&str> = case.expect_matches.iter().map(String::as_str).collect();
    if got != want {
        failures.push(format!("matches: wanted {want:?}, got {got:?}"));
    }
    let planned: BTreeSet<&str> = decision
        .rules
        .iter()
        .flat_map(|r| r.actions.iter().map(|a| a.kind.as_str()))
        .collect();
    for kind in &case.expect_actions {
        if !planned.contains(kind.as_str()) {
            failures.push(format!("action {kind} missing (planned {planned:?})"));
        }
    }
    for kind in &case.expect_no_actions {
        if planned.contains(kind.as_str()) {
            failures.push(format!("action {kind} should not be planned"));
        }
    }
    if let Some(rule) = decision.rules.iter().find(|r| r.summary.is_none()) {
        failures.push(format!("the action reply for {} could not be read", rule.rule_name));
    }
    failures
}

/// Run the cases; returns (passed, total).
pub async fn run(cfg: AgentRunnerConfig) -> EvalResult<(usize, usize)> {
    let mut cases = load_cases(&cfg.cases_path)?;
    if let Some(id) = &cfg.only_case {
        cases.retain(|c| &c.id == id);
        if cases.is_empty() {
            return Err(EvalError::Config(format!("no agent case with id `{id}`")));
        }
    }

    // In-memory DB: the cases are synthetic and nothing of the user's mailbox
    // is read. The llamacpp backend finds GGUFs through `app_data_dir`.
    let db = std::sync::Arc::new(Database::new_for_testing()?);
    if let Some(dir) = crate::evals::draft_cases::app_data_dir() {
        db.set_preference("app_data_dir", &dir.to_string_lossy())?;
    }
    db.set_preference("ai_provider", &cfg.provider)?;
    db.set_preference("ai_model", &cfg.model)?;
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    crate::evals::shared::preflight_models(&db, [cfg.model.as_str()])?;
    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;
    let model = db.get_preference("ai_model")?.unwrap_or_default();
    println!("[agent-eval] model = {model}");
    println!("[agent-eval] running {} case(s)", cases.len());

    let mut report = JsonRunReport::new("agent_eval", &model);
    let mut passed = 0;
    for case in &cases {
        let started = Instant::now();
        let decision = decide(
            provider.as_ref(),
            &skills_of(case),
            &rules_of(case),
            &panels_of(case),
            trigger_of(case),
            "eval-account",
            &context_of(case),
        )
        .await?;
        let latency_ms = started.elapsed().as_millis();
        let failures = score(case, &decision);
        let ok = failures.is_empty();
        passed += usize::from(ok);
        println!(
            "[agent-eval] {} {} ({latency_ms}ms)",
            if ok { "OK  " } else { "FAIL" },
            case.id
        );
        for failure in &failures {
            println!("[agent-eval]      {failure}");
        }
        for rule in &decision.rules {
            println!(
                "[agent-eval]      {}: {}",
                rule.rule_name,
                rule.summary.as_deref().unwrap_or("<unreadable>")
            );
        }
        report.push(ItemResult {
            id: case.id.clone(),
            passed: ok,
            score: Some(if ok { 1.0 } else { 0.0 }),
            detail: if ok { case.note.clone() } else { failures.join(" · ") },
            evidence: None,
        });
    }
    report.write(&cfg.out_dir)?;
    println!("[agent-eval] {passed}/{} cases passed", cases.len());
    Ok((passed, cases.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agent::planner::{Criterion, CriterionTarget, PlannedAction};
    use crate::services::agent::runner::RuleDecision;

    fn case() -> AgentCase {
        serde_yaml::from_str(
            r#"
id: c
rules: [{name: Support, match: "help requests", action: "reply"}]
panels: ["support"]
email: {from: "a@example.com", date: "2026-10-03", subject: "Help", body: "It broke"}
expect_matches: [R1, P1]
expect_actions: [draft_reply]
expect_no_actions: [archive]
"#,
        )
        .unwrap()
    }

    fn decision(keys: &[&str], kinds: &[crate::models::agent::AgentActionKind]) -> Decision {
        Decision {
            matched: keys
                .iter()
                .map(|k| Criterion {
                    key: (*k).into(),
                    target: CriterionTarget::Rule((*k).into()),
                    text: String::new(),
                })
                .collect(),
            unreadable: false,
            rules: vec![RuleDecision {
                rule_id: "rule-1".into(),
                rule_name: "Support".into(),
                summary: Some("s".into()),
                actions: kinds
                    .iter()
                    .map(|k| PlannedAction {
                        kind: *k,
                        detail: String::new(),
                        requires_approval: false,
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn a_case_passes_with_the_exact_matches_and_the_wanted_actions() {
        use crate::models::agent::AgentActionKind::*;
        assert!(score(&case(), &decision(&["P1", "R1"], &[DraftReply, CreateTask])).is_empty());
    }

    #[test]
    fn a_case_fails_on_a_missing_match_a_missing_action_or_a_forbidden_one() {
        use crate::models::agent::AgentActionKind::*;
        assert_eq!(score(&case(), &decision(&["R1"], &[DraftReply])).len(), 1);
        assert_eq!(score(&case(), &decision(&["R1", "P1"], &[CreateTask])).len(), 1);
        assert_eq!(
            score(&case(), &decision(&["R1", "P1"], &[DraftReply, Archive])).len(),
            1
        );
    }

    #[test]
    fn the_shipped_cases_load_and_each_has_one_trigger() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evals/agent/cases.yaml");
        let cases = load_cases(&path).unwrap();
        assert!(cases.len() >= 6);
        let ids: BTreeSet<_> = cases.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), cases.len(), "case ids are unique");
    }
}
