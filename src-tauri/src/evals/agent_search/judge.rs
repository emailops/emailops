// Pool-based relevance judge.
//
// Given a query, a per-query rubric, and a list of candidate emails, asks an
// OpenRouter model (default Claude Sonnet 4.5) to rate each candidate's
// relevance on a 0/1/2 scale (irrelevant / partial / clearly relevant).
//
// One HTTP request per candidate keeps the prompt focused and the judge calls
// independent — easier to debug than batched judging when a rating looks
// surprising. With 4 queries × ~30-pool size that's ~120 calls per eval,
// roughly $0.5 on Sonnet 4.5.
//
// Errors are captured per-candidate (so a transient flake doesn't abort the
// whole run) and surfaced in the HTML report.

use serde::{Deserialize, Serialize};

use crate::evals::EvalResult;
use crate::services::agent_search::AgentSearchHit;

const DEFAULT_JUDGE_MODEL: &str = "anthropic/claude-sonnet-4.5";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
const JUDGE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Judgment {
    /// 0 = irrelevant, 1 = partially relevant, 2 = clearly relevant.
    pub score: i32,
    pub rationale: String,
    pub error: Option<String>,
}

impl Judgment {
    pub fn is_relevant(&self) -> bool {
        self.score >= 1
    }
    pub fn is_clearly_relevant(&self) -> bool {
        self.score >= 2
    }
}

pub struct PoolJudge {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl PoolJudge {
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

    /// Score a single candidate. Never returns Err — judge errors are encoded
    /// in `Judgment::error` so the run can continue.
    pub async fn score(&self, query: &str, criteria: &str, hit: &AgentSearchHit) -> EvalResult<Judgment> {
        let prompt = build_prompt(query, criteria, hit);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": JUDGE_SYSTEM },
                { "role": "user", "content": prompt }
            ],
            "temperature": 0.0,
            "response_format": { "type": "json_object" }
        });

        let resp = match self
            .client
            .post(OPENROUTER_ENDPOINT)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://emailops.local/eval")
            .header("X-Title", "EmailOps Agent Search Eval")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(Judgment {
                    score: 0,
                    rationale: String::new(),
                    error: Some(format!("HTTP error: {}", e)),
                });
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(Judgment {
                    score: 0,
                    rationale: String::new(),
                    error: Some(format!("body read failed: {}", e)),
                });
            }
        };

        if !status.is_success() {
            return Ok(Judgment {
                score: 0,
                rationale: String::new(),
                error: Some(format!("HTTP {}: {}", status, truncate(&text, 400))),
            });
        }

        Ok(parse_judge_response(&text))
    }
}

#[derive(Deserialize)]
struct ORResp {
    choices: Vec<ORChoice>,
}
#[derive(Deserialize)]
struct ORChoice {
    message: ORMsg,
}
#[derive(Deserialize)]
struct ORMsg {
    content: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Payload {
    score: Option<i32>,
    rationale: Option<String>,
}

fn parse_judge_response(raw: &str) -> Judgment {
    let envelope: ORResp = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            return Judgment {
                score: 0,
                rationale: String::new(),
                error: Some(format!("malformed envelope: {} — raw: {}", e, truncate(raw, 400))),
            }
        }
    };
    let content = match envelope.choices.first() {
        Some(c) => &c.message.content,
        None => {
            return Judgment {
                score: 0,
                rationale: String::new(),
                error: Some("no choices in response".into()),
            }
        }
    };
    let stripped = strip_code_fence(content);
    let payload: Payload = match serde_json::from_str(&stripped) {
        Ok(p) => p,
        Err(e) => {
            return Judgment {
                score: 0,
                rationale: String::new(),
                error: Some(format!("malformed JSON: {} — content: {}", e, truncate(content, 400))),
            }
        }
    };
    Judgment {
        score: payload.score.unwrap_or(0).clamp(0, 2),
        rationale: payload.rationale.unwrap_or_default(),
        error: None,
    }
}

const JUDGE_SYSTEM: &str = "You are evaluating whether emails match a user's search query. \
The user is a freelance CTO who manages multiple client projects, sends proposals/invoices to \
clients, and receives invoices from vendors. Emails may be in any of the supported UI \
languages (English, Spanish, French, German); evaluate relevance on meaning, not on which \
language the email is written in. Be strict and follow the rubric. Return STRICT JSON only — \
no prose, no code fences.";

fn build_prompt(query: &str, criteria: &str, hit: &AgentSearchHit) -> String {
    let direction = if hit.sent_by_user {
        "SENT BY USER (user is the sender)"
    } else {
        "RECEIVED BY USER (user is a recipient)"
    };
    format!(
        "QUERY: {query}\n\n\
RELEVANCE RUBRIC:\n{criteria}\n\n\
CANDIDATE EMAIL:\n\
- direction: {direction}\n\
- from: {sender} <{sender_email}>\n\
- to: {recipients}\n\
- subject: {subject}\n\
- snippet (first ~600 chars):\n{snippet}\n\n\
Rate this email's relevance to the query, applying the rubric strictly:\n\
  2 = clearly matches (right topic AND, if the rubric specifies direction, right direction)\n\
  1 = plausibly matches but ambiguous (e.g. right topic but direction unclear; \
or relevant context but not the asked-for artifact)\n\
  0 = does not match the rubric (wrong topic, wrong direction, unrelated)\n\n\
Return STRICT JSON:\n\
{{ \"score\": 0|1|2, \"rationale\": \"<=2 sentences explaining the decision\" }}",
        query = query,
        criteria = criteria,
        direction = direction,
        sender = truncate(&hit.sender, 80),
        sender_email = truncate(&hit.sender_email, 120),
        recipients = truncate(&hit.recipients.join(", "), 200),
        subject = truncate(&hit.subject, 200),
        snippet = truncate(&hit.snippet, 1200),
    )
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim().to_string();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}
