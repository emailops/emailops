// HTML report for the tag-classification harness (dark theme, inline CSS, no
// runtime assets — same shape as the other eval reports).

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::tag_classification::runner::{CaseRun, TagMetricsReport};
use crate::evals::{EvalError, EvalResult};

#[derive(Serialize)]
struct CaseView {
    id: String,
    lang: String,
    subject: String,
    snippet: String,
    tags: String,
    passed: bool,
    expected: String,
    actual: String,
    detail: String,
    latency_ms: String,
}

#[derive(Serialize)]
struct AxisView {
    axis: String,
    strict: String,
    accepted: String,
    macro_f1: String,
    worst_labels: String,
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Tag classification — {{ model }}</title>
<style>
 body{background:#111418;color:#e6e6e6;font:14px/1.5 -apple-system,Segoe UI,sans-serif;margin:0;padding:24px}
 h1{font-size:20px;margin:0 0 4px} .sub{color:#8a94a0;margin-bottom:20px}
 table{border-collapse:collapse;width:100%;margin-bottom:28px}
 th,td{border-bottom:1px solid #222a33;padding:6px 8px;text-align:left;vertical-align:top}
 th{color:#8a94a0;font-weight:600;font-size:12px;text-transform:uppercase;letter-spacing:.04em}
 .pass{color:#4ade80}.fail{color:#f87171}
 .mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}
 .snippet{color:#8a94a0;font-size:12px;max-width:480px}
</style></head><body>
<h1>Tag classification — {{ model }}</h1>
<div class="sub">{{ stamp }} · mode {{ mode }} · {{ repeats }} repeat(s) · {{ total }} synthetic cases</div>

<table>
<tr><th>Axis</th><th>Strict</th><th>Accepted</th><th>Macro-F1</th><th>Weakest labels</th></tr>
{% for a in axes %}<tr><td>{{ a.axis }}</td><td>{{ a.strict }}</td><td>{{ a.accepted }}</td><td>{{ a.macro_f1 }}</td><td class="mono">{{ a.worst_labels }}</td></tr>{% endfor %}
</table>

<table>
<tr><th>Metric</th><th>Value</th></tr>
<tr><td>All three axes accepted</td><td>{{ all_axes }}</td></tr>
<tr><td>Repaired / fell back</td><td>{{ repaired }} / {{ fallback }}</td></tr>
<tr><td>Failed calls (unparseable)</td><td>{{ failed }} ({{ unparseable }})</td></tr>
<tr><td>Latency mean / p50 / p95</td><td>{{ latency }}</td></tr>
<tr><td>Prefill mean</td><td>{{ prefill }}</td></tr>
<tr><td>Prompt / completion tokens (mean)</td><td>{{ tokens }}</td></tr>
<tr><td>Throughput</td><td>{{ throughput }}</td></tr>
</table>

<table>
<tr><th>Case</th><th>Expected</th><th>Actual</th><th>ms</th></tr>
{% for c in cases %}<tr>
<td><div class="{% if c.passed %}pass{% else %}fail{% endif %}">{{ c.id }}</div>
<div class="snippet">[{{ c.lang }}] {{ c.subject }} — {{ c.snippet }}{% if c.tags %} <em>({{ c.tags }})</em>{% endif %}</div></td>
<td class="mono">{{ c.expected }}</td>
<td class="mono">{{ c.actual }}{% if c.detail %}<div class="fail">{{ c.detail }}</div>{% endif %}</td>
<td class="mono">{{ c.latency_ms }}</td>
</tr>{% endfor %}
</table>
</body></html>"#;

/// Write the report and return its path.
pub fn render(out_dir: &Path, metrics: &TagMetricsReport, runs: &[CaseRun]) -> EvalResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let path = out_dir.join(format!("tag_classification_report_{stamp}.html"));

    let pct = |v: Option<f64>| v.map(|x| format!("{:.1}%", x * 100.0)).unwrap_or_else(|| "n/a".into());
    let axes: Vec<AxisView> = [
        ("intent", &metrics.intent),
        ("topic", &metrics.topic),
        ("urgency", &metrics.urgency),
    ]
    .into_iter()
    .map(|(axis, score)| {
        let mut labels = score.labels.clone();
        labels.sort_by(|a, b| a.f1.total_cmp(&b.f1));
        AxisView {
            axis: axis.to_string(),
            strict: pct(score.strict_accuracy()),
            accepted: pct(score.accepted_accuracy()),
            macro_f1: score
                .macro_f1
                .map(|f| format!("{f:.3}"))
                .unwrap_or_else(|| "n/a".into()),
            worst_labels: labels
                .iter()
                .take(3)
                .map(|l| format!("{} {:.2}", l.label, l.f1))
                .collect::<Vec<_>>()
                .join(" · "),
        }
    })
    .collect();

    let cases: Vec<CaseView> = runs
        .iter()
        .map(|r| CaseView {
            id: r.case.id.clone(),
            lang: r.case.lang.clone(),
            subject: r.case.subject.clone(),
            snippet: r.case.snippet.clone(),
            tags: r.case.tags.join(", "),
            passed: r.passed(),
            expected: format!(
                "{} / {} / {}",
                r.case.expect.intent.join("|"),
                r.case.expect.topic.join("|"),
                r.case.expect.urgency.join("|")
            ),
            actual: format!(
                "{} / {} / {}",
                r.intent.as_deref().unwrap_or("-"),
                r.topic.as_deref().unwrap_or("-"),
                r.urgency.as_deref().unwrap_or("-")
            ),
            detail: r.failure.clone().unwrap_or_default(),
            latency_ms: r
                .latencies_ms
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" / "),
        })
        .collect();

    let mut ctx = Context::new();
    ctx.insert("model", &metrics.model);
    ctx.insert("mode", &metrics.mode);
    ctx.insert("repeats", &metrics.repeats);
    ctx.insert("stamp", &stamp);
    ctx.insert("total", &metrics.total_cases);
    ctx.insert("axes", &axes);
    ctx.insert("cases", &cases);
    ctx.insert(
        "all_axes",
        &format!("{}/{}", metrics.all_axes_accepted, metrics.total_cases),
    );
    ctx.insert("repaired", &metrics.repaired_cases);
    ctx.insert("fallback", &metrics.fallback_cases);
    ctx.insert("failed", &metrics.failed_calls);
    ctx.insert("unparseable", &metrics.unparseable_replies);
    ctx.insert(
        "latency",
        &format!(
            "{} / {} / {} ms",
            fmt_f(metrics.latency_ms_mean),
            fmt_u(metrics.latency_ms_p50),
            fmt_u(metrics.latency_ms_p95)
        ),
    );
    ctx.insert("prefill", &format!("{} ms", fmt_f(metrics.prefill_ms_mean)));
    ctx.insert(
        "tokens",
        &format!(
            "{} / {} (cached {})",
            fmt_f(metrics.prompt_tokens_mean),
            fmt_f(metrics.completion_tokens_mean),
            fmt_f(metrics.cached_prompt_tokens_mean)
        ),
    );
    ctx.insert(
        "throughput",
        &metrics
            .emails_per_minute
            .map(|v| format!("{v:.1} emails/min"))
            .unwrap_or_else(|| "n/a".into()),
    );

    let html = Tera::one_off(TEMPLATE, &ctx, true).map_err(|e| EvalError::Config(format!("report render: {e}")))?;
    std::fs::write(&path, html)?;
    Ok(path)
}

fn fmt_f(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.0}")).unwrap_or_else(|| "n/a".into())
}

fn fmt_u(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "n/a".into())
}
