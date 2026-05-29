// Side-by-side HTML report for the shortcut-variant harness.
//
// One card per shortcut, with a horizontal list of variant columns inside so
// you can eyeball structure/faithfulness/usefulness/tone numbers across
// variants at a glance. Highlights the winning variant per shortcut.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tera::{Context, Tera};

use crate::evals::harness::CaseOutcome;
use crate::evals::shortcuts::case_loader::{ShortcutCase, ShortcutVariant};
use crate::evals::shortcuts::judge::VariantScores;
use crate::evals::shortcuts::metrics::RubricReport;
use crate::evals::EvalResult;

pub struct ReportShortcut {
    pub case: ShortcutCase,
    pub variants: Vec<ReportVariant>,
}

pub struct ReportVariant {
    pub variant: ShortcutVariant,
    pub outcome: Option<CaseOutcome>,
    pub rubric: RubricReport,
    pub scores: VariantScores,
    pub error: Option<String>,
}

#[derive(Serialize)]
struct VariantView {
    id: String,
    description: String,
    prompt: String,
    answer: String,
    rubric_passed: usize,
    rubric_total: usize,
    rubric_all_passed: bool,
    rubric_checks: Vec<CheckView>,
    latency_ms: i64,
    wall_elapsed_ms: i64,
    token_count: Option<i32>,
    score_structure: Option<ScoreView>,
    score_faithfulness: Option<ScoreView>,
    score_usefulness: Option<ScoreView>,
    score_tone: Option<ScoreView>,
    composite: Option<ScoreView>,
    rationale: Option<String>,
    judge_error: Option<String>,
    harness_error: Option<String>,
    is_winner: bool,
}

#[derive(Serialize, Clone)]
struct ScoreView {
    pct: i32,
    label: String,
    class: String,
}

#[derive(Serialize)]
struct CheckView {
    name: String,
    passed: bool,
    detail: String,
}

#[derive(Serialize)]
struct ShortcutView {
    shortcut_id: String,
    label: String,
    account: String,
    model: String,
    rubric_language: String,
    rubric_description: String,
    variants: Vec<VariantView>,
    winner_id: Option<String>,
}

#[derive(Serialize)]
struct SummaryView {
    generated_at: String,
    total_shortcuts: usize,
    total_variants: usize,
    judge_enabled: bool,
    judge_model: String,
}

pub fn render(
    out_dir: &Path,
    shortcuts: &[ReportShortcut],
    judge_enabled: bool,
    judge_model: &str,
) -> EvalResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let path = out_dir.join(format!("shortcut_report_{}.html", stamp));

    let mut tera = Tera::default();
    tera.add_raw_template("report", REPORT_TEMPLATE)?;

    let mut total_variants = 0usize;
    let mut shortcut_views: Vec<ShortcutView> = Vec::with_capacity(shortcuts.len());

    for sc in shortcuts {
        total_variants += sc.variants.len();

        // Compute composites and pick a winner. Tiebreaker: rubric pass count,
        // then lower latency.
        let mut composites: Vec<(usize, Option<f64>)> = sc
            .variants
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.scores.composite()))
            .collect();
        composites.sort_by(|a, b| {
            b.1.unwrap_or(-1.0)
                .partial_cmp(&a.1.unwrap_or(-1.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let winner_idx: Option<usize> = composites.first().and_then(|(i, v)| v.map(|_| *i));

        let winner_id: Option<String> = winner_idx.map(|i| sc.variants[i].variant.id.clone());

        let rubric_desc = describe_rubric(&sc.case);

        let variants: Vec<VariantView> = sc
            .variants
            .iter()
            .enumerate()
            .map(|(i, rv)| {
                let (answer, latency_ms, wall_elapsed_ms, token_count) = match &rv.outcome {
                    Some(o) => (
                        o.assistant_content.clone(),
                        o.assistant_latency_ms.unwrap_or(0),
                        o.wall_elapsed_ms,
                        o.assistant_token_count,
                    ),
                    None => ("(no answer — harness error)".into(), 0, 0, None),
                };

                VariantView {
                    id: rv.variant.id.clone(),
                    description: rv.variant.description.clone(),
                    prompt: rv.variant.prompt.clone(),
                    answer,
                    rubric_passed: rv.rubric.passed_count(),
                    rubric_total: rv.rubric.total(),
                    rubric_all_passed: rv.rubric.all_passed() && rv.rubric.total() > 0,
                    rubric_checks: rv
                        .rubric
                        .checks
                        .iter()
                        .map(|c| CheckView {
                            name: c.name.clone(),
                            passed: c.passed,
                            detail: c.detail.clone(),
                        })
                        .collect(),
                    latency_ms,
                    wall_elapsed_ms,
                    token_count,
                    score_structure: to_score(rv.scores.structure),
                    score_faithfulness: to_score(rv.scores.faithfulness),
                    score_usefulness: to_score(rv.scores.usefulness),
                    score_tone: to_score(rv.scores.tone),
                    composite: to_score(rv.scores.composite()),
                    rationale: rv.scores.rationale.clone(),
                    judge_error: rv.scores.error.clone(),
                    harness_error: rv.error.clone(),
                    is_winner: Some(i) == winner_idx,
                }
            })
            .collect();

        shortcut_views.push(ShortcutView {
            shortcut_id: sc.case.shortcut_id.clone(),
            label: sc.case.label.clone(),
            account: sc.case.account.clone(),
            model: sc.case.model.clone(),
            rubric_language: sc.case.rubric.language.clone(),
            rubric_description: rubric_desc,
            variants,
            winner_id,
        });
    }

    let summary = SummaryView {
        generated_at: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        total_shortcuts: shortcuts.len(),
        total_variants,
        judge_enabled,
        judge_model: judge_model.to_string(),
    };

    let mut ctx = Context::new();
    ctx.insert("summary", &summary);
    ctx.insert("shortcuts", &shortcut_views);
    let html = tera.render("report", &ctx)?;
    std::fs::write(&path, html)?;
    Ok(path)
}

fn describe_rubric(case: &ShortcutCase) -> String {
    let mut parts: Vec<String> = Vec::new();
    if case.rubric.must_contain_table {
        parts.push("table required".into());
    }
    if !case.rubric.required_columns.is_empty() {
        parts.push(format!("columns: {}", case.rubric.required_columns.join(" | ")));
    }
    if case.rubric.min_rows > 0 {
        parts.push(format!("≥ {} rows", case.rubric.min_rows));
    }
    if case.rubric.require_row_citations {
        parts.push("row citations".into());
    }
    if case.rubric.must_end_with_summary_paragraph {
        parts.push("summary paragraph".into());
    }
    parts.push(format!("lang={}", case.rubric.language));
    parts.join(" · ")
}

fn to_score(v: Option<f64>) -> Option<ScoreView> {
    v.map(|x| {
        let pct = (x * 100.0).round() as i32;
        let class = if pct >= 80 {
            "pass"
        } else if pct >= 60 {
            "mixed"
        } else {
            "fail"
        };
        ScoreView {
            pct,
            label: format!("{:.2}", x),
            class: class.to_string(),
        }
    })
}

const REPORT_TEMPLATE: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>EmailOps Shortcut Variants — {{ summary.generated_at }}</title>
<style>
  :root {
    --green:#22c55e; --red:#ef4444; --amber:#f59e0b; --violet:#8b5cf6;
    --bg:#0f172a; --surface:#1e293b; --surface2:#334155;
    --text:#f1f5f9; --text-muted:#94a3b8; --border:#475569;
  }
  *{box-sizing:border-box;margin:0;padding:0;}
  body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;background:var(--bg);color:var(--text);line-height:1.5;padding:2rem;max-width:1600px;margin:0 auto;}
  h1{font-size:1.75rem;margin-bottom:.2rem;}
  .subtitle{color:var(--text-muted);font-size:.875rem;margin-bottom:2rem;}

  .summary{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:1rem;margin-bottom:2rem;}
  .card{background:var(--surface);border-radius:12px;padding:1.1rem 1.25rem;border:1px solid var(--border);}
  .card-label{font-size:.7rem;text-transform:uppercase;letter-spacing:.05em;color:var(--text-muted);margin-bottom:.2rem;}
  .card-value{font-size:1.8rem;font-weight:700;}

  .sc-card{background:var(--surface);border-radius:12px;padding:1.25rem 1.5rem;border:1px solid var(--border);margin-bottom:1.25rem;}
  .sc-header{display:flex;justify-content:space-between;align-items:flex-start;gap:1rem;margin-bottom:.75rem;}
  .sc-title{font-size:1.05rem;font-weight:600;}
  .sc-meta{color:var(--text-muted);font-size:.78rem;margin-top:.2rem;}

  .variant-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(360px,1fr));gap:1rem;margin-top:.75rem;}
  .var-col{background:var(--surface2);border-radius:10px;border:1px solid var(--border);padding:.9rem 1rem;display:flex;flex-direction:column;gap:.6rem;}
  .var-col.winner{border-color:var(--violet);box-shadow:0 0 0 1px var(--violet);}
  .var-head{display:flex;justify-content:space-between;align-items:center;}
  .var-id{font-weight:600;font-size:.92rem;}
  .var-desc{color:var(--text-muted);font-size:.75rem;}
  .badge{display:inline-block;padding:.15rem .55rem;border-radius:9999px;font-size:.67rem;font-weight:600;text-transform:uppercase;}
  .badge-winner{background:rgba(139,92,246,.18);color:var(--violet);}
  .badge-pass{background:rgba(34,197,94,.15);color:var(--green);}
  .badge-fail{background:rgba(239,68,68,.15);color:var(--red);}

  .score-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:.35rem;}
  .score{background:var(--bg);border-radius:6px;padding:.35rem .5rem;border-left:3px solid var(--border);}
  .score.pass{border-left-color:var(--green);}
  .score.mixed{border-left-color:var(--amber);}
  .score.fail{border-left-color:var(--red);}
  .score-label{font-size:.66rem;color:var(--text-muted);text-transform:uppercase;letter-spacing:.03em;}
  .score-val{font-weight:700;font-size:.95rem;}
  .score-val.pass{color:var(--green);}
  .score-val.mixed{color:var(--amber);}
  .score-val.fail{color:var(--red);}

  .checks{display:flex;flex-wrap:wrap;gap:.3rem;}
  .chk{font-size:.7rem;padding:.12rem .45rem;border-radius:9999px;background:var(--bg);border:1px solid var(--border);}
  .chk.pass{color:var(--green);border-color:rgba(34,197,94,.5);}
  .chk.fail{color:var(--red);border-color:rgba(239,68,68,.5);}

  details > summary{cursor:pointer;font-size:.78rem;color:var(--text-muted);user-select:none;}
  details > pre, details > div{margin-top:.35rem;padding:.5rem .65rem;background:#0b1020;color:#d9e2f3;font-size:.76rem;border-radius:6px;white-space:pre-wrap;word-break:break-word;max-height:340px;overflow:auto;}

  .rationale{font-size:.78rem;color:var(--text-muted);font-style:italic;}
  .meta-line{display:flex;flex-wrap:wrap;gap:.65rem;color:var(--text-muted);font-size:.74rem;}
</style>
</head>
<body>
<h1>EmailOps Shortcut Variant Evaluation</h1>
<div class="subtitle">{{ summary.generated_at }} — {{ summary.total_shortcuts }} shortcut(s), {{ summary.total_variants }} variant(s) · judge: {{ summary.judge_model }}{% if not summary.judge_enabled %} (skipped){% endif %}</div>

<div class="summary">
  <div class="card"><div class="card-label">Shortcuts</div><div class="card-value">{{ summary.total_shortcuts }}</div></div>
  <div class="card"><div class="card-label">Variants</div><div class="card-value">{{ summary.total_variants }}</div></div>
</div>

{% for sc in shortcuts %}
<div class="sc-card">
  <div class="sc-header">
    <div>
      <div class="sc-title">[{{ sc.shortcut_id }}] {{ sc.label }}</div>
      <div class="sc-meta">account: {{ sc.account }} · model: {{ sc.model }} · rubric: {{ sc.rubric_description }}{% if sc.winner_id %} · winner: <strong>{{ sc.winner_id }}</strong>{% endif %}</div>
    </div>
  </div>

  <div class="variant-grid">
    {% for v in sc.variants %}
    <div class="var-col {% if v.is_winner %}winner{% endif %}">
      <div class="var-head">
        <div>
          <div class="var-id">{{ v.id }}{% if v.is_winner %} <span class="badge badge-winner">winner</span>{% endif %}</div>
          <div class="var-desc">{{ v.description }}</div>
        </div>
        <div>{% if v.rubric_all_passed %}<span class="badge badge-pass">rubric ✓</span>{% else %}<span class="badge badge-fail">rubric {{ v.rubric_passed }}/{{ v.rubric_total }}</span>{% endif %}</div>
      </div>

      <div class="score-grid">
        {% if v.score_structure %}<div class="score {{ v.score_structure.class }}"><div class="score-label">structure</div><div class="score-val {{ v.score_structure.class }}">{{ v.score_structure.label }}</div></div>{% else %}<div class="score"><div class="score-label">structure</div><div class="score-val">–</div></div>{% endif %}
        {% if v.score_faithfulness %}<div class="score {{ v.score_faithfulness.class }}"><div class="score-label">faith.</div><div class="score-val {{ v.score_faithfulness.class }}">{{ v.score_faithfulness.label }}</div></div>{% else %}<div class="score"><div class="score-label">faith.</div><div class="score-val">–</div></div>{% endif %}
        {% if v.score_usefulness %}<div class="score {{ v.score_usefulness.class }}"><div class="score-label">useful</div><div class="score-val {{ v.score_usefulness.class }}">{{ v.score_usefulness.label }}</div></div>{% else %}<div class="score"><div class="score-label">useful</div><div class="score-val">–</div></div>{% endif %}
        {% if v.score_tone %}<div class="score {{ v.score_tone.class }}"><div class="score-label">tone</div><div class="score-val {{ v.score_tone.class }}">{{ v.score_tone.label }}</div></div>{% else %}<div class="score"><div class="score-label">tone</div><div class="score-val">–</div></div>{% endif %}
      </div>

      {% if v.composite %}<div class="meta-line"><span><strong style="color:var(--text)">composite:</strong> {{ v.composite.label }}</span><span>{{ v.latency_ms }}ms{% if v.token_count %} · {{ v.token_count }} tok{% endif %}</span></div>{% else %}<div class="meta-line"><span>{{ v.latency_ms }}ms</span></div>{% endif %}

      <div class="checks">
        {% for c in v.rubric_checks %}<span class="chk {% if c.passed %}pass{% else %}fail{% endif %}" title="{{ c.detail }}">{{ c.name }}</span>{% endfor %}
      </div>

      {% if v.rationale %}<div class="rationale">“{{ v.rationale }}”</div>{% endif %}

      <details>
        <summary>prompt ({{ v.prompt | length }} chars)</summary>
        <pre>{{ v.prompt }}</pre>
      </details>
      <details>
        <summary>answer ({{ v.answer | length }} chars)</summary>
        <div>{{ v.answer }}</div>
      </details>

      {% if v.judge_error %}<div class="rationale" style="color:var(--amber)">judge: {{ v.judge_error }}</div>{% endif %}
      {% if v.harness_error %}<div class="rationale" style="color:var(--red)">harness: {{ v.harness_error }}</div>{% endif %}
    </div>
    {% endfor %}
  </div>
</div>
{% endfor %}

</body>
</html>
"###;
