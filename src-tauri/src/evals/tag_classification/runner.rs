// Tag-classification runner.
//
// One completion per case per repeat, straight through
// `services::classification::classify_with_provider` — the same function the
// app calls after a sync, with the rule engine left out so the number on the
// report is the model's. Cases are synthetic, so no mailbox is read; the DB
// is only there to resolve the provider, the model and the live
// `classify.email` template.

use std::path::PathBuf;

use serde::Serialize;

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::json_report::{ItemResult, JsonRunReport};
use crate::evals::shared::percentile;
use crate::evals::tag_classification::case_loader::{load_tag_cases, TagCase};
use crate::evals::tag_classification::metrics::{score_field, FieldOutcome, FieldScore};
use crate::evals::tag_classification::report;
use crate::evals::{EvalError, EvalResult};
use crate::services::classification::{
    build_classify_prompt, classify_with_provider, ClassificationConfig, ClassifyMethod, EmailToClassify, Repair,
    ReplyError,
};

/// Pinned so the `{{today}}` line in the prompt — and therefore the cached
/// prefix — is identical from run to run.
pub const EVAL_TODAY: &str = "2026-06-15";

/// How the classifier is asked for its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeMode {
    /// The shipped path: the model writes a JSON object.
    Json,
}

impl DecodeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DecodeMode::Json => "json",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TagRunnerConfig {
    pub only_case: Option<String>,
    pub only_lang: Option<String>,
    pub model_override: Option<String>,
    pub out_dir: PathBuf,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
    pub mode: DecodeMode,
    /// Repeats per case. Labels come from the first pass; every pass feeds
    /// the latency percentiles.
    pub repeats: usize,
    /// Print the machine-readable summary to stdout instead of prose.
    pub json_stdout: bool,
}

/// What one case produced.
pub struct CaseRun {
    pub case: TagCase,
    pub intent: Option<String>,
    pub topic: Option<String>,
    pub urgency: Option<String>,
    pub failure: Option<String>,
    pub repaired: bool,
    pub fell_back: bool,
    pub latencies_ms: Vec<u64>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub prefill_ms: Option<i64>,
    pub cached_prompt_tokens: Option<u32>,
}

impl CaseRun {
    pub fn passed(&self) -> bool {
        self.failure.is_none()
            && self.axis_accepted(&self.case.expect.intent, self.intent.as_deref())
            && self.axis_accepted(&self.case.expect.topic, self.topic.as_deref())
            && self.axis_accepted(&self.case.expect.urgency, self.urgency.as_deref())
    }

    fn axis_accepted(&self, accepted: &[String], predicted: Option<&str>) -> bool {
        match predicted {
            Some(p) => accepted.iter().any(|a| a == p),
            None => false,
        }
    }
}

/// The whole run, as written to `<run_id>_metrics.json`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagMetricsReport {
    pub run_id: String,
    pub model: String,
    pub mode: String,
    pub repeats: usize,
    pub total_cases: usize,
    /// Cases whose call failed outright (provider error, empty or
    /// unparseable reply) — they count as misses on every axis.
    pub failed_calls: usize,
    pub unparseable_replies: usize,
    /// Cases where a label had to be repaired onto the taxonomy by substring
    /// match, and where nothing matched and a default was substituted.
    pub repaired_cases: usize,
    pub fallback_cases: usize,
    pub intent: FieldScore,
    pub topic: FieldScore,
    pub urgency: FieldScore,
    /// All three axes right (accepted set) on the same email.
    pub all_axes_accepted: usize,
    pub latency_ms_mean: Option<f64>,
    pub latency_ms_p50: Option<u64>,
    pub latency_ms_p95: Option<u64>,
    pub prefill_ms_mean: Option<f64>,
    pub prompt_tokens_mean: Option<f64>,
    pub completion_tokens_mean: Option<f64>,
    pub cached_prompt_tokens_mean: Option<f64>,
    /// Throughput implied by the mean latency, which is what a backfill of
    /// thousands of emails actually waits on.
    pub emails_per_minute: Option<f64>,
}

pub struct TagEvalSummary {
    pub metrics: TagMetricsReport,
    pub report_path: PathBuf,
    pub metrics_path: PathBuf,
    pub html_path: PathBuf,
}

pub async fn run(cfg: TagRunnerConfig) -> EvalResult<TagEvalSummary> {
    // Same isolation as the other harnesses: work on a copy, never the live DB.
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "tag-classification")?;
    let db = std::sync::Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    if let Some(model) = &cfg.model_override {
        db.set_preference("ai_model", model)?;
    }

    // The built-in taxonomy, not the one stored in this DB: the corpus is
    // labelled against what the product ships, and a stale database would
    // otherwise reject every case using a tag added since it was seeded.
    let config = ClassificationConfig::built_in();
    let mut cases = load_tag_cases(&cfg.cases_dir, &config.intents, &config.topics)?;
    if let Some(lang) = &cfg.only_lang {
        cases.retain(|c| &c.lang == lang);
    }
    if let Some(id) = &cfg.only_case {
        cases.retain(|c| &c.id == id);
    }
    if cases.is_empty() {
        return Err(EvalError::Config(
            "no classification cases left after filtering".to_string(),
        ));
    }

    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;
    let template = crate::services::prompts::get_template(&db, "classify.email")
        .map_err(|e| EvalError::Config(format!("cannot load classify.email: {e}")))?;
    let language = crate::services::i18n::resolve_ai_language(&db)
        .map_err(|e| EvalError::Config(format!("cannot resolve AI language: {e}")))?;
    let language_clause = format!("Respond in {}.\n", language.english_name());
    let model = db.get_preference("ai_model")?.unwrap_or_default();
    let repeats = cfg.repeats.max(1);

    if !cfg.json_stdout {
        println!("[tag-eval] model = {model}");
        println!("[tag-eval] mode = {}, repeats = {repeats}", cfg.mode.as_str());
        println!("[tag-eval] running {} case(s)", cases.len());
    }

    // One throwaway call so the first measured case doesn't carry model load
    // and warm-up in its latency.
    if let Some(first) = cases.first() {
        let warm = build_classify_prompt(&template, &config, &language_clause, EVAL_TODAY, &email_of(first));
        let _ = classify_with_provider(provider.as_ref(), &config, &warm.full()).await;
    }

    let mut runs: Vec<CaseRun> = Vec::with_capacity(cases.len());
    for case in cases {
        let prompt = build_classify_prompt(&template, &config, &language_clause, EVAL_TODAY, &email_of(&case));
        let full = prompt.full();

        let mut latencies = Vec::with_capacity(repeats);
        let mut decided: Option<CaseRun> = None;
        let mut failure: Option<String> = None;

        for _ in 0..repeats {
            match classify_with_provider(provider.as_ref(), &config, &full).await {
                Ok(run) => {
                    latencies.push(run.latency_ms);
                    if decided.is_none() {
                        decided = Some(CaseRun {
                            case: case.clone(),
                            intent: Some(run.classified.intent.clone()),
                            topic: Some(run.classified.topic.clone()),
                            urgency: Some(run.classified.urgency.clone()),
                            failure: None,
                            repaired: [run.repairs.intent, run.repairs.topic, run.repairs.urgency]
                                .contains(&Repair::Matched),
                            fell_back: [run.repairs.intent, run.repairs.topic, run.repairs.urgency]
                                .contains(&Repair::Fallback),
                            latencies_ms: Vec::new(),
                            prompt_tokens: run.prompt_tokens,
                            completion_tokens: run.completion_tokens,
                            prefill_ms: run.prefill_ms,
                            cached_prompt_tokens: run.cached_prompt_tokens,
                        });
                    }
                    debug_assert_eq!(run.classified.method, ClassifyMethod::LlmJson);
                }
                Err(e) => {
                    if failure.is_none() {
                        failure = Some(describe(&e));
                    }
                }
            }
        }

        let mut run = decided.unwrap_or_else(|| CaseRun {
            case: case.clone(),
            intent: None,
            topic: None,
            urgency: None,
            failure: failure.clone(),
            repaired: false,
            fell_back: false,
            latencies_ms: Vec::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
            prefill_ms: None,
            cached_prompt_tokens: None,
        });
        run.latencies_ms = latencies;
        if run.intent.is_none() {
            run.failure = failure;
        }

        if !cfg.json_stdout {
            println!(
                "[tag-eval] {} {} → {}/{}/{}",
                if run.passed() { "OK  " } else { "FAIL" },
                run.case.id,
                run.intent.as_deref().unwrap_or("-"),
                run.topic.as_deref().unwrap_or("-"),
                run.urgency.as_deref().unwrap_or("-"),
            );
        }
        runs.push(run);
    }

    let mut metrics = summarise(&runs, &model, cfg.mode, repeats);

    let mut json = JsonRunReport::new("tag_classification", &model);
    metrics.run_id = json.run_id.clone();
    for run in &runs {
        json.push(ItemResult {
            id: run.case.id.clone(),
            passed: run.passed(),
            score: Some(if run.passed() { 1.0 } else { 0.0 }),
            detail: match &run.failure {
                Some(err) => format!("call failed: {err}"),
                None => format!(
                    "{}/{}/{}",
                    run.intent.as_deref().unwrap_or("-"),
                    run.topic.as_deref().unwrap_or("-"),
                    run.urgency.as_deref().unwrap_or("-")
                ),
            },
            evidence: None,
        });
    }

    let report_path = json.write(&cfg.out_dir)?;
    std::fs::create_dir_all(&cfg.out_dir)?;
    let metrics_path = cfg.out_dir.join(format!("{}_metrics.json", metrics.run_id));
    std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics)?)?;
    let html_path = report::render(&cfg.out_dir, &metrics, &runs)?;

    if cfg.json_stdout {
        println!("{}", serde_json::to_string_pretty(&metrics)?);
    } else {
        print_summary(&metrics);
        println!("[tag-eval] report written to {}", html_path.display());
    }

    Ok(TagEvalSummary {
        metrics,
        report_path,
        metrics_path,
        html_path,
    })
}

fn email_of(case: &TagCase) -> EmailToClassify<'_> {
    EmailToClassify {
        sender: &case.from_name,
        sender_email: &case.from_email,
        subject: &case.subject,
        snippet: &case.snippet,
    }
}

fn describe(err: &ReplyError) -> String {
    match err {
        ReplyError::Provider(e) => format!("provider: {e}"),
        ReplyError::Empty => "empty reply".to_string(),
        ReplyError::Unparseable(detail) => format!("unparseable: {detail}"),
    }
}

fn summarise(runs: &[CaseRun], model: &str, mode: DecodeMode, repeats: usize) -> TagMetricsReport {
    let outcomes = |pick: fn(&CaseRun) -> (&Vec<String>, Option<&str>)| -> Vec<FieldOutcome> {
        runs.iter()
            .map(|r| {
                let (accepted, predicted) = pick(r);
                FieldOutcome {
                    gold: TagCase::gold(accepted).to_string(),
                    accepted: accepted.clone(),
                    predicted: predicted.map(str::to_string),
                }
            })
            .collect()
    };

    let intent = score_field(&outcomes(|r| (&r.case.expect.intent, r.intent.as_deref())));
    let topic = score_field(&outcomes(|r| (&r.case.expect.topic, r.topic.as_deref())));
    let urgency = score_field(&outcomes(|r| (&r.case.expect.urgency, r.urgency.as_deref())));

    let mut latencies: Vec<u64> = runs.iter().flat_map(|r| r.latencies_ms.iter().copied()).collect();
    latencies.sort_unstable();
    let latency_ms_mean = mean(latencies.iter().map(|v| *v as f64));

    TagMetricsReport {
        run_id: String::new(),
        model: model.to_string(),
        mode: mode.as_str().to_string(),
        repeats,
        total_cases: runs.len(),
        failed_calls: runs.iter().filter(|r| r.failure.is_some()).count(),
        unparseable_replies: runs
            .iter()
            .filter(|r| r.failure.as_deref().is_some_and(|f| f.starts_with("unparseable")))
            .count(),
        repaired_cases: runs.iter().filter(|r| r.repaired).count(),
        fallback_cases: runs.iter().filter(|r| r.fell_back).count(),
        all_axes_accepted: runs.iter().filter(|r| r.passed()).count(),
        intent,
        topic,
        urgency,
        latency_ms_mean,
        latency_ms_p50: percentile(&latencies, 0.5),
        latency_ms_p95: percentile(&latencies, 0.95),
        prefill_ms_mean: mean(runs.iter().filter_map(|r| r.prefill_ms).map(|v| v as f64)),
        prompt_tokens_mean: mean(
            runs.iter()
                .filter(|r| r.failure.is_none())
                .map(|r| r.prompt_tokens as f64),
        ),
        completion_tokens_mean: mean(
            runs.iter()
                .filter(|r| r.failure.is_none())
                .map(|r| r.completion_tokens as f64),
        ),
        cached_prompt_tokens_mean: mean(runs.iter().filter_map(|r| r.cached_prompt_tokens).map(|v| v as f64)),
        emails_per_minute: latency_ms_mean.filter(|m| *m > 0.0).map(|m| 60_000.0 / m),
    }
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut count = 0usize;
    let mut total = 0.0;
    for v in values {
        count += 1;
        total += v;
    }
    (count > 0).then(|| total / count as f64)
}

fn print_summary(m: &TagMetricsReport) {
    let pct = |v: Option<f64>| v.map(|x| format!("{:.1}%", x * 100.0)).unwrap_or_else(|| "n/a".into());
    println!("[tag-eval] ── {} cases, model {} ──", m.total_cases, m.model);
    for (name, score) in [("intent", &m.intent), ("topic", &m.topic), ("urgency", &m.urgency)] {
        println!(
            "[tag-eval] {name:<8} strict {} accepted {} macro-F1 {}",
            pct(score.strict_accuracy()),
            pct(score.accepted_accuracy()),
            score
                .macro_f1
                .map(|f| format!("{f:.3}"))
                .unwrap_or_else(|| "n/a".into()),
        );
    }
    println!(
        "[tag-eval] all three axes accepted on {}/{} emails",
        m.all_axes_accepted, m.total_cases
    );
    println!(
        "[tag-eval] repaired {} · fell back {} · failed calls {} (unparseable {})",
        m.repaired_cases, m.fallback_cases, m.failed_calls, m.unparseable_replies
    );
    println!(
        "[tag-eval] latency mean {} p50 {} p95 {} ms · prefill mean {} ms · {} emails/min",
        m.latency_ms_mean
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "n/a".into()),
        m.latency_ms_p50.map(|v| v.to_string()).unwrap_or_else(|| "n/a".into()),
        m.latency_ms_p95.map(|v| v.to_string()).unwrap_or_else(|| "n/a".into()),
        m.prefill_ms_mean
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "n/a".into()),
        m.emails_per_minute
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "n/a".into()),
    );
}
