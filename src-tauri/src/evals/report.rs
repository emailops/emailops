// HTML report rendering (dark theme, inline CSS, self-contained).
//
// We keep the Tera template as an inline string so the report can be produced
// from a single binary without chasing template files at runtime.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::case_loader::EvalCase;
use crate::evals::harness::CaseOutcome;
use crate::evals::judge::JudgeScores;
use crate::evals::metrics::HeuristicReport;
use crate::evals::EvalResult;
use crate::models::ChatTrace;

/// Inputs for a single case card in the report.
pub struct ReportCase<'a> {
    pub case: &'a EvalCase,
    pub outcome: &'a CaseOutcome,
    pub heuristics: &'a HeuristicReport,
    pub judge: &'a JudgeScores,
}

#[derive(Serialize)]
struct CaseView {
    id: String,
    question: String,
    category: String,
    tier: String,
    overall_pass: bool,
    title: String,
    answer: String,
    latency_ms: i64,
    wall_elapsed_ms: i64,
    token_count: Option<i32>,
    /// Execution path of the turn on one line — see [`flow_summary`].
    flow: Option<String>,
    /// The email the turn ran against (bound or open thread), as the model saw it.
    open_thread: Option<String>,
    /// Every step of the turn with what it read or produced — see [`step_views`].
    steps: Vec<StepView>,
    heuristics: Vec<CheckView>,
    metric_rows: Vec<MetricRowView>,
    judge_error: Option<String>,
    judge_rationale: Option<String>,
}

#[derive(Serialize)]
struct CheckView {
    name: String,
    passed: bool,
    expected: String,
    actual: String,
    detail: String,
}

#[derive(Serialize)]
struct MetricRowView {
    name: String,
    score_pct: Option<i32>,
    score_label: String,
    score_class: String,
}

#[derive(Serialize)]
struct SummaryView {
    generated_at: String,
    total: usize,
    passed: usize,
    failed: usize,
    chat_model: String,
    judge_enabled: bool,
    judge_model: String,
    avg_answer_relevancy: Option<i32>,
    avg_faithfulness: Option<i32>,
}

/// Render the report and write it to `{out_dir}/eval_report_{stamp}.html`.
pub fn render(
    out_dir: &Path,
    cases: &[ReportCase<'_>],
    chat_model: &str,
    judge_enabled: bool,
    judge_model: &str,
) -> EvalResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let path = out_dir.join(format!("eval_report_{}.html", stamp));

    let mut tera = Tera::default();
    tera.add_raw_template("report", REPORT_TEMPLATE)?;

    // Aggregate.
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut ar_vals: Vec<f64> = Vec::new();
    let mut ff_vals: Vec<f64> = Vec::new();

    let mut case_views: Vec<CaseView> = Vec::with_capacity(cases.len());
    for rc in cases {
        total += 1;
        let overall_pass =
            crate::evals::judge::case_passes(rc.heuristics.all_passed(), rc.judge, rc.case, judge_enabled);
        if overall_pass {
            passed += 1;
        }

        if let Some(v) = rc.judge.answer_relevancy {
            ar_vals.push(v);
        }
        if let Some(v) = rc.judge.faithfulness {
            ff_vals.push(v);
        }

        case_views.push(build_case_view(rc, overall_pass));
    }

    let avg_ar = avg_pct(&ar_vals);
    let avg_ff = avg_pct(&ff_vals);

    let summary = SummaryView {
        generated_at: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        total,
        passed,
        failed: total.saturating_sub(passed),
        chat_model: chat_model.to_string(),
        judge_enabled,
        judge_model: judge_model.to_string(),
        avg_answer_relevancy: avg_ar,
        avg_faithfulness: avg_ff,
    };

    let mut ctx = Context::new();
    ctx.insert("summary", &summary);
    ctx.insert("cases", &case_views);

    let html = tera.render("report", &ctx)?;
    std::fs::write(&path, html)?;
    Ok(path)
}

fn avg_pct(vals: &[f64]) -> Option<i32> {
    if vals.is_empty() {
        return None;
    }
    let avg = vals.iter().sum::<f64>() / vals.len() as f64;
    Some((avg * 100.0).round() as i32)
}

fn build_case_view(rc: &ReportCase<'_>, overall_pass: bool) -> CaseView {
    let trace = rc.outcome.assistant_trace.as_ref();
    let flow = trace.map(flow_summary);
    let steps = trace.map(|t| step_views(t, rc.outcome)).unwrap_or_default();

    let heuristics = rc
        .heuristics
        .checks
        .iter()
        .map(|c| CheckView {
            name: c.name.clone(),
            passed: c.passed,
            expected: c.expected.clone(),
            actual: c.actual.clone(),
            detail: c.detail.clone(),
        })
        .collect();

    let mut metric_rows = Vec::new();
    push_metric(&mut metric_rows, "answer_relevancy", rc.judge.answer_relevancy);
    push_metric(&mut metric_rows, "faithfulness", rc.judge.faithfulness);
    push_metric(&mut metric_rows, "contextual_relevancy", rc.judge.contextual_relevancy);
    push_metric(&mut metric_rows, "contextual_recall", rc.judge.contextual_recall);

    CaseView {
        id: rc.case.id.clone(),
        question: rc.case.question.clone(),
        category: rc.case.category.clone(),
        tier: rc.case.tier.clone(),
        overall_pass,
        title: rc.outcome.conversation_title.clone(),
        answer: rc.outcome.assistant_content.clone(),
        latency_ms: rc.outcome.assistant_latency_ms.unwrap_or(0),
        wall_elapsed_ms: rc.outcome.wall_elapsed_ms,
        token_count: rc.outcome.assistant_token_count,
        flow,
        open_thread: rc.outcome.open_thread.clone(),
        steps,
        heuristics,
        metric_rows,
        judge_error: rc.judge.error.clone(),
        judge_rationale: rc.judge.rationale.clone(),
    }
}

fn push_metric(rows: &mut Vec<MetricRowView>, name: &str, score: Option<f64>) {
    match score {
        None => {}
        Some(v) => {
            let pct = (v * 100.0).round() as i32;
            let class = if pct >= 80 {
                "pass"
            } else if pct >= 60 {
                "mixed"
            } else {
                "fail"
            };
            rows.push(MetricRowView {
                name: name.to_string(),
                score_pct: Some(pct),
                score_label: format!("{:.2}", v),
                score_class: class.to_string(),
            });
        }
    }
}

// The report template. Inline CSS + a small JS snippet for case expand/collapse.
const REPORT_TEMPLATE: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>EmailOps Chat Eval — {{ summary.generated_at }}</title>
<style>
  :root {
    --green: #22c55e; --red: #ef4444; --amber: #f59e0b;
    --bg: #0f172a; --surface: #1e293b; --surface2: #334155;
    --text: #f1f5f9; --text-muted: #94a3b8; --border: #475569;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: var(--bg); color: var(--text); line-height: 1.55;
    padding: 2rem; max-width: 1280px; margin: 0 auto;
  }
  h1 { font-size: 1.75rem; margin-bottom: 0.25rem; }
  h2 { font-size: 1.1rem; margin-bottom: 0.75rem; color: var(--text); }
  h3 { font-size: 0.95rem; margin-bottom: 0.4rem; color: var(--text); }
  .subtitle { color: var(--text-muted); font-size: 0.875rem; margin-bottom: 2rem; }

  .summary { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 1rem; margin-bottom: 2rem; }
  .card { background: var(--surface); border-radius: 12px; padding: 1.1rem 1.25rem; border: 1px solid var(--border); }
  .card-label { font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); margin-bottom: 0.2rem; }
  .card-value { font-size: 1.8rem; font-weight: 700; }
  .card-value.pass { color: var(--green); }
  .card-value.fail { color: var(--red); }
  .card-value.mixed { color: var(--amber); }
  .card-hint { color: var(--text-muted); font-size: 0.75rem; margin-top: 0.15rem; }

  .tc-card { background: var(--surface); border-radius: 12px; padding: 1.25rem 1.5rem; border: 1px solid var(--border); margin-bottom: 1rem; }
  .tc-header { display: flex; justify-content: space-between; align-items: center; cursor: pointer; gap: 1rem; }
  .tc-title { font-weight: 600; flex: 1; }
  .tc-meta { color: var(--text-muted); font-size: 0.8rem; margin-top: 0.15rem; }
  /* The path the turn took, so "planner or model choice?" is answerable
     without opening the card and cross-reading three blocks. */
  .tc-flow { font-family: ui-monospace, 'SF Mono', Menlo, monospace; font-size: 0.78rem; margin-top: 0.3rem; color: #7aa2f7; }
  .badge { display: inline-block; padding: 0.2rem 0.7rem; border-radius: 9999px; font-size: 0.7rem; font-weight: 600; text-transform: uppercase; }
  .badge-pass { background: rgba(34,197,94,0.15); color: var(--green); }
  .badge-fail { background: rgba(239,68,68,0.15); color: var(--red); }
  .badge-mixed { background: rgba(245,158,11,0.15); color: var(--amber); }

  .tc-body { display: none; margin-top: 1rem; }
  .tc-card.open .tc-body { display: block; }
  .tc-chevron { color: var(--text-muted); transition: transform 0.2s; }
  .tc-card.open .tc-chevron { transform: rotate(180deg); }

  .section { margin-bottom: 1.1rem; }
  .section-label { font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); margin-bottom: 0.35rem; }
  .content-box { background: var(--surface2); border-radius: 8px; padding: 0.85rem 1rem; font-size: 0.85rem; white-space: pre-wrap; word-break: break-word; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.8rem; }

  .checks { display: grid; grid-template-columns: 1fr; gap: 0.5rem; }
  .check { background: var(--surface2); border-radius: 8px; padding: 0.6rem 0.85rem; border-left: 3px solid var(--border); font-size: 0.85rem; }
  .check.pass { border-left-color: var(--green); }
  .check.fail { border-left-color: var(--red); }
  .check-head { display: flex; justify-content: space-between; font-weight: 600; margin-bottom: 0.15rem; }
  .check-detail { color: var(--text-muted); font-size: 0.78rem; }

  .metrics { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 0.5rem; }
  .metric { background: var(--surface2); border-radius: 8px; padding: 0.6rem 0.85rem; border-left: 3px solid var(--border); }
  .metric.pass { border-left-color: var(--green); }
  .metric.mixed { border-left-color: var(--amber); }
  .metric.fail { border-left-color: var(--red); }
  .metric-head { display: flex; justify-content: space-between; font-size: 0.85rem; font-weight: 600; }
  .metric-score.pass { color: var(--green); }
  .metric-score.mixed { color: var(--amber); }
  .metric-score.fail { color: var(--red); }
  .metric-bar-track { background: var(--bg); border-radius: 4px; height: 6px; margin-top: 0.35rem; overflow: hidden; }
  .metric-bar-fill { height: 100%; border-radius: 4px; }
  .metric-bar-fill.pass { background: var(--green); }
  .metric-bar-fill.mixed { background: var(--amber); }
  .metric-bar-fill.fail { background: var(--red); }

  .sources { list-style: none; }
  .sources li { background: var(--surface2); border-radius: 8px; padding: 0.6rem 0.85rem; margin-bottom: 0.4rem; font-size: 0.82rem; border-left: 3px solid var(--border); }
  .sources li strong { color: var(--text-muted); font-size: 0.72rem; display: block; margin-bottom: 0.1rem; }

  .tool-call { background: var(--surface2); border-radius: 8px; padding: 0.7rem 0.9rem; margin-bottom: 0.4rem; font-size: 0.82rem; border-left: 3px solid var(--border); }
  .tool-head { font-weight: 600; display: flex; justify-content: space-between; }
  .tool-args { color: var(--text-muted); font-size: 0.76rem; margin-top: 0.2rem; white-space: pre-wrap; word-break: break-word; }
  .tool-result { margin-top: 0.4rem; }
  .tool-result > summary { cursor: pointer; font-size: 0.76rem; color: var(--text-muted); user-select: none; }
  .tool-result > summary::marker { color: var(--text-muted); }
  .tool-result > pre { margin: 0.3rem 0 0; padding: 0.5rem 0.6rem; background: #0b1020; color: #d9e2f3; font-size: 0.74rem; border-radius: 6px; max-height: 320px; overflow: auto; white-space: pre-wrap; word-break: break-word; }
  .tool-result[open] > summary { color: var(--text); }

  .retr-stats { display: flex; flex-wrap: wrap; gap: 1rem; color: var(--text-muted); font-size: 0.82rem; }
  .retr-stats span strong { color: var(--text); }
  .warning { color: var(--amber); font-size: 0.8rem; margin-top: 0.3rem; }
</style>
</head>
<body>
<h1>EmailOps Chat Evaluation</h1>
<div class="subtitle">{{ summary.generated_at }} — {{ summary.total }} case(s) · chat model: <strong>{{ summary.chat_model }}</strong> · judge: {{ summary.judge_model }}{% if not summary.judge_enabled %} (skipped){% endif %}</div>

<div class="summary">
  <div class="card">
    <div class="card-label">Cases</div>
    <div class="card-value">{{ summary.total }}</div>
  </div>
  <div class="card">
    <div class="card-label">Passed</div>
    <div class="card-value pass">{{ summary.passed }}</div>
  </div>
  <div class="card">
    <div class="card-label">Failed</div>
    <div class="card-value {% if summary.failed > 0 %}fail{% endif %}">{{ summary.failed }}</div>
  </div>
  {% if summary.avg_answer_relevancy %}
  <div class="card">
    <div class="card-label">Avg Answer Relevancy</div>
    <div class="card-value">{{ summary.avg_answer_relevancy }}%</div>
  </div>
  {% endif %}
  {% if summary.avg_faithfulness %}
  <div class="card">
    <div class="card-label">Avg Faithfulness</div>
    <div class="card-value">{{ summary.avg_faithfulness }}%</div>
  </div>
  {% endif %}
</div>

{% for c in cases %}
<div class="tc-card">
  <div class="tc-header" onclick="this.parentElement.classList.toggle('open')">
    <div>
      <div class="tc-title">[{{ c.id }}] {{ c.question }}</div>
      <div class="tc-meta">category: {{ c.category }} · tier: {{ c.tier }} · title: "{{ c.title }}" · {{ c.latency_ms }}ms{% if c.token_count %} · {{ c.token_count }} tok{% endif %}</div>
      {% if c.flow %}<div class="tc-flow">{{ c.flow }}</div>{% endif %}
    </div>
    <div>
      {% if c.overall_pass %}<span class="badge badge-pass">pass</span>{% else %}<span class="badge badge-fail">fail</span>{% endif %}
      <span class="tc-chevron">▾</span>
    </div>
  </div>
  <div class="tc-body">

    {# The email in context, then the turn step by step (trace_steps). #}
    {% if c.open_thread %}
    <div class="section">
      <div class="section-label">Open email (context the model had)</div>
      <details open><summary style="cursor:pointer;font-size:0.76rem;color:var(--text-muted);user-select:none;">{{ c.open_thread | length }} chars</summary>
        <pre style="margin:0.3rem 0 0;padding:0.5rem 0.6rem;background:#0b1020;color:#d9e2f3;font-size:0.74rem;border-radius:6px;max-height:320px;overflow:auto;white-space:pre-wrap;word-break:break-word;">{{ c.open_thread }}</pre>
      </details>
    </div>
    {% endif %}

    {% if c.steps | length > 0 %}
    <div class="section">
      <div class="section-label">Trace</div>
      {% for st in c.steps %}
      <div class="tool-call">
        <div class="tool-head"><span>{{ st.label }}</span><span>{{ st.detail }}</span></div>
        {% for b in st.blocks %}
        <details class="tool-result"><summary>{{ b.title }}</summary><pre>{{ b.text }}</pre></details>
        {% endfor %}
      </div>
      {% endfor %}
    </div>
    {% endif %}

    <div class="section">
      <div class="section-label">Heuristic checks</div>
      <div class="checks">
        {% for ck in c.heuristics %}
        <div class="check {% if ck.passed %}pass{% else %}fail{% endif %}">
          <div class="check-head"><span>{{ ck.name }}</span><span>{% if ck.passed %}pass{% else %}fail{% endif %}</span></div>
          <div class="check-detail">expected: {{ ck.expected }} · actual: {{ ck.actual }}</div>
          <div class="check-detail">{{ ck.detail }}</div>
        </div>
        {% endfor %}
      </div>
    </div>

    {% if c.metric_rows | length > 0 %}
    <div class="section">
      <div class="section-label">Judge metrics</div>
      <div class="metrics">
        {% for m in c.metric_rows %}
        <div class="metric {{ m.score_class }}">
          <div class="metric-head"><span>{{ m.name }}</span><span class="metric-score {{ m.score_class }}">{{ m.score_label }}</span></div>
          <div class="metric-bar-track"><div class="metric-bar-fill {{ m.score_class }}" style="width: {{ m.score_pct }}%;"></div></div>
        </div>
        {% endfor %}
      </div>
      {% if c.judge_rationale %}<div class="content-box" style="margin-top: 0.5rem;">{{ c.judge_rationale }}</div>{% endif %}
    </div>
    {% endif %}

    {% if c.judge_error %}
    <div class="section">
      <div class="section-label">Judge error</div>
      <div class="content-box" style="color: var(--amber);">{{ c.judge_error }}</div>
    </div>
    {% endif %}

    <div class="section">
      <div class="section-label">Final answer</div>
      <div class="content-box">{{ c.answer }}</div>
    </div>

  </div>
</div>
{% endfor %}

</body>
</html>
"###;

/// One-line summary of the path a turn took, for the header of each case:
/// `route: planner → planner → RAG retrieval → llm round 0 → answer`. The
/// labels and their order come from `services::chat::trace_steps`, the same
/// list the reasoning panel and `emailops-cli chat --trace` walk.
pub(crate) fn flow_summary(trace: &ChatTrace) -> String {
    trace
        .steps
        .iter()
        .map(|s| crate::services::chat::trace_steps::step_label(trace, s))
        .collect::<Vec<_>>()
        .join(" → ")
}

/// One expandable block under a step: a source, a guide section, a prompt.
#[derive(Serialize, Debug, PartialEq)]
struct StepBlock {
    title: String,
    text: String,
}

/// One row of the report's trace: what ran, the numbers behind it, and what
/// it read or produced.
#[derive(Serialize, Debug)]
struct StepView {
    label: String,
    detail: String,
    blocks: Vec<StepBlock>,
}

/// The turn's steps with their content attached. The trace carries ids and
/// numbers; the case outcome carries the RAG sources and guide sections the
/// model actually read, so both are passed in.
fn step_views(trace: &ChatTrace, outcome: &CaseOutcome) -> Vec<StepView> {
    use crate::models::TraceStep;
    use crate::services::chat::trace_steps::{step_detail, step_label};
    trace
        .steps
        .iter()
        .map(|step| {
            let blocks = match step {
                TraceStep::Retrieval => outcome
                    .sources_used
                    .iter()
                    .map(|s| StepBlock {
                        title: format!(
                            "[{}] {} <{}> — {}",
                            s.citation_number, s.sender, s.sender_email, s.subject
                        ),
                        text: s.body_snippet.clone(),
                    })
                    .collect(),
                TraceStep::Help => outcome
                    .help_sections
                    .iter()
                    .map(|h| StepBlock {
                        title: h.lines().next().unwrap_or_default().to_string(),
                        text: h.clone(),
                    })
                    .collect(),
                TraceStep::Llm { index, .. } => trace
                    .llm_calls
                    .get(*index)
                    .map(|c| {
                        [("input", &c.input), ("output", &c.output)]
                            .into_iter()
                            .filter_map(|(title, text)| {
                                text.as_ref().map(|t| StepBlock {
                                    title: title.to_string(),
                                    text: t.clone(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                TraceStep::Tool { index } => trace
                    .tool_calls
                    .get(*index)
                    .map(|t| {
                        vec![
                            StepBlock {
                                title: "arguments".into(),
                                text: serde_json::to_string_pretty(&t.arguments).unwrap_or_else(|_| "{}".into()),
                            },
                            StepBlock {
                                title: format!("result ({} chars)", t.result_chars),
                                text: t.result_preview.clone(),
                            },
                        ]
                    })
                    .unwrap_or_default(),
                TraceStep::Route => Vec::new(),
            };
            StepView {
                label: step_label(trace, step),
                detail: step_detail(trace, step),
                blocks,
            }
        })
        .collect()
}

#[cfg(test)]
mod step_view_tests {
    use super::*;
    use crate::evals::harness::SourceSummary;

    fn outcome(sources: Vec<SourceSummary>, help_sections: Vec<String>) -> CaseOutcome {
        CaseOutcome {
            conversation_id: String::new(),
            conversation_title: String::new(),
            assistant_message_id: String::new(),
            assistant_content: String::new(),
            assistant_trace: None,
            assistant_token_count: None,
            assistant_latency_ms: None,
            wall_elapsed_ms: 0,
            sources_used: sources,
            open_thread: None,
            help_sections,
            memory: None,
        }
    }

    fn trace() -> ChatTrace {
        let t: ChatTrace = serde_json::from_value(serde_json::json!({
            "route": { "mode": "rag_first", "reason": "", "classifier": "planner" },
            "retrieval": { "vectorHits": 20, "ftsHits": 30, "fusedTopK": 9, "elapsedMs": 7, "ftsSearchMs": 1, "fetchMs": 0, "expansionMs": 0 },
            "help": { "lang": "en", "candidates": 24, "included": 1, "vectorAvailable": true, "elapsedMs": 7, "chunkIds": ["en/ai-features#1.0"] },
            "toolCalls": [{ "name": "get_thread", "round": 0, "arguments": {"thread_id": "t1"}, "resultPreview": "6 messages", "resultChars": 10, "elapsedMs": 2 }],
            "model": "m", "totalElapsedMs": 1,
            "llmCalls": [
                { "kind": "planner", "round": -2, "latencyMs": 200 },
                { "kind": "tool_round", "round": 0, "latencyMs": 900, "toolCallsRequested": 1, "input": "prompt", "output": "call get_thread" },
                { "kind": "final_stream", "round": -1, "latencyMs": 3000 }
            ]
        }))
        .expect("trace fixture");
        crate::services::chat::trace_steps::with_steps(t)
    }

    #[test]
    fn the_flow_summary_joins_the_step_labels_in_execution_order() {
        assert_eq!(
            flow_summary(&trace()),
            "route: planner → planner → RAG retrieval → guides (1 of 24 sections) → llm round 0 → get_thread → answer"
        );
    }

    #[test]
    fn each_step_carries_what_it_read_or_produced() {
        let source = SourceSummary {
            citation_number: 1,
            email_id: "e1".into(),
            subject: "Invoice".into(),
            sender: "Billing".into(),
            sender_email: "billing@example.com".into(),
            relevance_score: None,
            body_snippet: "Servers: CPX31".into(),
        };
        let views = step_views(
            &trace(),
            &outcome(
                vec![source],
                vec!["AI features › Choosing a backend\nOllama at :11434".into()],
            ),
        );
        let by_label = |l: &str| views.iter().find(|v| v.label == l).expect(l);

        assert_eq!(
            by_label("RAG retrieval").blocks,
            [StepBlock {
                title: "[1] Billing <billing@example.com> — Invoice".into(),
                text: "Servers: CPX31".into()
            }]
        );
        assert_eq!(
            by_label("guides (1 of 24 sections)").blocks[0].title,
            "AI features › Choosing a backend"
        );
        let round = by_label("llm round 0");
        assert_eq!(
            round.blocks.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(),
            ["input", "output"]
        );
        assert_eq!(round.detail, "900 ms · 1 tool call");
        let tool = by_label("get_thread");
        assert!(
            tool.blocks[0].text.contains("\"thread_id\": \"t1\""),
            "{}",
            tool.blocks[0].text
        );
        assert_eq!(tool.blocks[1].title, "result (10 chars)");
    }
}
