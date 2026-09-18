// HTML report for the query-planner harness (dark theme, inline CSS, no
// runtime assets — same shape as the chat report so both read alike).

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::query_plan::case_loader::PlanCase;
use crate::evals::query_plan::metrics::{CheckStatus, PlanReport};
use crate::evals::EvalResult;
use crate::services::chat::planner::SearchPlan;

/// One case's inputs and outputs, as the runner collected them.
pub struct ReportCase<'a> {
    pub case: &'a PlanCase,
    pub plan: Option<&'a SearchPlan>,
    pub report: &'a PlanReport,
    pub latency_ms: u128,
}

#[derive(Serialize)]
struct CaseView {
    id: String,
    question: String,
    note: String,
    passed: bool,
    latency_ms: u128,
    /// The plan the model produced, pretty-printed, or "(deferred)".
    plan_json: String,
    checks: Vec<CheckView>,
}

#[derive(Serialize)]
struct CheckView {
    field: String,
    expected: String,
    actual: String,
    /// "pass" | "fail" | "unchecked" — drives the row's colour.
    status: String,
}

/// Write the report and return its path.
pub fn render(out_dir: &Path, model: &str, cases: &[ReportCase<'_>]) -> EvalResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let path = out_dir.join(format!("query_plan_report_{stamp}.html"));

    let views: Vec<CaseView> = cases
        .iter()
        .map(|c| CaseView {
            id: c.case.id.clone(),
            question: c.case.question.clone(),
            note: c.case.note.clone(),
            passed: c.report.passed,
            latency_ms: c.latency_ms,
            plan_json: match c.plan {
                Some(plan) => serde_json::to_string_pretty(plan).unwrap_or_else(|_| "{}".into()),
                None => "(deferred — no search plan)".to_string(),
            },
            checks: c
                .report
                .checks
                .iter()
                .map(|check| CheckView {
                    field: check.field.clone(),
                    expected: check.expected.clone(),
                    actual: check.actual.clone(),
                    status: match check.status {
                        CheckStatus::Pass => "pass",
                        CheckStatus::Fail => "fail",
                        CheckStatus::Unchecked => "unchecked",
                    }
                    .to_string(),
                })
                .collect(),
        })
        .collect();

    let passed = views.iter().filter(|v| v.passed).count();
    let total_ms: u128 = cases.iter().map(|c| c.latency_ms).sum();
    let median_ms = {
        let mut all: Vec<u128> = cases.iter().map(|c| c.latency_ms).collect();
        all.sort_unstable();
        all.get(all.len() / 2).copied().unwrap_or(0)
    };

    let mut tera = Tera::default();
    tera.add_raw_template("report", REPORT_TEMPLATE)?;
    let mut ctx = Context::new();
    ctx.insert("generated_at", &Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string());
    ctx.insert("model", model);
    ctx.insert("cases", &views);
    ctx.insert("total", &views.len());
    ctx.insert("passed", &passed);
    ctx.insert("failed", &(views.len() - passed));
    ctx.insert("total_ms", &total_ms);
    ctx.insert("median_ms", &median_ms);

    std::fs::write(&path, tera.render("report", &ctx)?)?;
    Ok(path)
}

const REPORT_TEMPLATE: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Query planner eval</title>
<style>
  :root {
    --green: #22c55e; --red: #ef4444;
    --bg: #0f172a; --surface: #1e293b; --surface2: #334155;
    --text: #f1f5f9; --text-muted: #94a3b8; --border: #475569; --accent: #7aa2f7;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: var(--bg); color: var(--text); line-height: 1.55;
    padding: 2rem; max-width: 1100px; margin: 0 auto;
  }
  h1 { font-size: 1.6rem; }
  .sub { color: var(--text-muted); font-size: 0.85rem; margin-bottom: 1.5rem; }
  .totals { display: flex; gap: 1rem; flex-wrap: wrap; margin-bottom: 1.5rem; }
  .tile { background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 0.75rem 1rem; }
  .tile .n { font-size: 1.4rem; font-weight: 600; }
  .tile .l { color: var(--text-muted); font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; }
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: 8px; margin-bottom: 1rem; overflow: hidden; }
  .card-head { padding: 0.75rem 1rem; display: flex; justify-content: space-between; gap: 1rem; align-items: flex-start; }
  .q { font-weight: 600; }
  .meta { color: var(--text-muted); font-size: 0.8rem; margin-top: 0.15rem; }
  .note { color: var(--accent); font-size: 0.8rem; margin-top: 0.3rem; }
  .badge { font-size: 0.7rem; padding: 0.15rem 0.5rem; border-radius: 999px; font-weight: 600; white-space: nowrap; }
  .pass { background: rgba(34,197,94,0.15); color: var(--green); }
  .fail { background: rgba(239,68,68,0.15); color: var(--red); }
  .body { display: grid; grid-template-columns: minmax(240px, 1fr) 2fr; gap: 1rem; padding: 0 1rem 1rem; }
  @media (max-width: 720px) { .body { grid-template-columns: 1fr; } }
  pre { background: var(--surface2); border-radius: 6px; padding: 0.75rem; overflow-x: auto;
        font-family: ui-monospace, 'SF Mono', Menlo, monospace; font-size: 0.78rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.8rem;
          font-family: ui-monospace, 'SF Mono', Menlo, monospace; }
  th { text-align: left; color: var(--text-muted); font-weight: 500; padding: 0.3rem 0.5rem 0.3rem 0; }
  td { padding: 0.3rem 0.5rem 0.3rem 0; border-top: 1px solid var(--border); vertical-align: top; }
  td.pass { color: var(--green); } td.fail { color: var(--red); }
  /* Set by the planner, asserted by nobody: shown so a passing case cannot
     hide a field the case never mentioned. */
  td.unchecked { color: var(--text-muted); }
  .label { color: var(--text-muted); font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.05em; margin-bottom: 0.35rem; }
</style>
</head>
<body>
<h1>Query planner eval</h1>
<div class="sub">{{ generated_at }} · model {{ model }}</div>

<div class="totals">
  <div class="tile"><div class="n">{{ passed }}/{{ total }}</div><div class="l">cases passed</div></div>
  <div class="tile"><div class="n">{{ median_ms }}ms</div><div class="l">median plan</div></div>
  <div class="tile"><div class="n">{{ total_ms }}ms</div><div class="l">total</div></div>
</div>

{% for c in cases %}
<div class="card">
  <div class="card-head">
    <div>
      <div class="q">[{{ c.id }}] {{ c.question }}</div>
      <div class="meta">{{ c.latency_ms }}ms</div>
      {% if c.note %}<div class="note">{{ c.note }}</div>{% endif %}
    </div>
    {% if c.passed %}<span class="badge pass">pass</span>{% else %}<span class="badge fail">fail</span>{% endif %}
  </div>
  <div class="body">
    <div>
      <div class="label">Plan produced</div>
      <pre>{{ c.plan_json }}</pre>
    </div>
    <div>
      <div class="label">Field checks</div>
      <table>
        <tr><th>field</th><th>expected</th><th>actual</th></tr>
        {% for ck in c.checks %}
        <tr>
          <td>{{ ck.field }}</td>
          <td>{{ ck.expected }}</td>
          <td class="{{ ck.status }}">{{ ck.actual }}</td>
        </tr>
        {% endfor %}
      </table>
    </div>
  </div>
</div>
{% endfor %}

</body>
</html>
"###;
