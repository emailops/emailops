// Self-contained HTML report for the agent-search eval.
//
// Layout per case:
//   - Question + criteria + tags + pool stats.
//   - Side-by-side summary table: P@K / R@K / F1@K / MRR / latency for each mode.
//   - Per-mode collapsible block listing the top-K hits, each tagged with the
//     judge's score (0/1/2) and rationale. Easy to scan for "where did mode X
//     go wrong?" cases.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::evals::agent_search::runner::CaseOutcome;
use crate::evals::EvalResult;

fn encode_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn render(
    out_dir: &Path,
    cases: &[CaseOutcome],
    judge_enabled: bool,
    judge_model: &str,
    top_k: usize,
) -> EvalResult<PathBuf> {
    fs::create_dir_all(out_dir)?;
    let ts = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let path = out_dir.join(format!("agent_search_eval_{}.html", ts));

    let mut html = String::new();
    html.push_str("<!DOCTYPE html><html><head><meta charset='utf-8'>");
    html.push_str(&format!("<title>Agent Search Eval — {}</title>", encode_text(&ts)));
    html.push_str("<style>");
    html.push_str(CSS);
    html.push_str("</style></head><body>");

    html.push_str("<h1>Agent Search Evaluation Report</h1>");
    html.push_str(&format!(
        "<p class='meta'>Generated {} • Judge: {} {} • Top-K: {} • {} case(s)</p>",
        encode_text(&Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        if judge_enabled { "enabled" } else { "disabled" },
        encode_text(judge_model),
        top_k,
        cases.len()
    ));

    // ── Aggregate table ──────────────────────────────────────────────────────
    html.push_str("<h2>Aggregate (mean across cases)</h2>");
    html.push_str(&render_aggregate_table(cases, top_k));

    // ── Per-case detail ──────────────────────────────────────────────────────
    for (idx, co) in cases.iter().enumerate() {
        html.push_str("<div class='case'>");
        html.push_str(&format!(
            "<h2>Case {}: <code>{}</code></h2>",
            idx + 1,
            encode_text(&co.case.id)
        ));
        html.push_str(&format!("<p class='question'>{}</p>", encode_text(&co.case.question)));
        if !co.case.tags.is_empty() {
            html.push_str("<div class='tags'>");
            for t in &co.case.tags {
                html.push_str(&format!("<span class='tag'>{}</span>", encode_text(t)));
            }
            html.push_str("</div>");
        }
        html.push_str("<details class='criteria'><summary>Rubric</summary>");
        html.push_str(&format!("<pre>{}</pre>", encode_text(&co.case.judge_criteria)));
        html.push_str("</details>");

        // Per-case summary table.
        html.push_str(&render_case_summary_table(co, top_k));

        // Per-mode detail.
        for mo in &co.mode_outcomes {
            html.push_str("<details class='mode'>");
            html.push_str(&format!(
                "<summary>Mode: <strong>{}</strong> — {} hits • {} ms • P@{}={:.0}% R@{}={:.0}% F1={:.0}% MRR={:.2}{}</summary>",
                encode_text(mo.mode.as_str()),
                mo.hits.len(),
                mo.elapsed_ms,
                top_k,
                mo.metrics.precision_at_k * 100.0,
                top_k,
                mo.metrics.recall_at_k * 100.0,
                mo.metrics.f1_at_k * 100.0,
                mo.metrics.mrr,
                mo.error.as_deref().map(|e| format!(" • <span class='err'>ERR: {}</span>", encode_text(e))).unwrap_or_default(),
            ));

            if let Some(qp) = &mo.query_plan_raw {
                html.push_str(&format!(
                    "<details class='plan'><summary>Query plan</summary><pre>{}</pre></details>",
                    encode_text(qp)
                ));
            }
            if !mo.stage_counts.is_empty() {
                html.push_str("<p class='stages'>Stages: ");
                let mut parts: Vec<String> = mo.stage_counts.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                parts.sort();
                html.push_str(&encode_text(&parts.join(" • ")));
                html.push_str("</p>");
            }

            html.push_str("<ol class='hits'>");
            for (rank, h) in mo.hits.iter().enumerate() {
                let judgment = co.judgments.get(&h.email_id);
                let badge = match judgment.map(|j| j.score) {
                    Some(2) => "<span class='judge j2'>2 • clearly relevant</span>",
                    Some(1) => "<span class='judge j1'>1 • partial</span>",
                    Some(0) => "<span class='judge j0'>0 • irrelevant</span>",
                    _ => "<span class='judge jn'>—</span>",
                };
                let err = judgment
                    .and_then(|j| j.error.as_deref())
                    .map(|e| format!("<span class='err'>(judge err: {})</span>", encode_text(e)))
                    .unwrap_or_default();
                let dir = if h.sent_by_user { "SENT" } else { "RECV" };
                html.push_str("<li>");
                html.push_str(&format!(
                    "<div class='hit-head'><span class='rank'>#{}</span> {badge} {err} <span class='dir d-{lc}'>{dir}</span> <strong>{subject}</strong></div>",
                    rank + 1,
                    badge = badge,
                    err = err,
                    lc = dir.to_ascii_lowercase(),
                    dir = dir,
                    subject = encode_text(&h.subject),
                ));
                html.push_str(&format!(
                    "<div class='hit-meta'>from {sender} &lt;{sender_email}&gt; • score={:.3} • reason: {reason}</div>",
                    h.score,
                    sender = encode_text(&h.sender),
                    sender_email = encode_text(&h.sender_email),
                    reason = encode_text(&h.reason),
                ));
                if let Some(j) = judgment {
                    if !j.rationale.is_empty() {
                        html.push_str(&format!(
                            "<div class='rationale'>judge: {}</div>",
                            encode_text(&j.rationale)
                        ));
                    }
                }
                html.push_str(&format!(
                    "<details class='snippet'><summary>Snippet</summary><pre>{}</pre></details>",
                    encode_text(&h.snippet)
                ));
                html.push_str("</li>");
            }
            html.push_str("</ol>");

            // List of relevant emails the mode MISSED (in pool but not in this
            // mode's top-K). Useful for diagnosing recall failures.
            let returned: std::collections::HashSet<&str> =
                mo.hits.iter().take(top_k).map(|h| h.email_id.as_str()).collect();
            let missed: Vec<(&String, &crate::evals::agent_search::judge::Judgment)> = co
                .judgments
                .iter()
                .filter(|(eid, j)| j.is_clearly_relevant() && !returned.contains(eid.as_str()))
                .collect();
            if !missed.is_empty() {
                html.push_str("<details class='missed'><summary>Missed clearly-relevant emails (in pool, not in this mode's top-K)</summary><ul>");
                for (eid, j) in missed {
                    html.push_str(&format!(
                        "<li><code>{}</code> — {}</li>",
                        encode_text(eid),
                        encode_text(&j.rationale)
                    ));
                }
                html.push_str("</ul></details>");
            }

            html.push_str("</details>");
        }

        html.push_str("</div>");
    }

    html.push_str("</body></html>");
    fs::write(&path, html)?;
    Ok(path)
}

fn render_aggregate_table(cases: &[CaseOutcome], top_k: usize) -> String {
    use std::collections::BTreeMap;
    let mut sums: BTreeMap<String, (f32, f32, f32, f32, i64, usize)> = BTreeMap::new();
    for co in cases {
        for mo in &co.mode_outcomes {
            let key = mo.mode.as_str().to_string();
            let e = sums.entry(key).or_insert((0.0, 0.0, 0.0, 0.0, 0, 0));
            e.0 += mo.metrics.precision_at_k;
            e.1 += mo.metrics.recall_at_k;
            e.2 += mo.metrics.f1_at_k;
            e.3 += mo.metrics.mrr;
            e.4 += mo.elapsed_ms;
            e.5 += 1;
        }
    }
    let mut s = String::new();
    s.push_str("<table class='summary'><thead><tr>");
    s.push_str(&format!(
        "<th>Mode</th><th>P@{k}</th><th>R@{k}</th><th>F1@{k}</th><th>MRR</th><th>Avg latency</th>",
        k = top_k
    ));
    s.push_str("</tr></thead><tbody>");
    for (mode, v) in sums {
        let n = v.5.max(1) as f32;
        s.push_str(&format!(
            "<tr><td><strong>{}</strong></td><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td><td>{} ms</td></tr>",
            mode,
            score_cell(v.0 / n),
            score_cell(v.1 / n),
            score_cell(v.2 / n),
            v.3 / n,
            (v.4 / v.5.max(1) as i64),
        ));
    }
    s.push_str("</tbody></table>");
    s
}

fn render_case_summary_table(co: &CaseOutcome, top_k: usize) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "<p class='pool'>Pool: {} unique emails • clearly relevant in pool: {} • partial+: {}</p>",
        co.mode_outcomes.first().map(|m| m.metrics.pool_size).unwrap_or(0),
        co.judgments.values().filter(|j| j.is_clearly_relevant()).count(),
        co.judgments.values().filter(|j| j.is_relevant()).count(),
    ));
    s.push_str("<table class='summary'><thead><tr>");
    s.push_str(&format!(
        "<th>Mode</th><th>P@{k}</th><th>R@{k}</th><th>F1@{k}</th><th>MRR</th><th>Relevant in top-{k}</th><th>Latency</th>",
        k = top_k
    ));
    s.push_str("</tr></thead><tbody>");
    for mo in &co.mode_outcomes {
        s.push_str(&format!(
            "<tr><td><strong>{}</strong></td><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td><td>{}</td><td>{} ms</td></tr>",
            mo.mode.as_str(),
            score_cell(mo.metrics.precision_at_k),
            score_cell(mo.metrics.recall_at_k),
            score_cell(mo.metrics.f1_at_k),
            mo.metrics.mrr,
            mo.metrics.relevant_in_top_k,
            mo.elapsed_ms,
        ));
    }
    s.push_str("</tbody></table>");
    s
}

fn score_cell(v: f32) -> String {
    let cls = if v >= 0.8 {
        "score-good"
    } else if v >= 0.5 {
        "score-mid"
    } else {
        "score-bad"
    };
    format!("<span class='{}'>{:.0}%</span>", cls, v * 100.0)
}

const CSS: &str = r#"
body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; max-width: 1400px; margin: 0 auto; padding: 20px; background: #f5f5f7; color: #222; }
h1 { margin: 0 0 4px; } h2 { margin: 24px 0 8px; font-size: 18px; }
.meta { color: #666; font-size: 13px; margin-bottom: 24px; }
.case { background: #fff; border-radius: 8px; box-shadow: 0 1px 4px rgba(0,0,0,0.08); padding: 18px; margin-bottom: 18px; }
.question { font-style: italic; color: #333; margin: 4px 0 8px; }
.tags { margin-bottom: 8px; }
.tag { display: inline-block; background: #e8eaf6; color: #3949ab; padding: 2px 8px; border-radius: 4px; font-size: 11px; margin-right: 4px; }
.criteria pre { background: #f9f9fb; border: 1px solid #eee; padding: 8px; font-size: 12px; white-space: pre-wrap; }
.summary { width: 100%; border-collapse: collapse; background: #fff; margin: 12px 0; }
.summary th, .summary td { padding: 8px 10px; border-bottom: 1px solid #eee; font-size: 13px; text-align: left; }
.summary th { background: #fafafa; font-size: 11px; text-transform: uppercase; color: #666; }
.pool { font-size: 12px; color: #666; margin: 6px 0; }
.mode { margin-top: 14px; border-left: 3px solid #5c6bc0; padding-left: 10px; }
.mode > summary { font-weight: 500; padding: 6px 0; cursor: pointer; font-size: 13px; }
.plan pre, .snippet pre { font-size: 11px; background: #fafafa; padding: 6px; white-space: pre-wrap; max-height: 200px; overflow: auto; }
.stages { font-size: 11px; color: #888; margin: 4px 0 8px; }
.hits { padding-left: 22px; }
.hits li { margin: 8px 0; padding: 6px 8px; background: #fafbfd; border-left: 2px solid #d0d7e6; }
.hit-head { font-size: 13px; }
.hit-meta { font-size: 11px; color: #666; }
.rationale { font-size: 11px; color: #555; margin-top: 4px; font-style: italic; }
.rank { font-weight: 600; color: #5c6bc0; margin-right: 4px; }
.judge { display: inline-block; padding: 1px 6px; border-radius: 3px; font-size: 11px; font-weight: 600; margin-right: 4px; }
.j2 { background: #c8e6c9; color: #1b5e20; }
.j1 { background: #fff9c4; color: #6d4c00; }
.j0 { background: #ffcdd2; color: #8e0000; }
.jn { background: #eee; color: #888; }
.dir { font-size: 10px; padding: 1px 4px; border-radius: 3px; margin-right: 4px; }
.d-sent { background: #e3f2fd; color: #0d47a1; }
.d-recv { background: #fce4ec; color: #880e4f; }
.err { color: #c62828; font-size: 11px; }
.score-good { color: #1b5e20; font-weight: 600; }
.score-mid { color: #ef6c00; font-weight: 600; }
.score-bad { color: #c62828; font-weight: 600; }
.missed { margin-top: 8px; font-size: 12px; color: #555; }
.missed ul { padding-left: 22px; }
"#;
