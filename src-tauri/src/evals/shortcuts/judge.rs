// Split-rubric LLM judge for shortcut variants.
//
// Unlike the chat eval which scores RAGAS-style metrics (answer_relevancy,
// faithfulness, contextual_*), this judge asks for FOUR shortcut-specific
// scores in [0,1]:
//   - structure    — follows the requested format (table, columns, summary)?
//   - faithfulness — are the table rows actually supported by the grounding
//                    context (tool calls or RAG sources)?
//   - usefulness   — does it actually help a busy founder / freelancer triage
//                    their inbox? (important stuff up top, actions clear, no
//                    filler)
//   - tone         — concise and natively in the case's expected language?
//                    (penalize robotic phrasing OR drift to a different
//                    language than the rubric specified.)
// Plus a short rationale.

use serde::{Deserialize, Serialize};

use crate::evals::harness::CaseOutcome;
use crate::evals::shortcuts::case_loader::{ShortcutCase, ShortcutVariant};

const DEFAULT_JUDGE_MODEL: &str = "anthropic/claude-sonnet-4.5";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
const JUDGE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Default, Serialize)]
pub struct VariantScores {
    pub structure: Option<f64>,
    pub faithfulness: Option<f64>,
    pub usefulness: Option<f64>,
    pub tone: Option<f64>,
    pub rationale: Option<String>,
    pub error: Option<String>,
}

impl VariantScores {
    /// Simple mean across the four metrics, ignoring missing ones.
    pub fn composite(&self) -> Option<f64> {
        let vals: Vec<f64> = [self.structure, self.faithfulness, self.usefulness, self.tone]
            .into_iter()
            .flatten()
            .collect();
        if vals.is_empty() {
            None
        } else {
            Some(vals.iter().sum::<f64>() / vals.len() as f64)
        }
    }
}

pub struct ShortcutJudge {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl ShortcutJudge {
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

    pub async fn score(&self, case: &ShortcutCase, variant: &ShortcutVariant, outcome: &CaseOutcome) -> VariantScores {
        let prompt = build_prompt(case, variant, outcome);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
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
            .header("X-Title", "EmailOps Shortcut Variant Eval")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return VariantScores {
                    error: Some(format!("judge HTTP error: {}", e)),
                    ..Default::default()
                }
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return VariantScores {
                    error: Some(format!("judge body read failed: {}", e)),
                    ..Default::default()
                }
            }
        };
        if !status.is_success() {
            return VariantScores {
                error: Some(format!("judge HTTP {}: {}", status, truncate(&text, 400))),
                ..Default::default()
            };
        }
        parse_response(&text)
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

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct JudgePayload {
    structure: Option<f64>,
    faithfulness: Option<f64>,
    usefulness: Option<f64>,
    tone: Option<f64>,
    rationale: Option<String>,
}

fn parse_response(raw: &str) -> VariantScores {
    let envelope: OpenRouterResponse = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            return VariantScores {
                error: Some(format!("malformed envelope: {} — raw: {}", e, truncate(raw, 400))),
                ..Default::default()
            }
        }
    };
    let content = match envelope.choices.first() {
        Some(c) => &c.message.content,
        None => {
            return VariantScores {
                error: Some("no choices in judge response".into()),
                ..Default::default()
            }
        }
    };
    let stripped = strip_code_fence(content);
    let p: JudgePayload = match serde_json::from_str(&stripped) {
        Ok(p) => p,
        Err(e) => {
            return VariantScores {
                error: Some(format!("malformed JSON: {} — content: {}", e, truncate(content, 400))),
                ..Default::default()
            }
        }
    };
    VariantScores {
        structure: p.structure,
        faithfulness: p.faithfulness,
        usefulness: p.usefulness,
        tone: p.tone,
        rationale: p.rationale,
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

const SYSTEM_PROMPT: &str = "You are a precise evaluator scoring variants of a shortcut prompt \
for an AI email assistant. Each variant must satisfy a structural rubric AND read well to a busy \
founder/freelancer. You will score FOUR dimensions in [0.0, 1.0] and return strict JSON with no \
prose outside the JSON. Be conservative — only award high scores when clearly deserved.";

fn build_prompt(case: &ShortcutCase, variant: &ShortcutVariant, outcome: &CaseOutcome) -> String {
    let sources = if outcome.sources_used.is_empty() {
        "(no pre-retrieved RAG sources — answer was grounded on tool-call results)".to_string()
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

    let tool_calls = outcome
        .assistant_trace
        .as_ref()
        .map(|t| &t.tool_calls)
        .filter(|tc| !tc.is_empty())
        .map(|tc| {
            let mut s = String::from("\nTOOL CALLS (grounding when there are no pre-retrieved sources):\n");
            for call in tc {
                s.push_str(&format!(
                    "- {}({}) → {} chars\n{}\n",
                    call.name,
                    serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
                    call.result_chars,
                    indent(&call.result_preview, "    "),
                ));
            }
            s
        })
        .unwrap_or_default();

    let required_cols = if case.rubric.required_columns.is_empty() {
        "(none)".to_string()
    } else {
        case.rubric.required_columns.join(" | ")
    };

    format!(
        "SHORTCUT: {shortcut_id} — {label}\n\
VARIANT: {variant_id} ({variant_desc})\n\n\
STRUCTURAL RUBRIC (must be honored by the answer):\n\
- language: {lang}\n\
- must_contain_table: {must_table}\n\
- required columns: {req_cols}\n\
- min_rows: {min_rows}\n\
- must_end_with_summary_paragraph: {summary}\n\
- require_row_citations: {row_cites}\n\n\
USER PROMPT SENT TO THE MODEL:\n{prompt}\n\n\
RAG SOURCES:\n{sources}{tool_calls}\n\
ASSISTANT RESPONSE:\n{response}\n\n\
Score these four metrics as floats in [0.0, 1.0]:\n\
- structure: does the response follow the structural rubric above (table, columns, rows, summary, citations)?\n\
- faithfulness: are the table rows and summary claims actually supported by the sources / tool results?\n\
- usefulness: does it help a busy founder triage their inbox? (important first, actions clear, no filler)\n\
- tone: is it concise, and natively in {lang}? Penalize robotic phrasing and language drift.\n\n\
Return strict JSON:\n\
{{\n\
  \"structure\": 0.0,\n\
  \"faithfulness\": 0.0,\n\
  \"usefulness\": 0.0,\n\
  \"tone\": 0.0,\n\
  \"rationale\": \"two or three sentences explaining the scores\"\n\
}}",
        shortcut_id = case.shortcut_id,
        label = case.label,
        variant_id = variant.id,
        variant_desc = if variant.description.is_empty() {
            "(no description)"
        } else {
            &variant.description
        },
        lang = case.rubric.language,
        must_table = case.rubric.must_contain_table,
        req_cols = required_cols,
        min_rows = case.rubric.min_rows,
        summary = case.rubric.must_end_with_summary_paragraph,
        row_cites = case.rubric.require_row_citations,
        prompt = variant.prompt,
        sources = sources,
        tool_calls = tool_calls,
        response = outcome.assistant_content,
    )
}

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{}{}", prefix, l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(s: &str, n: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= n {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}
