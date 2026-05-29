// 3-column HTML report for the extraction eval.
//
//   ┌───────────────┬──────────────────────┬────────────────────┐
//   │  EMAIL        │  EXTRACTED (tasks    │  JUDGE VERDICT     │
//   │  subject      │   or facts)          │  score + rationale │
//   │  from · date  │                      │                    │
//   │  body         │                      │                    │
//   └───────────────┴──────────────────────┴────────────────────┘

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::extraction::judge::{EmailSummary, JudgeVerdict};
use crate::evals::extraction::ExtractionKind;
use crate::evals::EvalResult;
use crate::models::Email;
use crate::services::memory::extractor::{ExtractedFact, ExtractedPayload, ExtractedTask};

// ── Public case type ────────────────────────────────────────────────────────

pub struct ReportCase {
    pub email_id: String,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub timestamp: i64,
    pub body_plain: String,
    pub tasks: Vec<ExtractedTask>,
    pub facts: Vec<ExtractedFact>,
    pub thread_summary: Option<String>,
    pub commitment: Option<String>,
    pub deadline_iso: Option<String>,
    pub verdict: Option<JudgeVerdict>,
    pub extract_ms: i64,
    pub status: CaseStatus,
    pub error: Option<String>,
}

pub enum CaseStatus {
    Ok,
    Skipped(String),
    Error,
}

impl ReportCase {
    pub fn ok(
        email_id: &str,
        email: &Email,
        summary: &EmailSummary,
        payload: &ExtractedPayload,
        thread_summary: Option<String>,
        commitment: Option<String>,
        deadline_iso: Option<String>,
        extract_ms: i64,
        verdict: Option<JudgeVerdict>,
    ) -> Self {
        Self {
            email_id: email_id.to_string(),
            subject: email.subject.clone(),
            sender: email.sender.clone(),
            sender_email: email.sender_email.clone(),
            timestamp: email.timestamp,
            body_plain: summary.body_plain.clone(),
            tasks: payload.tasks.clone(),
            facts: payload.facts.clone(),
            thread_summary,
            commitment,
            deadline_iso,
            verdict,
            extract_ms,
            status: CaseStatus::Ok,
            error: None,
        }
    }

    pub fn skipped(email_id: &str, email: &Email, summary: &EmailSummary, reason: &str) -> Self {
        Self {
            email_id: email_id.to_string(),
            subject: email.subject.clone(),
            sender: email.sender.clone(),
            sender_email: email.sender_email.clone(),
            timestamp: email.timestamp,
            body_plain: summary.body_plain.clone(),
            tasks: Vec::new(),
            facts: Vec::new(),
            thread_summary: None,
            commitment: None,
            deadline_iso: None,
            verdict: None,
            extract_ms: 0,
            status: CaseStatus::Skipped(reason.to_string()),
            error: None,
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
            tasks: Vec::new(),
            facts: Vec::new(),
            thread_summary: None,
            commitment: None,
            deadline_iso: None,
            verdict: None,
            extract_ms: 0,
            status: CaseStatus::Error,
            error: Some(message),
        }
    }
}

// ── Template-facing views ───────────────────────────────────────────────────

#[derive(Serialize)]
struct CaseView {
    index: usize,
    email_id: String,
    subject: String,
    sender: String,
    sender_email: String,
    date: String,
    body_excerpt: String,
    tasks: Vec<TaskView>,
    facts: Vec<FactView>,
    thread_summary: Option<String>,
    commitment: Option<String>,
    deadline_iso: Option<String>,
    extract_ms: i64,
    status_label: String,
    status_class: String,
    error: Option<String>,
    verdict: Option<VerdictView>,
}

#[derive(Serialize)]
struct TaskView {
    title: String,
    detail: Option<String>,
    priority: Option<String>,
    due_at_iso: Option<String>,
}

#[derive(Serialize)]
struct FactView {
    subject_kind: String,
    subject_key: String,
    fact: String,
    confidence: Option<f64>,
}

#[derive(Serialize)]
struct VerdictView {
    score_pct: Option<i32>,
    score_class: String,
    verdict_label: String,
    verdict_class: String,
    missed: Vec<String>,
    spurious: Vec<String>,
    rationale: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct SummaryView {
    generated_at: String,
    kind: String,
    kind_upper: String,
    account: String,
    model: String,
    total: usize,
    ok_count: usize,
    skipped_count: usize,
    error_count: usize,
    mean_score: Option<f64>,
    judge_enabled: bool,
    judge_model: String,
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn render_report(
    out_dir: &Path,
    cases: &[ReportCase],
    kind: ExtractionKind,
    account: &str,
    model: &str,
    judge_enabled: bool,
    judge_model: &str,
) -> EvalResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("{}_extract_report_{}.html", kind.label(), stamp);
    let path = out_dir.join(filename);

    let mut tera = Tera::default();
    tera.add_raw_template("report", REPORT_TEMPLATE)?;

    let mut ok_count = 0usize;
    let mut skipped_count = 0usize;
    let mut error_count = 0usize;
    let mut score_sum = 0.0f64;
    let mut score_n = 0usize;

    let mut views: Vec<CaseView> = Vec::with_capacity(cases.len());
    for (i, c) in cases.iter().enumerate() {
        let (status_label, status_class) = match &c.status {
            CaseStatus::Ok => ("OK".to_string(), "ok".to_string()),
            CaseStatus::Skipped(r) => (format!("SKIPPED · {}", r), "skipped".to_string()),
            CaseStatus::Error => ("ERROR".to_string(), "error".to_string()),
        };
        match c.status {
            CaseStatus::Ok => ok_count += 1,
            CaseStatus::Skipped(_) => skipped_count += 1,
            CaseStatus::Error => error_count += 1,
        }

        let verdict_view = c.verdict.as_ref().map(|v| {
            if let Some(s) = v.score {
                score_sum += s;
                score_n += 1;
            }
            let score_pct = v.score.map(|s| (s * 100.0).round() as i32);
            let score_class = match v.score {
                Some(s) if s >= 0.75 => "score-good",
                Some(s) if s >= 0.5 => "score-ok",
                Some(_) => "score-poor",
                None => "score-na",
            }
            .to_string();
            let verdict_label = v.verdict.clone().unwrap_or_else(|| "-".into());
            let verdict_class = match verdict_label.to_ascii_lowercase().as_str() {
                "good" | "empty" => "verdict-good",
                "ok" => "verdict-ok",
                "poor" => "verdict-poor",
                _ => "verdict-na",
            }
            .to_string();
            VerdictView {
                score_pct,
                score_class,
                verdict_label,
                verdict_class,
                missed: v.missed.clone(),
                spurious: v.spurious.clone(),
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
            body_excerpt: truncate(&c.body_plain, 2000),
            tasks: c
                .tasks
                .iter()
                .map(|t| TaskView {
                    title: t.title.clone(),
                    detail: t.detail.clone(),
                    priority: t.priority.clone(),
                    due_at_iso: t.due_at_iso.clone(),
                })
                .collect(),
            facts: c
                .facts
                .iter()
                .map(|f| FactView {
                    subject_kind: f.subject_kind.clone(),
                    subject_key: f.subject_key.clone(),
                    fact: f.fact.clone(),
                    confidence: f.confidence,
                })
                .collect(),
            thread_summary: c.thread_summary.clone(),
            commitment: c.commitment.clone(),
            deadline_iso: c.deadline_iso.clone(),
            extract_ms: c.extract_ms,
            status_label,
            status_class,
            error: c.error.clone(),
            verdict: verdict_view,
        });
    }

    let mean_score = if score_n > 0 {
        Some(score_sum / score_n as f64)
    } else {
        None
    };

    let summary = SummaryView {
        generated_at: Utc::now().to_rfc3339(),
        kind: kind.label().to_string(),
        kind_upper: kind.label().to_uppercase(),
        account: account.to_string(),
        model: model.to_string(),
        total: cases.len(),
        ok_count,
        skipped_count,
        error_count,
        mean_score,
        judge_enabled,
        judge_model: judge_model.to_string(),
    };

    let mut ctx = Context::new();
    ctx.insert("summary", &summary);
    ctx.insert("cases", &views);
    let html = tera.render("report", &ctx)?;
    std::fs::write(&path, html)?;
    Ok(path)
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

const REPORT_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Extraction eval · {{ summary.kind_upper }} · {{ summary.account }}</title>
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
  .pill.skipped { color: var(--warn); border-color: #5a4a1e; }
  .pill.error { color: var(--err); border-color: #5a2e2e; }
  main { padding: 16px 24px 48px; }
  .case { background: var(--panel); border: 1px solid var(--border); border-radius: 8px; margin-bottom: 14px; overflow: hidden; }
  .case-header { display: flex; gap: 12px; align-items: center; padding: 8px 14px; border-bottom: 1px solid var(--border); background: var(--panel-2); font-size: 12px; color: var(--fg-dim); }
  .case-header .idx { color: var(--accent); font-weight: 600; }
  .status { padding: 2px 8px; border-radius: 10px; font-weight: 600; font-size: 11px; }
  .status.ok { color: var(--ok); background: rgba(62,207,142,0.1); }
  .status.skipped { color: var(--warn); background: rgba(255,201,74,0.1); }
  .status.error { color: var(--err); background: rgba(255,106,106,0.1); }
  .cols { display: grid; grid-template-columns: 1.2fr 1fr 1fr; gap: 1px; background: var(--border); }
  .col { background: var(--panel); padding: 12px 14px; min-width: 0; }
  .col h3 { margin: 0 0 8px; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--fg-dim); font-weight: 600; }
  .email-subject { font-weight: 600; margin-bottom: 4px; word-break: break-word; }
  .email-from { font-size: 12px; color: var(--fg-dim); margin-bottom: 8px; }
  .email-body { font-size: 12px; white-space: pre-wrap; color: var(--fg); max-height: 260px; overflow-y: auto; padding: 6px; background: var(--panel-2); border-radius: 4px; }
  .items { list-style: none; padding: 0; margin: 0; }
  .items li { background: var(--panel-2); border-radius: 4px; padding: 6px 8px; margin-bottom: 6px; font-size: 12px; }
  .item-title { font-weight: 600; margin-bottom: 2px; }
  .item-meta { font-size: 11px; color: var(--fg-dim); }
  .empty { color: var(--fg-dim); font-style: italic; font-size: 12px; }
  .score-line { display: flex; align-items: center; gap: 10px; margin-bottom: 6px; }
  .score-pct { font-size: 22px; font-weight: 700; }
  .score-good { color: var(--ok); }
  .score-ok { color: var(--warn); }
  .score-poor { color: var(--err); }
  .score-na { color: var(--fg-dim); }
  .verdict-label { padding: 2px 8px; border-radius: 10px; font-size: 11px; font-weight: 600; text-transform: uppercase; }
  .verdict-good { color: var(--ok); background: rgba(62,207,142,0.1); }
  .verdict-ok { color: var(--warn); background: rgba(255,201,74,0.1); }
  .verdict-poor { color: var(--err); background: rgba(255,106,106,0.1); }
  .verdict-na { color: var(--fg-dim); background: var(--panel-2); }
  .miss-block { margin-top: 8px; }
  .miss-block h4 { margin: 4px 0; font-size: 11px; text-transform: uppercase; color: var(--fg-dim); font-weight: 600; }
  .miss-block ul { margin: 0; padding-left: 16px; font-size: 12px; }
  .rationale { font-size: 12px; color: var(--fg); margin-top: 8px; padding: 6px 8px; background: var(--panel-2); border-radius: 4px; white-space: pre-wrap; }
  .thread-info { margin-top: 8px; padding: 6px 8px; background: rgba(122,162,255,0.08); border-left: 2px solid var(--accent); font-size: 11px; color: var(--fg-dim); border-radius: 0 4px 4px 0; }
  .thread-info .tag { font-weight: 600; color: var(--accent); margin-right: 4px; }
  .judge-error { color: var(--err); font-size: 11px; padding: 4px 6px; background: rgba(255,106,106,0.08); border-radius: 4px; }
</style>
</head>
<body>
<header>
  <h1>Extraction eval — {{ summary.kind_upper }}</h1>
  <div class="meta">
    <span><strong>account</strong> {{ summary.account }}</span>
    <span><strong>model</strong> {{ summary.model }}</span>
    <span><strong>generated</strong> {{ summary.generated_at }}</span>
  </div>
  <div class="stats">
    <span class="pill">{{ summary.total }} total</span>
    <span class="pill ok">{{ summary.ok_count }} ok</span>
    <span class="pill skipped">{{ summary.skipped_count }} skipped</span>
    <span class="pill error">{{ summary.error_count }} errors</span>
    {% if summary.mean_score %}<span class="pill">mean judge score {{ summary.mean_score | round(method="common", precision=2) }}</span>{% endif %}
    {% if summary.judge_enabled %}<span class="pill">judge: {{ summary.judge_model }}</span>{% else %}<span class="pill">judge disabled</span>{% endif %}
  </div>
</header>
<main>
{% for c in cases %}
  <section class="case">
    <div class="case-header">
      <span class="idx">#{{ c.index }}</span>
      <span>{{ c.email_id }}</span>
      <span class="status {{ c.status_class }}">{{ c.status_label }}</span>
      {% if c.extract_ms > 0 %}<span>{{ c.extract_ms }} ms</span>{% endif %}
    </div>
    {% if c.error %}
      <div class="col"><div class="judge-error">{{ c.error }}</div></div>
    {% else %}
    <div class="cols">
      {# LEFT: email #}
      <div class="col">
        <h3>Email</h3>
        <div class="email-subject">{{ c.subject }}</div>
        <div class="email-from">{{ c.sender }} &lt;{{ c.sender_email }}&gt; · {{ c.date }}</div>
        <div class="email-body">{{ c.body_excerpt }}</div>
      </div>
      {# MIDDLE: extracted items #}
      <div class="col">
        <h3>Extracted {{ summary.kind }}</h3>
        {% if summary.kind == "tasks" %}
          {% if c.tasks and c.tasks | length > 0 %}
            <ul class="items">
              {% for t in c.tasks %}
                <li>
                  <div class="item-title">{{ t.title }}</div>
                  {% if t.detail %}<div>{{ t.detail }}</div>{% endif %}
                  <div class="item-meta">
                    {% if t.priority %}priority: {{ t.priority }}{% endif %}
                    {% if t.due_at_iso %} · due: {{ t.due_at_iso }}{% endif %}
                  </div>
                </li>
              {% endfor %}
            </ul>
          {% else %}
            <div class="empty">No tasks extracted.</div>
          {% endif %}
          {% if c.thread_summary or c.commitment or c.deadline_iso %}
          <div class="thread-info">
            {% if c.thread_summary %}<div><span class="tag">summary</span>{{ c.thread_summary }}</div>{% endif %}
            {% if c.commitment %}<div><span class="tag">commitment</span>{{ c.commitment }}</div>{% endif %}
            {% if c.deadline_iso %}<div><span class="tag">deadline</span>{{ c.deadline_iso }}</div>{% endif %}
          </div>
          {% endif %}
        {% else %}
          {% if c.facts and c.facts | length > 0 %}
            <ul class="items">
              {% for f in c.facts %}
                <li>
                  <div class="item-title">{{ f.fact }}</div>
                  <div class="item-meta">
                    {{ f.subject_kind }} · {{ f.subject_key }}
                    {% if f.confidence %} · conf {{ f.confidence | round(method="common", precision=2) }}{% endif %}
                  </div>
                </li>
              {% endfor %}
            </ul>
          {% else %}
            <div class="empty">No facts extracted.</div>
          {% endif %}
        {% endif %}
      </div>
      {# RIGHT: judge verdict #}
      <div class="col">
        <h3>Judge verdict</h3>
        {% if c.verdict %}
          {% if c.verdict.error %}
            <div class="judge-error">{{ c.verdict.error }}</div>
          {% else %}
            <div class="score-line">
              <span class="score-pct {{ c.verdict.score_class }}">{% if c.verdict.score_pct %}{{ c.verdict.score_pct }}%{% else %}—{% endif %}</span>
              <span class="verdict-label {{ c.verdict.verdict_class }}">{{ c.verdict.verdict_label }}</span>
            </div>
            {% if c.verdict.missed and c.verdict.missed | length > 0 %}
            <div class="miss-block">
              <h4>Missed</h4>
              <ul>{% for m in c.verdict.missed %}<li>{{ m }}</li>{% endfor %}</ul>
            </div>
            {% endif %}
            {% if c.verdict.spurious and c.verdict.spurious | length > 0 %}
            <div class="miss-block">
              <h4>Spurious</h4>
              <ul>{% for m in c.verdict.spurious %}<li>{{ m }}</li>{% endfor %}</ul>
            </div>
            {% endif %}
            {% if c.verdict.rationale %}
            <div class="rationale">{{ c.verdict.rationale }}</div>
            {% endif %}
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
