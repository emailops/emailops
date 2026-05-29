// OpenRouter LLM-as-a-judge.
//
// For each case we POST a single chat completion to OpenRouter, asking the
// judge model to score the assistant's response against the question and
// reference answer. The judge returns JSON with numeric metric scores in
// [0.0, 1.0]; network or parse errors become `None` values so the report
// still renders.

use serde::{Deserialize, Serialize};

use crate::evals::case_loader::{EvalCase, MetricKind};
use crate::evals::harness::CaseOutcome;

const DEFAULT_JUDGE_MODEL: &str = "anthropic/claude-sonnet-4.5";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
/// How long we give the judge per case before giving up.
const JUDGE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Default)]
pub struct JudgeScores {
    pub answer_relevancy: Option<f64>,
    pub faithfulness: Option<f64>,
    pub contextual_relevancy: Option<f64>,
    pub contextual_recall: Option<f64>,
    pub rationale: Option<String>,
    pub error: Option<String>,
}

pub struct Judge {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl Judge {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(JUDGE_TIMEOUT_SECS))
            .build()
            .expect("reqwest client");
        let model = model.unwrap_or_else(|| DEFAULT_JUDGE_MODEL.to_string());
        Self { client, api_key, model }
    }

    /// Score one case. Never returns `Err` — network / parse failures produce
    /// a `JudgeScores` with `error` set so the report can surface them.
    pub async fn score(&self, case: &EvalCase, outcome: &CaseOutcome) -> JudgeScores {
        if case.metrics.is_empty() {
            return JudgeScores::default();
        }

        let prompt = build_prompt(case, outcome);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": JUDGE_SYSTEM },
                { "role": "user", "content": prompt }
            ],
            "temperature": 0.0,
            "response_format": { "type": "json_object" }
        });

        let response = self
            .client
            .post(OPENROUTER_ENDPOINT)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://emailops.local/eval")
            .header("X-Title", "EmailOps Chat Eval")
            .json(&body)
            .send()
            .await;

        let resp = match response {
            Ok(r) => r,
            Err(e) => {
                return JudgeScores {
                    error: Some(format!("judge HTTP error: {}", e)),
                    ..Default::default()
                }
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return JudgeScores {
                    error: Some(format!("judge body read failed: {}", e)),
                    ..Default::default()
                }
            }
        };

        if !status.is_success() {
            return JudgeScores {
                error: Some(format!("judge HTTP {}: {}", status, truncate(&text, 400))),
                ..Default::default()
            };
        }

        parse_judge_response(&text, case)
    }
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    choices: Vec<OpenRouterChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    message: OpenRouterMessage,
}

#[derive(Debug, Deserialize)]
struct OpenRouterMessage {
    content: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
struct JudgePayload {
    answer_relevancy: Option<f64>,
    faithfulness: Option<f64>,
    contextual_relevancy: Option<f64>,
    contextual_recall: Option<f64>,
    rationale: Option<String>,
}

fn parse_judge_response(raw: &str, case: &EvalCase) -> JudgeScores {
    // OpenRouter wraps the judge's content in a chat completion envelope.
    let envelope: OpenRouterResponse = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            return JudgeScores {
                error: Some(format!("malformed judge envelope: {} — raw: {}", e, truncate(raw, 400))),
                ..Default::default()
            }
        }
    };

    let content = match envelope.choices.first() {
        Some(c) => &c.message.content,
        None => {
            return JudgeScores {
                error: Some("judge response had no choices".into()),
                ..Default::default()
            }
        }
    };

    // The content is JSON — but models sometimes wrap it in ```json fences.
    let stripped = strip_code_fence(content);
    let payload: JudgePayload = match serde_json::from_str(&stripped) {
        Ok(p) => p,
        Err(e) => {
            return JudgeScores {
                error: Some(format!(
                    "malformed judge JSON: {} — content: {}",
                    e,
                    truncate(content, 400)
                )),
                ..Default::default()
            }
        }
    };

    // Only report metrics that were requested for this case.
    let wanted = |m: MetricKind| case.metrics.contains(&m);
    JudgeScores {
        answer_relevancy: if wanted(MetricKind::AnswerRelevancy) {
            payload.answer_relevancy
        } else {
            None
        },
        faithfulness: if wanted(MetricKind::Faithfulness) {
            payload.faithfulness
        } else {
            None
        },
        contextual_relevancy: if wanted(MetricKind::ContextualRelevancy) {
            payload.contextual_relevancy
        } else {
            None
        },
        contextual_recall: if wanted(MetricKind::ContextualRecall) {
            payload.contextual_recall
        } else {
            None
        },
        rationale: payload.rationale,
        error: None,
    }
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        rest.trim_start_matches('\n').trim_end_matches("```").to_string()
    } else if let Some(rest) = t.strip_prefix("```") {
        rest.trim_start_matches('\n').trim_end_matches("```").to_string()
    } else {
        t.to_string()
    }
}

const JUDGE_SYSTEM: &str = "You are an evaluator for a retrieval-augmented chat assistant that \
answers questions about a user's own emails. You will score the assistant's response on \
several numeric metrics in [0.0, 1.0] and return them as strict JSON (no prose outside JSON). \
Be conservative — only award high scores when the claim is clearly justified by the sources.";

fn build_prompt(case: &EvalCase, outcome: &CaseOutcome) -> String {
    let expected = case
        .expected_output
        .clone()
        .unwrap_or_else(|| "(no golden reference provided)".into());

    let sources = if outcome.sources_used.is_empty() {
        "(no pre-retrieved RAG sources — the assistant was routed tools-first)".to_string()
    } else {
        let mut s = String::new();
        for src in &outcome.sources_used {
            s.push_str(&format!(
                "- [{}] {} — {} <{}>\n",
                src.citation_number, src.subject, src.sender, src.sender_email
            ));
        }
        s
    };

    // When the assistant went through tool calls, include the actual tool I/O
    // so the judge can score faithfulness against those results. Without this,
    // tools-first cases were auto-scoring faithfulness = 0 because the judge
    // saw no grounding at all.
    let tool_calls_section = outcome
        .assistant_trace
        .as_ref()
        .map(|t| &t.tool_calls)
        .filter(|tc| !tc.is_empty())
        .map(|tc| {
            let mut s = String::from(
                "\nTOOL CALLS MADE BY THE ASSISTANT (these are its grounding context when no RAG sources were used):\n",
            );
            for call in tc {
                s.push_str(&format!(
                    "- {}({}) → {} chars\n{}\n",
                    call.name,
                    serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
                    call.result_chars,
                    indent_lines(&call.result_preview, "    "),
                ));
            }
            s
        })
        .unwrap_or_default();

    let metrics: Vec<&str> = case.metrics.iter().map(|m| m.as_str()).collect();

    format!(
        "QUESTION:\n{question}\n\n\
GOLDEN REFERENCE ANSWER:\n{expected}\n\n\
SOURCES SHOWN TO THE ASSISTANT:\n{sources}{tool_calls}\n\
ASSISTANT RESPONSE:\n{response}\n\n\
Score the assistant response on the following metrics only: {metrics}.\n\
For faithfulness / contextual_* metrics, treat BOTH the SOURCES block and the TOOL CALLS block \
(if present) as valid grounding context — the assistant is allowed to ground claims on either.\n\
Each score is a float in [0.0, 1.0]. If you cannot score a metric, return null for it.\n\
Return strict JSON with this shape:\n\
{{\n\
  \"answer_relevancy\": 0.0,\n\
  \"faithfulness\": 0.0,\n\
  \"contextual_relevancy\": 0.0,\n\
  \"contextual_recall\": 0.0,\n\
  \"rationale\": \"one or two sentences\"\n\
}}\n\
Only include keys for the metrics requested; set others to null.",
        question = case.question,
        expected = expected,
        sources = sources,
        tool_calls = tool_calls_section,
        response = outcome.assistant_content,
        metrics = metrics.join(", "),
    )
}

fn indent_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
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
