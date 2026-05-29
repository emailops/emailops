// HTML report rendering (dark theme, inline CSS, self-contained).
//
// We keep the Tera template as an inline string so the report can be produced
// from a single binary without chasing template files at runtime.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::case_loader::EvalCase;
use crate::evals::harness::{CaseOutcome, SourceSummary};
use crate::evals::judge::JudgeScores;
use crate::evals::metrics::HeuristicReport;
use crate::evals::EvalResult;

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
    route_mode: Option<String>,
    route_reason: Option<String>,
    route_classifier: Option<String>,
    retrieval: Option<RetrievalView>,
    tool_calls: Vec<ToolCallView>,
    heuristics: Vec<CheckView>,
    metric_rows: Vec<MetricRowView>,
    sources: Vec<SourceView>,
    judge_error: Option<String>,
    judge_rationale: Option<String>,
}

#[derive(Serialize)]
struct RetrievalView {
    vector_hits: i32,
    fts_hits: i32,
    fused_top_k: i32,
    elapsed_ms: i64,
    vector_fallback: bool,
}

#[derive(Serialize)]
struct ToolCallView {
    name: String,
    arguments_json: String,
    result_preview: String,
    result_chars: i32,
    elapsed_ms: i64,
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
struct SourceView {
    citation_number: i32,
    email_id: String,
    subject: String,
    sender: String,
    sender_email: String,
    score: Option<f32>,
    body_snippet: String,
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
        let overall_pass = rc.heuristics.all_passed();
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
    let route_mode = trace.map(|t| format!("{:?}", t.route.mode));
    let route_reason = trace.map(|t| t.route.reason.clone());
    let route_classifier = trace.map(|t| t.route.classifier.clone());

    let retrieval = trace.and_then(|t| t.retrieval.as_ref()).map(|r| RetrievalView {
        vector_hits: r.vector_hits,
        fts_hits: r.fts_hits,
        fused_top_k: r.fused_top_k,
        elapsed_ms: r.elapsed_ms,
        vector_fallback: r.vector_fallback,
    });

    let tool_calls = trace
        .map(|t| {
            t.tool_calls
                .iter()
                .map(|tc| ToolCallView {
                    name: tc.name.clone(),
                    arguments_json: serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into()),
                    result_preview: tc.result_preview.clone(),
                    result_chars: tc.result_chars,
                    elapsed_ms: tc.elapsed_ms,
                })
                .collect()
        })
        .unwrap_or_default();

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

    let sources = rc
        .outcome
        .sources_used
        .iter()
        .map(|s: &SourceSummary| SourceView {
            citation_number: s.citation_number,
            email_id: s.email_id.clone(),
            subject: s.subject.clone(),
            sender: s.sender.clone(),
            sender_email: s.sender_email.clone(),
            score: s.relevance_score,
            body_snippet: s.body_snippet.clone(),
        })
        .collect();

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
        route_mode,
        route_reason,
        route_classifier,
        retrieval,
        tool_calls,
        heuristics,
        metric_rows,
        sources,
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
    </div>
    <div>
      {% if c.overall_pass %}<span class="badge badge-pass">pass</span>{% else %}<span class="badge badge-fail">fail</span>{% endif %}
      <span class="tc-chevron">▾</span>
    </div>
  </div>
  <div class="tc-body">

    <div class="section">
      <div class="section-label">Route</div>
      <div class="content-box mono">
        mode: {{ c.route_mode | default(value="?") }} · classifier: {{ c.route_classifier | default(value="?") }}
        {% if c.route_reason %}
        reason: {{ c.route_reason }}
        {% endif %}
      </div>
    </div>

    {% if c.retrieval %}
    <div class="section">
      <div class="section-label">Retrieval</div>
      <div class="retr-stats">
        <span><strong>vector:</strong> {{ c.retrieval.vector_hits }}</span>
        <span><strong>fts:</strong> {{ c.retrieval.fts_hits }}</span>
        <span><strong>fused top-k:</strong> {{ c.retrieval.fused_top_k }}</span>
        <span><strong>time:</strong> {{ c.retrieval.elapsed_ms }}ms</span>
      </div>
      {% if c.retrieval.vector_fallback %}<div class="warning">vector search fell back to FTS-only</div>{% endif %}
    </div>
    {% endif %}

    {% if c.tool_calls | length > 0 %}
    <div class="section">
      <div class="section-label">Tool calls</div>
      {% for tc in c.tool_calls %}
      <div class="tool-call">
        <div class="tool-head"><span>{{ tc.name }}</span><span>{{ tc.elapsed_ms }}ms · {{ tc.result_chars }} chars</span></div>
        <div class="tool-args mono">{{ tc.arguments_json }}</div>
        <details class="tool-result"><summary>result ({{ tc.result_chars }} chars) — click to expand</summary><pre>{{ tc.result_preview }}</pre></details>
      </div>
      {% endfor %}
    </div>
    {% endif %}

    {% if c.sources | length > 0 %}
    <div class="section">
      <div class="section-label">Sources (RAG context fed to model)</div>
      <ul class="sources">
        {% for s in c.sources %}
        <li>
          <strong>[{{ s.citation_number }}] {{ s.sender }} &lt;{{ s.sender_email }}&gt; — {{ s.subject }}</strong>
          {% if s.score %}<span style="color:var(--text-muted);font-size:0.72rem"> · score {{ s.score | round(precision=4) }}</span>{% endif %}
          {% if s.body_snippet %}
          <details style="margin-top:0.35rem;">
            <summary style="cursor:pointer;font-size:0.76rem;color:var(--text-muted);user-select:none;">chunk fed to model ({{ s.body_snippet | length }} chars) — click to expand</summary>
            <pre style="margin:0.3rem 0 0;padding:0.5rem 0.6rem;background:#0b1020;color:#d9e2f3;font-size:0.74rem;border-radius:6px;max-height:320px;overflow:auto;white-space:pre-wrap;word-break:break-word;">{{ s.body_snippet }}</pre>
          </details>
          {% endif %}
        </li>
        {% endfor %}
      </ul>
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
