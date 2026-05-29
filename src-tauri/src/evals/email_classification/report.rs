// HTML report for the email_classification eval.
//
// Header: account / model / generated-at + summary pills + label distribution.
// Body:   one row per email with email · prediction · (optional) judge.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::email_classification::runner::JudgeVerdict;
use crate::evals::email_classification::LABELS;
use crate::evals::EvalResult;
use crate::models::Email;

pub struct ReportCase {
    pub email_id: String,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub timestamp: i64,
    pub body_plain: String,
    pub predicted_label: Option<String>,
    pub raw_output: String,
    pub classify_ms: i64,
    pub status: CaseStatus,
    pub error: Option<String>,
    pub verdict: Option<JudgeVerdict>,
}

pub enum CaseStatus {
    Ok,
    Error,
}

impl ReportCase {
    #[allow(clippy::too_many_arguments)]
    pub fn ok(
        email_id: &str,
        email: &Email,
        body_plain: String,
        predicted_label: Option<String>,
        raw_output: String,
        classify_ms: i64,
        verdict: Option<JudgeVerdict>,
    ) -> Self {
        Self {
            email_id: email_id.to_string(),
            subject: email.subject.clone(),
            sender: email.sender.clone(),
            sender_email: email.sender_email.clone(),
            timestamp: email.timestamp,
            body_plain,
            predicted_label,
            raw_output,
            classify_ms,
            status: CaseStatus::Ok,
            error: None,
            verdict,
        }
    }

    pub fn error(email_id: &str, message: String) -> Self {
        Self {
            email_id: email_id.to_string(),
            subject: "(unavailable)".into(),
            sender: String::new(),
            sender_email: String::new(),
            timestamp: 0,
            body_plain: String::new(),
            predicted_label: None,
            raw_output: String::new(),
            classify_ms: 0,
            status: CaseStatus::Error,
            error: Some(message),
            verdict: None,
        }
    }
}

#[derive(Serialize)]
struct CaseView {
    index: usize,
    email_id: String,
    subject: String,
    sender: String,
    sender_email: String,
    date: String,
    body_excerpt: String,
    predicted_label: Option<String>,
    label_class: String,
    raw_output: String,
    classify_ms: i64,
    status_label: String,
    status_class: String,
    error: Option<String>,
    judge: Option<JudgeView>,
}

#[derive(Serialize)]
struct JudgeView {
    correct: Option<bool>,
    correct_label: String,
    correct_class: String,
    suggested_label: Option<String>,
    rationale: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct LabelCount {
    label: String,
    count: usize,
    pct: f64,
    class: String,
}

#[derive(Serialize)]
struct SummaryView {
    generated_at: String,
    account: String,
    model: String,
    total: usize,
    ok_count: usize,
    error_count: usize,
    parsed_count: usize,
    unparsed_count: usize,
    mean_latency_ms: i64,
    p95_latency_ms: i64,
    judge_enabled: bool,
    judge_model: String,
    judge_correct: usize,
    judge_incorrect: usize,
    judge_unscored: usize,
    judge_accuracy_pct: Option<i32>,
    distribution: Vec<LabelCount>,
}

pub fn render_report(
    out_dir: &Path,
    cases: &[ReportCase],
    account: &str,
    model: &str,
    judge_enabled: bool,
    judge_model: &str,
) -> EvalResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("email_classification_report_{}.html", stamp);
    let path = out_dir.join(filename);

    let mut tera = Tera::default();
    tera.add_raw_template("report", REPORT_TEMPLATE)?;

    let mut ok_count = 0usize;
    let mut error_count = 0usize;
    let mut parsed_count = 0usize;
    let mut unparsed_count = 0usize;
    let mut latencies: Vec<i64> = Vec::new();
    let mut judge_correct = 0usize;
    let mut judge_incorrect = 0usize;
    let mut judge_unscored = 0usize;

    let mut counts: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();

    let mut views: Vec<CaseView> = Vec::with_capacity(cases.len());
    for (i, c) in cases.iter().enumerate() {
        let (status_label, status_class) = match c.status {
            CaseStatus::Ok => ("OK".to_string(), "ok".to_string()),
            CaseStatus::Error => ("ERROR".to_string(), "error".to_string()),
        };
        match c.status {
            CaseStatus::Ok => {
                ok_count += 1;
                latencies.push(c.classify_ms);
                if let Some(label) = c.predicted_label.as_deref() {
                    parsed_count += 1;
                    if let Some(canon) = LABELS.iter().find(|l| l.eq_ignore_ascii_case(label)) {
                        *counts.entry(*canon).or_insert(0) += 1;
                    }
                } else {
                    unparsed_count += 1;
                }
            }
            CaseStatus::Error => error_count += 1,
        }

        let judge_view = c.verdict.as_ref().map(|v| {
            match v.correct {
                Some(true) => judge_correct += 1,
                Some(false) => judge_incorrect += 1,
                None => judge_unscored += 1,
            };
            let (label, class) = match v.correct {
                Some(true) => ("CORRECT", "judge-good"),
                Some(false) => ("INCORRECT", "judge-poor"),
                None => ("—", "judge-na"),
            };
            JudgeView {
                correct: v.correct,
                correct_label: label.into(),
                correct_class: class.into(),
                suggested_label: v.suggested_label.clone(),
                rationale: v.rationale.clone(),
                error: v.error.clone(),
            }
        });

        views.push(CaseView {
            index: i + 1,
            email_id: c.email_id.clone(),
            subject: c.subject.clone(),
            sender: c.sender.clone(),
            sender_email: c.sender_email.clone(),
            date: format_ts(c.timestamp),
            body_excerpt: truncate(&c.body_plain, 1500),
            predicted_label: c.predicted_label.clone(),
            label_class: c
                .predicted_label
                .as_deref()
                .map(label_to_class)
                .unwrap_or_else(|| "label-na".into()),
            raw_output: truncate(&c.raw_output, 200),
            classify_ms: c.classify_ms,
            status_label,
            status_class,
            error: c.error.clone(),
            judge: judge_view,
        });
    }

    let mean_latency_ms = if !latencies.is_empty() {
        latencies.iter().sum::<i64>() / latencies.len() as i64
    } else {
        0
    };
    let p95_latency_ms = percentile(&mut latencies, 0.95);

    // Build label distribution in canonical order so the histogram is stable
    // across runs even when some labels are absent.
    let total_predicted = parsed_count.max(1);
    let distribution: Vec<LabelCount> = LABELS
        .iter()
        .map(|l| {
            let count = counts.get(l).copied().unwrap_or(0);
            LabelCount {
                label: (*l).into(),
                count,
                pct: count as f64 * 100.0 / total_predicted as f64,
                class: label_to_class(l),
            }
        })
        .collect();

    let judge_accuracy_pct = if judge_correct + judge_incorrect > 0 {
        Some(((judge_correct as f64 / (judge_correct + judge_incorrect) as f64) * 100.0).round() as i32)
    } else {
        None
    };

    let summary = SummaryView {
        generated_at: Utc::now().to_rfc3339(),
        account: account.into(),
        model: model.into(),
        total: cases.len(),
        ok_count,
        error_count,
        parsed_count,
        unparsed_count,
        mean_latency_ms,
        p95_latency_ms,
        judge_enabled,
        judge_model: judge_model.into(),
        judge_correct,
        judge_incorrect,
        judge_unscored,
        judge_accuracy_pct,
        distribution,
    };

    let mut ctx = Context::new();
    ctx.insert("summary", &summary);
    ctx.insert("cases", &views);
    let html = tera.render("report", &ctx)?;
    std::fs::write(&path, html)?;
    Ok(path)
}

fn label_to_class(label: &str) -> String {
    format!("label-{}", label.to_ascii_lowercase())
}

fn format_ts(ts: i64) -> String {
    if ts == 0 {
        return "-".into();
    }
    chrono::DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "-".into())
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

fn percentile(xs: &mut [i64], p: f64) -> i64 {
    if xs.is_empty() {
        return 0;
    }
    xs.sort_unstable();
    let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[idx]
}

const REPORT_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Email classification eval · {{ summary.account }}</title>
<style>
  :root {
    --bg: #0e0f12; --panel: #1a1c22; --panel-2: #23262e; --border: #2e323c;
    --fg: #e6e8ee; --fg-dim: #9aa0ac; --accent: #7aa2ff;
    --ok: #3ecf8e; --warn: #ffc94a; --err: #ff6a6a;
  }
  body { margin: 0; background: var(--bg); color: var(--fg); font: 13px/1.5 -apple-system, BlinkMacSystemFont, sans-serif; }
  header { padding: 16px 24px; background: var(--panel); border-bottom: 1px solid var(--border); }
  h1 { margin: 0 0 6px; font-size: 18px; font-weight: 600; }
  .meta { color: var(--fg-dim); font-size: 12px; }
  .meta span { margin-right: 16px; }
  .stats { margin-top: 8px; font-size: 12px; }
  .stats .pill { display: inline-block; padding: 2px 10px; border-radius: 10px; margin-right: 6px; background: var(--panel-2); border: 1px solid var(--border); }
  .pill.ok { color: var(--ok); border-color: #2e5a45; }
  .pill.warn { color: var(--warn); border-color: #5a4a1e; }
  .pill.error { color: var(--err); border-color: #5a2e2e; }
  .dist { margin-top: 14px; display: flex; flex-wrap: wrap; gap: 6px; }
  .dist .bar { display: flex; align-items: center; gap: 6px; font-size: 11px; padding: 3px 8px; background: var(--panel-2); border: 1px solid var(--border); border-radius: 4px; }
  .dist .bar .swatch { width: 10px; height: 10px; border-radius: 2px; }
  .dist .bar .pct { color: var(--fg-dim); }
  main { padding: 16px 24px 48px; }
  .case { background: var(--panel); border: 1px solid var(--border); border-radius: 8px; margin-bottom: 14px; overflow: hidden; }
  .case-header { display: flex; gap: 12px; align-items: center; padding: 8px 14px; border-bottom: 1px solid var(--border); background: var(--panel-2); font-size: 12px; color: var(--fg-dim); }
  .case-header .idx { color: var(--accent); font-weight: 600; }
  .status { padding: 2px 8px; border-radius: 10px; font-weight: 600; font-size: 11px; }
  .status.ok { color: var(--ok); background: rgba(62,207,142,0.1); }
  .status.error { color: var(--err); background: rgba(255,106,106,0.1); }
  .cols { display: grid; grid-template-columns: 1.4fr 1fr 1fr; gap: 1px; background: var(--border); }
  .col { background: var(--panel); padding: 12px 14px; min-width: 0; }
  .col h3 { margin: 0 0 8px; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--fg-dim); font-weight: 600; }
  .email-subject { font-weight: 600; margin-bottom: 4px; word-break: break-word; }
  .email-from { font-size: 12px; color: var(--fg-dim); margin-bottom: 8px; }
  .email-body { font-size: 12px; white-space: pre-wrap; color: var(--fg); max-height: 260px; overflow-y: auto; padding: 6px; background: var(--panel-2); border-radius: 4px; }
  .label-chip { display: inline-block; padding: 4px 12px; border-radius: 14px; font-weight: 600; font-size: 13px; }
  .label-na { color: var(--fg-dim); background: var(--panel-2); }
  .label-billing     { color: #ff8a3d; background: rgba(255,138,61,0.10); }
  .label-newsletter  { color: #7aa2ff; background: rgba(122,162,255,0.10); }
  .label-work        { color: #3ecf8e; background: rgba(62,207,142,0.10); }
  .label-personal    { color: #c08bff; background: rgba(192,139,255,0.10); }
  .label-promotional { color: #ffc94a; background: rgba(255,201,74,0.10); }
  .label-security    { color: #ff6a6a; background: rgba(255,106,106,0.10); }
  .label-shipping    { color: #5acdb6; background: rgba(90,205,182,0.10); }
  .label-travel      { color: #6ec1ff; background: rgba(110,193,255,0.10); }
  .label-spam        { color: #b86a6a; background: rgba(184,106,106,0.10); }
  .label-other       { color: #9aa0ac; background: rgba(154,160,172,0.10); }
  .raw { margin-top: 8px; font-family: ui-monospace, SFMono-Regular, monospace; font-size: 11px; padding: 6px 8px; background: var(--panel-2); color: var(--fg-dim); border-radius: 4px; word-break: break-all; }
  .judge-line { display: flex; align-items: center; gap: 10px; margin-bottom: 6px; }
  .judge-label { padding: 2px 8px; border-radius: 10px; font-size: 11px; font-weight: 600; text-transform: uppercase; }
  .judge-good { color: var(--ok); background: rgba(62,207,142,0.1); }
  .judge-poor { color: var(--err); background: rgba(255,106,106,0.1); }
  .judge-na { color: var(--fg-dim); background: var(--panel-2); }
  .rationale { font-size: 12px; color: var(--fg); margin-top: 8px; padding: 6px 8px; background: var(--panel-2); border-radius: 4px; white-space: pre-wrap; }
  .empty { color: var(--fg-dim); font-style: italic; font-size: 12px; }
  .judge-error { color: var(--err); font-size: 11px; padding: 4px 6px; background: rgba(255,106,106,0.08); border-radius: 4px; }
</style>
</head>
<body>
<header>
  <h1>Email classification eval — distil-labs/distil-email-classifier</h1>
  <div class="meta">
    <span><strong>account</strong> {{ summary.account }}</span>
    <span><strong>model</strong> {{ summary.model }}</span>
    <span><strong>generated</strong> {{ summary.generated_at }}</span>
  </div>
  <div class="stats">
    <span class="pill">{{ summary.total }} total</span>
    <span class="pill ok">{{ summary.ok_count }} ok</span>
    {% if summary.error_count > 0 %}<span class="pill error">{{ summary.error_count }} errors</span>{% endif %}
    {% if summary.unparsed_count > 0 %}<span class="pill warn">{{ summary.unparsed_count }} unparsed</span>{% endif %}
    <span class="pill">mean {{ summary.mean_latency_ms }} ms</span>
    <span class="pill">p95 {{ summary.p95_latency_ms }} ms</span>
    {% if summary.judge_enabled %}
      {% if summary.judge_accuracy_pct %}<span class="pill ok">judge: {{ summary.judge_accuracy_pct }}% correct</span>{% endif %}
      <span class="pill">{{ summary.judge_correct }}✓ {{ summary.judge_incorrect }}✗ ({{ summary.judge_model }})</span>
    {% else %}
      <span class="pill">judge disabled</span>
    {% endif %}
  </div>
  <div class="dist">
    {% for d in summary.distribution %}
      <span class="bar"><span class="swatch label-chip {{ d.class }}">&nbsp;</span><strong>{{ d.label }}</strong> {{ d.count }} <span class="pct">({{ d.pct | round(method="common", precision=1) }}%)</span></span>
    {% endfor %}
  </div>
</header>
<main>
{% for c in cases %}
  <section class="case">
    <div class="case-header">
      <span class="idx">#{{ c.index }}</span>
      <span>{{ c.email_id }}</span>
      <span class="status {{ c.status_class }}">{{ c.status_label }}</span>
      {% if c.classify_ms > 0 %}<span>{{ c.classify_ms }} ms</span>{% endif %}
    </div>
    {% if c.error %}
      <div class="col"><div class="judge-error">{{ c.error }}</div></div>
    {% else %}
    <div class="cols">
      <div class="col">
        <h3>Email</h3>
        <div class="email-subject">{{ c.subject }}</div>
        <div class="email-from">{{ c.sender }} &lt;{{ c.sender_email }}&gt; · {{ c.date }}</div>
        <div class="email-body">{{ c.body_excerpt }}</div>
      </div>
      <div class="col">
        <h3>Predicted label</h3>
        {% if c.predicted_label %}
          <span class="label-chip {{ c.label_class }}">{{ c.predicted_label }}</span>
        {% else %}
          <span class="label-chip label-na">unparsed</span>
        {% endif %}
        {% if c.raw_output %}<div class="raw">raw: {{ c.raw_output }}</div>{% endif %}
      </div>
      <div class="col">
        <h3>Judge verdict</h3>
        {% if c.judge %}
          {% if c.judge.error %}
            <div class="judge-error">{{ c.judge.error }}</div>
          {% else %}
            <div class="judge-line">
              <span class="judge-label {{ c.judge.correct_class }}">{{ c.judge.correct_label }}</span>
              {% if c.judge.suggested_label %}<span class="empty">suggested: <strong>{{ c.judge.suggested_label }}</strong></span>{% endif %}
            </div>
            {% if c.judge.rationale %}<div class="rationale">{{ c.judge.rationale }}</div>{% endif %}
          {% endif %}
        {% else %}
          <div class="empty">Judge disabled.</div>
        {% endif %}
      </div>
    </div>
    {% endif %}
  </section>
{% endfor %}
</main>
</body>
</html>"#;
