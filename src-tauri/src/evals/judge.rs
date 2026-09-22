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

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
    parse_judge_content(content, case)
}

/// Parse the judge's reply body (the JSON object it was asked for, possibly
/// fenced) into scores, keeping only the metrics the case requested.
pub fn parse_judge_content(content: &str, case: &EvalCase) -> JudgeScores {
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

/// Score a case with a model served by the app itself (embedded llama.cpp,
/// Ollama…) instead of OpenRouter. Same system prompt, same rubric, same
/// parsing; the provider's `complete` is asked for JSON with temperature 0.
pub async fn score_with_provider(
    provider: &dyn crate::ai::provider::AIProvider,
    case: &EvalCase,
    outcome: &CaseOutcome,
) -> JudgeScores {
    if case.metrics.is_empty() {
        return JudgeScores::default();
    }
    let prompt = format!(
        "{JUDGE_SYSTEM}\n\n{}\n\nReply with the JSON object only.",
        build_prompt(case, outcome)
    );
    let options = crate::ai::provider::CompletionOptions {
        temperature: Some(0.0),
        max_tokens: Some(600),
        think: Some(false),
    };
    match provider.complete(&prompt, options).await {
        Ok(result) => parse_judge_content(&result.text, case),
        Err(e) => JudgeScores {
            error: Some(format!("judge provider error: {e}")),
            ..Default::default()
        },
    }
}

/// Score every requested judge metric must reach for the case to pass.
pub const JUDGE_THRESHOLD: f64 = 0.7;

/// Whether a case passes: its heuristic checks, and — when the judge ran —
/// every judge metric it asked for at [`JUDGE_THRESHOLD`]. A judge error on a
/// judged run is a failure: an unscored case must not read as a pass.
pub fn case_passes(heuristics_passed: bool, scores: &JudgeScores, case: &EvalCase, judge_enabled: bool) -> bool {
    heuristics_passed && (!judge_enabled || judge_passes(scores, case, JUDGE_THRESHOLD))
}

/// A case passes the judge when every metric it asked for scored at least
/// `threshold`. A judge error is a failure, never a silent pass. When the case
/// requested no metrics there is nothing to judge and this returns `true`.
pub fn judge_passes(scores: &JudgeScores, case: &EvalCase, threshold: f64) -> bool {
    if case.metrics.is_empty() {
        return true;
    }
    if scores.error.is_some() {
        return false;
    }
    let wanted = |m: MetricKind, v: Option<f64>| !case.metrics.contains(&m) || v.is_some_and(|x| x >= threshold);
    wanted(MetricKind::AnswerRelevancy, scores.answer_relevancy)
        && wanted(MetricKind::Faithfulness, scores.faithfulness)
        && wanted(MetricKind::ContextualRelevancy, scores.contextual_relevancy)
        && wanted(MetricKind::ContextualRecall, scores.contextual_recall)
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

/// Chars of each source body shown to the judge — enough to check the
/// answer's facts, bounded so 9 sources cannot blow the judge's context.
const JUDGE_SOURCE_BODY_CHARS: usize = 1500;

const JUDGE_SYSTEM: &str = "You are an evaluator for a retrieval-augmented chat assistant that \
answers questions about a user's own emails, and about the EmailOps app itself from its bundled \
user guides. You will score the assistant's response on \
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
        // The body the model read, not just the envelope: without it every
        // fact the answer took from an email scores as invented.
        let mut s = String::new();
        for src in &outcome.sources_used {
            s.push_str(&format!(
                "- [{}] {} — {} <{}>\n{}\n",
                src.citation_number,
                src.subject,
                src.sender,
                src.sender_email,
                indent_lines(&truncate(&src.body_snippet, JUDGE_SOURCE_BODY_CHARS), "    "),
            ));
        }
        s
    };

    // An answer about the app is grounded in the bundled guides, not in mail.
    let guide_section = if outcome.help_sections.is_empty() {
        String::new()
    } else {
        let mut s = String::from("\nEMAILOPS GUIDE SECTIONS SHOWN TO THE ASSISTANT (the app's own user guides):\n");
        for section in &outcome.help_sections {
            s.push_str(&format!("{}\n", indent_lines(section, "    ")));
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

    // An open email is the third kind of grounding: the turn answered from
    // that thread, with no RAG sources and no tools, so without it the judge
    // reads every detail of the answer as invented.
    let open_thread_section = outcome
        .open_thread
        .as_deref()
        .map(|t| {
            format!(
                "\nOPEN THREAD SHOWN TO THE ASSISTANT (the email the user had open):\n{}\n",
                indent_lines(t, "    ")
            )
        })
        .unwrap_or_default();

    // The memory header rides in the user message: an answer taken from it
    // has no RAG source and no tool call, so without it the judge reads the
    // remembered fact as invented.
    let memory_section = outcome
        .memory
        .as_deref()
        .map(|m| {
            format!(
                "\nMEMORY SHOWN TO THE ASSISTANT (facts it remembers about the user):\n{}\n",
                indent_lines(m, "    ")
            )
        })
        .unwrap_or_default();

    let metrics: Vec<&str> = case.metrics.iter().map(|m| m.as_str()).collect();

    format!(
        "QUESTION:\n{question}\n\n\
GOLDEN REFERENCE ANSWER:\n{expected}\n\n\
SOURCES SHOWN TO THE ASSISTANT:\n{sources}{tool_calls}{open_thread}{memory}{guides}\n\
ASSISTANT RESPONSE:\n{response}\n\n\
Score the assistant response on the following metrics only: {metrics}.\n\
For faithfulness / contextual_* metrics, treat the SOURCES, TOOL CALLS, OPEN THREAD, MEMORY and EMAILOPS GUIDE SECTIONS blocks \
(whichever are present) as valid grounding context — the assistant is allowed to ground claims on any of them.\n\
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
        open_thread = open_thread_section,
        memory = memory_section,
        guides = guide_section,
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

#[cfg(test)]
mod judge_rule_tests {
    use super::{build_prompt, case_passes, judge_passes, parse_judge_content, JudgeScores};
    use crate::evals::case_loader::{EvalCase, MetricKind};
    use crate::evals::harness::CaseOutcome;

    fn case_with(metrics: Vec<MetricKind>) -> EvalCase {
        let mut c: EvalCase =
            serde_yaml::from_str("id: t\nquestion: q\ncategory: c\ntier: smoke\n").expect("minimal case");
        c.metrics = metrics;
        c
    }

    fn outcome_with(open_thread: Option<&str>) -> CaseOutcome {
        CaseOutcome {
            conversation_id: String::new(),
            conversation_title: String::new(),
            assistant_message_id: String::new(),
            assistant_content: "answer".into(),
            assistant_trace: None,
            assistant_token_count: None,
            assistant_latency_ms: None,
            wall_elapsed_ms: 0,
            sources_used: Vec::new(),
            open_thread: open_thread.map(str::to_string),
            help_sections: Vec::new(),
            memory: None,
        }
    }

    // The judge scored faithfulness against source subjects and senders only,
    // so any answer that used what an email SAID read as invented, and an
    // answer from the bundled guides had no grounding at all.

    // A case whose answer the judge scores low is a failing case, not a
    // passing one with a footnote: "resume este correo" answered with
    // invented setup steps passed its word checks at 0.30 / 0.10.

    #[test]
    fn a_low_judge_score_fails_the_case() {
        let case = case_with(vec![MetricKind::AnswerRelevancy, MetricKind::Faithfulness]);
        let low = JudgeScores {
            answer_relevancy: Some(0.30),
            faithfulness: Some(0.10),
            ..Default::default()
        };
        assert!(!case_passes(true, &low, &case, true));
    }

    #[test]
    fn a_judge_error_fails_the_case_only_when_the_judge_ran() {
        let case = case_with(vec![MetricKind::Faithfulness]);
        let err = JudgeScores {
            error: Some("judge HTTP 401".into()),
            ..Default::default()
        };
        assert!(!case_passes(true, &err, &case, true), "an unscored case is not a pass");
        assert!(
            case_passes(true, &JudgeScores::default(), &case, false),
            "no judge: heuristics decide"
        );
    }

    #[test]
    fn heuristics_still_gate_a_well_judged_answer() {
        let case = case_with(vec![MetricKind::Faithfulness]);
        let good = JudgeScores {
            faithfulness: Some(0.95),
            ..Default::default()
        };
        assert!(case_passes(true, &good, &case, true));
        assert!(!case_passes(false, &good, &case, true));
    }

    #[test]
    fn prompt_shows_what_each_source_said() {
        let case = case_with(vec![MetricKind::Faithfulness]);
        let mut outcome = outcome_with(None);
        outcome.sources_used = vec![crate::evals::harness::SourceSummary {
            citation_number: 1,
            email_id: "e1".into(),
            subject: "Invoice May".into(),
            sender: "Billing".into(),
            sender_email: "billing@example.com".into(),
            relevance_score: None,
            body_snippet: "Servers: CPX31 x2".into(),
        }];
        let prompt = build_prompt(&case, &outcome);
        assert!(prompt.contains("Invoice May"));
        assert!(prompt.contains("Servers: CPX31 x2"), "{prompt}");
    }

    #[test]
    fn prompt_shows_the_guide_sections_as_grounding() {
        let case = case_with(vec![MetricKind::Faithfulness]);
        let mut outcome = outcome_with(None);
        outcome.help_sections = vec!["AI features › Choosing a backend\nOllama runs at localhost:11434.".into()];
        let prompt = build_prompt(&case, &outcome);
        assert!(
            prompt.contains("EMAILOPS GUIDE SECTIONS SHOWN TO THE ASSISTANT"),
            "{prompt}"
        );
        assert!(prompt.contains("localhost:11434"));
        let without = build_prompt(&case, &outcome_with(None));
        assert!(!without.contains("EMAILOPS GUIDE SECTIONS SHOWN TO THE ASSISTANT"));
    }

    /// A turn run with an email open answers from that thread, not from RAG
    /// sources or tools; the judge must see it or it scores faithfulness 0.
    #[test]
    fn prompt_shows_the_open_thread_as_grounding_when_the_case_has_one() {
        let case = case_with(vec![MetricKind::Faithfulness]);
        let with = build_prompt(&case, &outcome_with(Some("From: Nadia\nHow do I add an account?")));
        assert!(with.contains("OPEN THREAD SHOWN TO THE ASSISTANT"));
        assert!(with.contains("How do I add an account?"));
        let without = build_prompt(&case, &outcome_with(None));
        assert!(!without.contains("OPEN THREAD SHOWN TO THE ASSISTANT"));
    }

    /// A turn that answered from the `<memory>` header (no RAG source, no
    /// tool) scored faithfulness 0 every run: the judge never saw the header.
    #[test]
    fn prompt_shows_the_memory_header_as_grounding_when_the_turn_had_one() {
        let case = case_with(vec![MetricKind::Faithfulness]);
        let mut outcome = outcome_with(None);
        outcome.memory = Some(
            "<memory>\nEntities matching query:\n  - domain [acme.test]: customer number AC-1234\n</memory>".into(),
        );
        let with = build_prompt(&case, &outcome);
        assert!(with.contains("MEMORY SHOWN TO THE ASSISTANT"), "{with}");
        assert!(with.contains("AC-1234"));
        let without = build_prompt(&case, &outcome_with(None));
        assert!(!without.contains("MEMORY SHOWN TO THE ASSISTANT"));
    }

    #[test]
    fn parses_a_fenced_json_reply_and_keeps_only_requested_metrics() {
        let case = case_with(vec![MetricKind::AnswerRelevancy]);
        let s = parse_judge_content(
            "```json\n{\"answer_relevancy\": 0.9, \"faithfulness\": 0.2, \"rationale\": \"ok\"}\n```",
            &case,
        );
        assert_eq!(s.answer_relevancy, Some(0.9));
        assert_eq!(s.faithfulness, None, "unrequested metrics are dropped");
        assert_eq!(s.rationale.as_deref(), Some("ok"));
        assert!(s.error.is_none());
    }

    #[test]
    fn a_case_passes_only_when_every_requested_metric_reaches_the_threshold() {
        let case = case_with(vec![MetricKind::AnswerRelevancy, MetricKind::Faithfulness]);
        let good = JudgeScores {
            answer_relevancy: Some(0.8),
            faithfulness: Some(0.7),
            ..Default::default()
        };
        let low = JudgeScores {
            answer_relevancy: Some(0.8),
            faithfulness: Some(0.4),
            ..Default::default()
        };
        let missing = JudgeScores {
            answer_relevancy: Some(0.9),
            ..Default::default()
        };
        assert!(judge_passes(&good, &case, 0.7));
        assert!(!judge_passes(&low, &case, 0.7));
        assert!(
            !judge_passes(&missing, &case, 0.7),
            "a metric the judge did not return is a failure"
        );
    }

    #[test]
    fn a_judge_error_never_passes_and_no_metrics_means_nothing_to_judge() {
        let err = JudgeScores {
            error: Some("timeout".into()),
            ..Default::default()
        };
        assert!(!judge_passes(&err, &case_with(vec![MetricKind::AnswerRelevancy]), 0.7));
        assert!(judge_passes(&err, &case_with(vec![]), 0.7));
    }
}
