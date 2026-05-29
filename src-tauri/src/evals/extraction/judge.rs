// OpenRouter-backed LLM-as-a-judge for extraction evals.
//
// One prompt variant per ExtractionKind; both return a compact JSON with a
// 0–1 score, a verdict label, lists of missed / spurious items, and a
// rationale. Network/parse failures degrade to `error` without blocking the
// rest of the run.

use serde::{Deserialize, Serialize};

use crate::evals::extraction::ExtractionKind;
use crate::services::memory::extractor::{ExtractedFact, ExtractedTask};

const DEFAULT_JUDGE_MODEL: &str = "anthropic/claude-sonnet-4.5";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
const JUDGE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Default, Serialize)]
pub struct JudgeVerdict {
    pub score: Option<f64>,
    pub verdict: Option<String>, // "good" | "ok" | "poor" | "empty"
    pub missed: Vec<String>,
    pub spurious: Vec<String>,
    pub rationale: Option<String>,
    pub error: Option<String>,
}

pub struct ExtractionJudge {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl ExtractionJudge {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(JUDGE_TIMEOUT_SECS))
            .build()
            .expect("reqwest client");
        Self {
            client,
            api_key,
            model: model.unwrap_or_else(|| DEFAULT_JUDGE_MODEL.to_string()),
        }
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    pub async fn score_tasks(&self, email_summary: &EmailSummary, tasks: &[ExtractedTask]) -> JudgeVerdict {
        let user = build_task_prompt(email_summary, tasks);
        self.call(SYSTEM_TASKS, &user).await
    }

    pub async fn score_facts(&self, email_summary: &EmailSummary, facts: &[ExtractedFact]) -> JudgeVerdict {
        let user = build_fact_prompt(email_summary, facts);
        self.call(SYSTEM_FACTS, &user).await
    }

    pub async fn score(
        &self,
        kind: ExtractionKind,
        email: &EmailSummary,
        tasks: &[ExtractedTask],
        facts: &[ExtractedFact],
    ) -> JudgeVerdict {
        match kind {
            ExtractionKind::Tasks => self.score_tasks(email, tasks).await,
            ExtractionKind::Facts => self.score_facts(email, facts).await,
        }
    }

    async fn call(&self, system: &str, user: &str) -> JudgeVerdict {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user",   "content": user }
            ],
            "temperature": 0.0,
            "response_format": { "type": "json_object" }
        });

        let resp = match self
            .client
            .post(OPENROUTER_ENDPOINT)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://emailops.local/eval")
            .header("X-Title", "EmailOps Extraction Eval")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return JudgeVerdict {
                    error: Some(format!("judge HTTP error: {}", e)),
                    ..Default::default()
                }
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return JudgeVerdict {
                    error: Some(format!("judge body error: {}", e)),
                    ..Default::default()
                }
            }
        };
        if !status.is_success() {
            return JudgeVerdict {
                error: Some(format!("judge HTTP {}: {}", status.as_u16(), truncate(&text, 400))),
                ..Default::default()
            };
        }

        let outer: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                return JudgeVerdict {
                    error: Some(format!("judge outer parse: {e}")),
                    ..Default::default()
                }
            }
        };
        let content = outer
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        // Some judge models (notably Gemma-family and a few Claude tool-calls)
        // still emit ```json … ``` fences even when response_format=json_object
        // is requested. Strip any fences and trim to the outermost {...}.
        let cleaned = strip_json_fences(content);
        let parsed: JudgePayload = match serde_json::from_str(&cleaned) {
            Ok(v) => v,
            Err(e) => {
                return JudgeVerdict {
                    error: Some(format!("judge JSON parse: {e}; raw={}", truncate(content, 400))),
                    ..Default::default()
                }
            }
        };
        JudgeVerdict {
            score: parsed.score.map(|s| s.clamp(0.0, 1.0)),
            verdict: parsed.verdict,
            missed: parsed.missed.unwrap_or_default(),
            spurious: parsed.spurious.unwrap_or_default(),
            rationale: parsed.rationale,
            error: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct JudgePayload {
    score: Option<f64>,
    verdict: Option<String>,
    missed: Option<Vec<String>>,
    spurious: Option<Vec<String>>,
    rationale: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EmailSummary {
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub body_plain: String,
}

const SYSTEM_TASKS: &str = r#"You are grading an automated task extractor.
Given an email, the extractor produced a list of tasks (action items the user needs to do).
Respond ONLY with JSON: {
  "score": 0.0-1.0,
  "verdict": "good" | "ok" | "poor" | "empty",
  "missed": [strings of concrete tasks the extractor failed to surface],
  "spurious": [strings of tasks the extractor produced that aren't real action items for the recipient],
  "rationale": "one short paragraph"
}
Grading rules:
- Only count as valid tasks things the email asks the recipient to DO (send, review, pay, reply, schedule, decide).
- Calendar invites, marketing/newsletters, receipts, and "FYI" notifications are NOT tasks.
- If there are zero real tasks in the email and the extractor returned none, verdict="empty", score=1.0.
- If the email has clear tasks and the extractor missed them, score low (<0.4) and list them under "missed".
- If the extractor fabricated tasks, penalise and list them under "spurious".
"#;

const SYSTEM_FACTS: &str = r#"You are grading an automated memory-fact extractor.
Given an email, the extractor produced a list of facts (durable knowledge about people, contacts, projects, or the user themselves).
Respond ONLY with JSON: {
  "score": 0.0-1.0,
  "verdict": "good" | "ok" | "poor" | "empty",
  "missed": [strings of durable facts the extractor failed to capture],
  "spurious": [strings of facts that aren't durable or aren't grounded in the email],
  "rationale": "one short paragraph"
}
Grading rules:
- Good facts to KEEP:
  - Roles, titles, relationships, project names, stable identifiers.
  - User preferences and decisions expressed in the email ("I prefer morning calls", "we picked Vendor X").
  - Communication-style signals attributed to the user (tone, typical sign-off, response cadence).
  - Durable context about contacts/domains/projects (recurring schedules, known responsibilities).
- Bad facts to REJECT (count as spurious):
  - Envelope metadata ("Email was sent by X", "Subject is Y"). These are NEVER valid.
  - One-off chatter, transient statuses, marketing fluff.
  - Ephemeral details (single-meeting times, tracking codes, pleasantries).
- The fact schema includes "domain" (personal|professional) and "vigency" (atemporal|deciduous). Obvious misclassifications reduce precision.
- If the email carries no durable facts and the extractor returned none, verdict="empty", score=1.0.
- Grade on recall AND precision. Hallucinations and envelope-restating facts count heavily against the score.
"#;

fn build_task_prompt(email: &EmailSummary, tasks: &[ExtractedTask]) -> String {
    let task_json = serde_json::to_string_pretty(tasks).unwrap_or_else(|_| "[]".into());
    format!(
        r#"EMAIL:
From: {sender} <{sender_email}>
Subject: {subject}

{body}

EXTRACTED TASKS (JSON):
{tasks}

Grade the extraction per the rules. Return JSON only."#,
        sender = email.sender,
        sender_email = email.sender_email,
        subject = email.subject,
        body = truncate(&email.body_plain, 4000),
        tasks = task_json,
    )
}

fn build_fact_prompt(email: &EmailSummary, facts: &[ExtractedFact]) -> String {
    let fact_json = serde_json::to_string_pretty(facts).unwrap_or_else(|_| "[]".into());
    format!(
        r#"EMAIL:
From: {sender} <{sender_email}>
Subject: {subject}

{body}

EXTRACTED FACTS (JSON):
{facts}

Grade the extraction per the rules. Return JSON only."#,
        sender = email.sender,
        sender_email = email.sender_email,
        subject = email.subject,
        body = truncate(&email.body_plain, 4000),
        facts = fact_json,
    )
}

/// Strip ```json … ``` fences (or unlabelled triple-backticks) and trim the
/// result to the outermost `{` … `}` so `serde_json::from_str` succeeds even
/// when the judge model ignores `response_format=json_object`. Mirrors the
/// `extract_json` helper in `services::memory::extractor`.
fn strip_json_fences(text: &str) -> String {
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
