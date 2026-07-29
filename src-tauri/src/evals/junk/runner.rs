//! Orchestration for the junk eval: load cases → run the pure planner → score →
//! write reports → report gate status.
//!
//! Synchronous and I/O-free apart from reading cases and writing reports:
//! `judge()` is pure, so the whole suite runs in milliseconds and is cheap
//! enough to sit in a pre-commit hook.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::services::junk::verdict::{judge, JunkAxis, Weights};

use super::super::json_report::{ItemResult, JsonRunReport};
use super::super::EvalResult;
use super::cases::{load_cases, JunkCase};
use super::metrics::{
    all_gates_passed, confusion_for, evaluate_gates, llm_routed_fraction, threshold_sweep, AxisConfusion, CaseOutcome,
    GateOutcome, Gates, SweepPoint,
};

pub struct JunkEvalConfig {
    pub cases_dir: PathBuf,
    pub out_dir: PathBuf,
    /// Run only cases in this tier.
    pub tier: Option<String>,
    /// Run only this case id.
    pub case_filter: Option<String>,
    pub gates: Gates,
}

/// Per-axis block written to the metrics report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisReport {
    pub axis: String,
    pub confusion: AxisConfusion,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub f1: Option<f64>,
    pub legit_fp_rate: Option<f64>,
    /// Precision/recall curve, so the production cutoff is chosen from data.
    pub sweep: Vec<SweepPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JunkMetricsReport {
    pub run_id: String,
    pub total_cases: usize,
    pub axes: Vec<AxisReport>,
    pub llm_routed_fraction: f64,
    pub gates: Vec<GateOutcome>,
    pub gates_passed: bool,
}

pub struct JunkEvalSummary {
    pub report_path: PathBuf,
    pub metrics_path: PathBuf,
    pub metrics: JunkMetricsReport,
}

fn score_case(case: &JunkCase, weights: &Weights) -> (CaseOutcome, Vec<String>) {
    let signals = case.to_signals();
    let verdict = judge(&signals, weights);
    let mut failures: Vec<String> = Vec::new();

    let expected = case.expect.flagged_map();
    for (axis, want_flagged) in &expected {
        let got_flagged = verdict.axis(*axis).band.is_flagged();
        if got_flagged != *want_flagged {
            failures.push(format!(
                "{}: expected {}, got band {:?}",
                axis.as_str(),
                if *want_flagged { "junk" } else { "clean" },
                verdict.axis(*axis).band
            ));
        }
    }

    let got_reasons = verdict.all_reason_codes();
    for code in &case.expect.reasons_include {
        if !got_reasons.contains(code) {
            failures.push(format!("missing reason {code:?}"));
        }
    }

    (
        CaseOutcome {
            case_id: case.id.clone(),
            expected_flagged: expected,
            verdict,
        },
        failures,
    )
}

pub fn run(cfg: JunkEvalConfig) -> EvalResult<JunkEvalSummary> {
    let weights = Weights::default();
    let mut cases = load_cases(&cfg.cases_dir)?;

    if let Some(tier) = &cfg.tier {
        cases.retain(|c| &c.tier == tier);
    }
    if let Some(id) = &cfg.case_filter {
        cases.retain(|c| &c.id == id);
    }

    let mut report = JsonRunReport::new("junk_eval", "deterministic");
    let mut outcomes: Vec<CaseOutcome> = Vec::with_capacity(cases.len());

    for case in &cases {
        let (outcome, failures) = score_case(case, &weights);
        report.push(ItemResult {
            id: case.id.clone(),
            passed: failures.is_empty(),
            score: Some(if failures.is_empty() { 1.0 } else { 0.0 }),
            detail: failures.join("; "),
        });
        outcomes.push(outcome);
    }

    let axes = JunkAxis::ALL
        .iter()
        .map(|axis| {
            let confusion = confusion_for(&outcomes, *axis);
            AxisReport {
                axis: axis.as_str().to_string(),
                confusion,
                precision: confusion.precision(),
                recall: confusion.recall(),
                f1: confusion.f1(),
                legit_fp_rate: confusion.legit_fp_rate(),
                sweep: threshold_sweep(&outcomes, *axis),
            }
        })
        .collect::<Vec<_>>();

    let gates = evaluate_gates(&outcomes, &cfg.gates);
    let metrics = JunkMetricsReport {
        run_id: report.run_id.clone(),
        total_cases: outcomes.len(),
        axes,
        llm_routed_fraction: llm_routed_fraction(&outcomes),
        gates_passed: all_gates_passed(&gates),
        gates,
    };

    let report_path = report.write(&cfg.out_dir)?;
    std::fs::create_dir_all(&cfg.out_dir)?;
    let metrics_path = cfg.out_dir.join(format!("{}_metrics.json", metrics.run_id));
    std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics)?)?;

    print_summary(&metrics, &report);

    Ok(JunkEvalSummary {
        report_path,
        metrics_path,
        metrics,
    })
}

fn fmt_opt(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{:.3}", v),
        None => "n/a".to_string(),
    }
}

fn print_summary(metrics: &JunkMetricsReport, report: &JsonRunReport) {
    eprintln!("\n[junk-eval] {} cases", metrics.total_cases);
    eprintln!(
        "[junk-eval] cases fully matching expectations: {}/{}",
        report.succeeded, report.total
    );

    eprintln!("\n  axis       TP  FP  TN  FN   precision  recall   legit_fp");
    for axis in &metrics.axes {
        let c = axis.confusion;
        eprintln!(
            "  {:<9} {:>3} {:>3} {:>3} {:>3}   {:>9} {:>7}   {:>8}",
            axis.axis,
            c.true_pos,
            c.false_pos,
            c.true_neg,
            c.false_neg,
            fmt_opt(axis.precision),
            fmt_opt(axis.recall),
            fmt_opt(axis.legit_fp_rate),
        );
    }

    eprintln!("\n  gates");
    for gate in &metrics.gates {
        eprintln!(
            "    {} {:<24} {:.4} (limit {:.4})",
            if gate.passed { "PASS" } else { "FAIL" },
            gate.name,
            gate.actual,
            gate.limit
        );
    }

    if metrics.gates_passed {
        eprintln!("\n[junk-eval] gates PASSED");
    } else {
        eprintln!("\n[junk-eval] gates FAILED — a false positive on real mail is a build failure");
    }
}

/// Per-tier case counts, for the corpus-composition assertion below.
pub fn case_counts_by_axis(cases: &[JunkCase]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for case in cases {
        let expected = case.expect.flagged_map();
        let label = if expected.values().any(|flagged| *flagged) {
            expected
                .iter()
                .find(|(_, flagged)| **flagged)
                .map(|(axis, _)| axis.as_str().to_string())
                .unwrap_or_else(|| "legit".to_string())
        } else {
            "legit".to_string()
        };
        *counts.entry(label).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cases_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evals/junk/cases")
    }

    #[test]
    fn the_shipped_corpus_loads_and_has_unique_ids() {
        let cases = load_cases(&cases_dir()).expect("corpus loads");
        assert!(!cases.is_empty(), "corpus must not be empty");
    }

    #[test]
    fn the_corpus_is_weighted_towards_legitimate_mail() {
        // The legit half is what the false-positive budget is measured against.
        // A corpus that is mostly attacks would let a trigger-happy detector
        // look good.
        let cases = load_cases(&cases_dir()).expect("corpus loads");
        let counts = case_counts_by_axis(&cases);
        let legit = counts.get("legit").copied().unwrap_or(0);
        assert!(
            legit * 3 >= cases.len(),
            "legit cases ({legit}) should be at least a third of {} — counts: {counts:?}",
            cases.len()
        );
    }

    #[test]
    fn every_case_takes_a_position_on_at_least_one_axis() {
        let cases = load_cases(&cases_dir()).expect("corpus loads");
        for case in &cases {
            assert!(
                !case.expect.flagged_map().is_empty(),
                "case {} declares no expectations",
                case.id
            );
        }
    }

    fn scored_corpus() -> Vec<CaseOutcome> {
        let cases = load_cases(&cases_dir()).expect("corpus loads");
        let weights = Weights::default();
        cases.iter().map(|c| score_case(c, &weights).0).collect()
    }

    #[test]
    fn the_detector_never_flags_legitimate_mail_in_the_corpus() {
        // The gate, enforced by `cargo test` and not only by `make eval-junk`.
        // This is the assertion that matters: a false positive on real mail is
        // the failure mode that destroys trust in the whole feature.
        let outcomes = scored_corpus();
        let gates = evaluate_gates(&outcomes, &Gates::default());
        assert!(all_gates_passed(&gates), "{gates:?}");

        for axis in JunkAxis::ALL {
            let c = confusion_for(&outcomes, axis);
            assert_eq!(c.false_pos, 0, "{} false positives", axis.as_str());
        }
    }

    #[test]
    fn the_detector_catches_what_the_corpus_declares_junk() {
        // The other half: without this, a detector that simply returns Clean
        // would pass the gate above forever.
        let outcomes = scored_corpus();
        for axis in JunkAxis::ALL {
            let c = confusion_for(&outcomes, axis);
            assert!(c.true_pos > 0, "{} caught nothing", axis.as_str());
            assert_eq!(c.false_neg, 0, "{} missed {} case(s)", axis.as_str(), c.false_neg);
        }
    }

    #[test]
    fn every_case_matches_its_expected_verdict_and_reasons() {
        // Reasons are checked too, not just the verdict — otherwise a case can
        // pass for the wrong cause and the detector's explanation to the user
        // would be wrong even when the badge is right.
        let cases = load_cases(&cases_dir()).expect("corpus loads");
        let weights = Weights::default();
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|case| {
                let (_, failures) = score_case(case, &weights);
                (!failures.is_empty()).then(|| format!("{}: {}", case.id, failures.join("; ")))
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
