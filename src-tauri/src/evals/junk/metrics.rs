//! Confusion matrix, per-axis precision/recall, false-positive rate, threshold
//! sweep and the CI gates for the junk detector.
//!
//! The asymmetry is the whole point. A false negative means a spam message
//! reaches the inbox: the user deletes it. A false positive means real mail is
//! badged as junk and pushed to the bottom of the list: the user misses an
//! invoice, and from then on checks the junk group every time — which is
//! exactly the work the feature was supposed to remove. So a miss is a warning
//! and a false positive fails the build.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::services::junk::verdict::{JunkAxis, JunkVerdict, Method};

/// Ground truth plus what the detector actually said, for one case.
#[derive(Debug, Clone)]
pub struct CaseOutcome {
    pub case_id: String,
    /// Per axis: should the detector have flagged this message?
    /// Ground truth is binary — `Uncertain` and `Unknown` are detector states,
    /// not labels a human can assign.
    pub expected_flagged: BTreeMap<JunkAxis, bool>,
    pub verdict: JunkVerdict,
}

/// Counts for one axis at one decision threshold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisConfusion {
    pub true_pos: usize,
    pub false_pos: usize,
    pub true_neg: usize,
    pub false_neg: usize,
}

impl AxisConfusion {
    pub fn total(&self) -> usize {
        self.true_pos + self.false_pos + self.true_neg + self.false_neg
    }

    /// Of what we flagged, how much was right.
    ///
    /// `None` when nothing was flagged: precision is genuinely undefined there,
    /// and collapsing it to 0.0 would make a silent detector look terrible while
    /// collapsing it to 1.0 would make it look perfect. Neither is true.
    pub fn precision(&self) -> Option<f64> {
        let flagged = self.true_pos + self.false_pos;
        if flagged == 0 {
            return None;
        }
        Some(self.true_pos as f64 / flagged as f64)
    }

    /// Of what was there, how much we caught.
    ///
    /// `None` when the axis had no positives in the corpus at all — there was
    /// nothing to recall.
    pub fn recall(&self) -> Option<f64> {
        let positives = self.true_pos + self.false_neg;
        if positives == 0 {
            return None;
        }
        Some(self.true_pos as f64 / positives as f64)
    }

    pub fn f1(&self) -> Option<f64> {
        let (p, r) = (self.precision()?, self.recall()?);
        if p + r == 0.0 {
            return None;
        }
        Some(2.0 * p * r / (p + r))
    }

    /// The headline number: of the messages that were genuinely fine, what
    /// fraction did we badge as junk.
    pub fn legit_fp_rate(&self) -> Option<f64> {
        let negatives = self.false_pos + self.true_neg;
        if negatives == 0 {
            return None;
        }
        Some(self.false_pos as f64 / negatives as f64)
    }
}

/// Build the confusion matrix for one axis using the detector's own banding.
///
/// Cases that declare no expectation for `axis` are skipped rather than assumed
/// clean: a phishing case says nothing about graymail, and counting it as an
/// implicit negative would inflate the true-negative population and silently
/// deflate every rate computed from it.
pub fn confusion_for(outcomes: &[CaseOutcome], axis: JunkAxis) -> AxisConfusion {
    let mut c = AxisConfusion::default();
    for outcome in outcomes {
        let Some(&expected) = outcome.expected_flagged.get(&axis) else {
            continue;
        };
        let flagged = outcome.verdict.axis(axis).band.is_flagged();
        match (expected, flagged) {
            (true, true) => c.true_pos += 1,
            (false, true) => c.false_pos += 1,
            (false, false) => c.true_neg += 1,
            (true, false) => c.false_neg += 1,
        }
    }
    c
}

/// One point on the precision/recall curve.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SweepPoint {
    pub cutoff: f64,
    pub confusion: AxisConfusion,
}

/// Cutoffs swept, inclusive, in 0.05 steps.
pub const SWEEP_MIN: f64 = 0.05;
pub const SWEEP_MAX: f64 = 0.95;
pub const SWEEP_STEP: f64 = 0.05;

/// Number of points in the sweep, inclusive of both endpoints.
pub fn sweep_len() -> usize {
    ((SWEEP_MAX - SWEEP_MIN) / SWEEP_STEP).round() as usize + 1
}

/// Re-band every case at each cutoff and recompute the matrix, so the chosen
/// production cutoff is picked off a curve instead of out of the air.
///
/// Deliberately ignores the detector's own band and thresholds the raw score —
/// that is the entire point of sweeping.
pub fn threshold_sweep(outcomes: &[CaseOutcome], axis: JunkAxis) -> Vec<SweepPoint> {
    (0..sweep_len())
        .map(|i| {
            let cutoff = SWEEP_MIN + i as f64 * SWEEP_STEP;
            let mut c = AxisConfusion::default();
            for outcome in outcomes {
                let Some(&expected) = outcome.expected_flagged.get(&axis) else {
                    continue;
                };
                let flagged = outcome.verdict.axis(axis).score as f64 >= cutoff;
                match (expected, flagged) {
                    (true, true) => c.true_pos += 1,
                    (false, true) => c.false_pos += 1,
                    (false, false) => c.true_neg += 1,
                    (true, false) => c.false_neg += 1,
                }
            }
            SweepPoint { cutoff, confusion: c }
        })
        .collect()
}

/// Fraction of cases that reached the LLM layer.
pub fn llm_routed_fraction(outcomes: &[CaseOutcome]) -> f64 {
    if outcomes.is_empty() {
        return 0.0;
    }
    let routed = outcomes.iter().filter(|o| o.verdict.method == Method::Llm).count();
    routed as f64 / outcomes.len() as f64
}

/// The CI budget. Blowing any of these fails the run.
#[derive(Debug, Clone, PartialEq)]
pub struct Gates {
    pub phishing_max_fp_rate: f64,
    pub spam_max_fp_rate: f64,
    pub graymail_max_fp_rate: f64,
    pub max_llm_routed_fraction: f64,
}

impl Default for Gates {
    fn default() -> Self {
        Self {
            // Zero tolerance: a message badged as a phishing attempt is the
            // strongest claim the app makes about anything, and on the curated
            // synthetic legit set there is no excuse for getting it wrong.
            phishing_max_fp_rate: 0.0,
            spam_max_fp_rate: 0.005,
            // Graymail only deprioritizes; being wrong costs the user a glance,
            // not a missed invoice.
            graymail_max_fp_rate: 0.02,
            max_llm_routed_fraction: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateOutcome {
    pub name: String,
    pub actual: f64,
    pub limit: f64,
    pub passed: bool,
}

/// Float slack so a gate with a 0.0 limit isn't tripped by representation noise.
const GATE_EPS: f64 = 1e-9;

pub fn evaluate_gates(outcomes: &[CaseOutcome], gates: &Gates) -> Vec<GateOutcome> {
    let mut results = Vec::with_capacity(4);

    for (axis, limit) in [
        (JunkAxis::Phishing, gates.phishing_max_fp_rate),
        (JunkAxis::Spam, gates.spam_max_fp_rate),
        (JunkAxis::Graymail, gates.graymail_max_fp_rate),
    ] {
        // No ground-truth-clean cases for this axis means there is nothing the
        // detector could have got wrong, so the gate is vacuously satisfied.
        let actual = confusion_for(outcomes, axis).legit_fp_rate().unwrap_or(0.0);
        results.push(GateOutcome {
            name: format!("{}_legit_fp_rate", axis.as_str()),
            actual,
            limit,
            passed: actual <= limit + GATE_EPS,
        });
    }

    let routed = llm_routed_fraction(outcomes);
    results.push(GateOutcome {
        name: "llm_routed_fraction".to_string(),
        actual: routed,
        limit: gates.max_llm_routed_fraction,
        passed: routed <= gates.max_llm_routed_fraction + GATE_EPS,
    });

    results
}

pub fn all_gates_passed(results: &[GateOutcome]) -> bool {
    results.iter().all(|g| g.passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::junk::verdict::{AxisScore, Band, JunkKind};

    const EPS: f64 = 1e-9;

    /// Build an outcome whose detector scores/bands we control directly.
    fn outcome(
        id: &str,
        expected: &[(JunkAxis, bool)],
        actual: &[(JunkAxis, f32, Band)],
        method: Method,
    ) -> CaseOutcome {
        let mut verdict = JunkVerdict {
            method,
            ..JunkVerdict::clean()
        };
        for (axis, score, band) in actual {
            let scored = AxisScore {
                score: *score,
                band: *band,
            };
            match axis {
                JunkAxis::Phishing => verdict.phishing = scored,
                JunkAxis::Spam => verdict.spam = scored,
                JunkAxis::Graymail => verdict.graymail = scored,
            }
        }
        if actual.iter().any(|(_, _, b)| b.is_flagged()) {
            verdict.primary = JunkKind::Spam;
        }
        CaseOutcome {
            case_id: id.to_string(),
            expected_flagged: expected.iter().copied().collect(),
            verdict,
        }
    }

    fn spam_case(id: &str, expected: bool, score: f32, band: Band) -> CaseOutcome {
        outcome(
            id,
            &[(JunkAxis::Spam, expected)],
            &[(JunkAxis::Spam, score, band)],
            Method::Deterministic,
        )
    }

    // ── AxisConfusion arithmetic ──────────────────────────────────────────

    #[test]
    fn precision_is_undefined_when_nothing_was_flagged() {
        let c = AxisConfusion {
            true_pos: 0,
            false_pos: 0,
            true_neg: 90,
            false_neg: 10,
        };
        assert_eq!(c.precision(), None);
    }

    #[test]
    fn recall_is_zero_when_positives_exist_but_none_were_caught() {
        let c = AxisConfusion {
            true_pos: 0,
            false_pos: 0,
            true_neg: 90,
            false_neg: 10,
        };
        let recall = c.recall().expect("recall is defined when positives exist");
        assert!(recall.abs() < EPS, "expected recall 0.0, got {recall}");
    }

    #[test]
    fn recall_is_undefined_when_there_are_no_positives_to_catch() {
        let c = AxisConfusion {
            true_pos: 0,
            false_pos: 3,
            true_neg: 97,
            false_neg: 0,
        };
        assert_eq!(c.recall(), None);
    }

    #[test]
    fn precision_and_recall_use_the_standard_ratios() {
        let c = AxisConfusion {
            true_pos: 9,
            false_pos: 3,
            true_neg: 87,
            false_neg: 1,
        };
        let p = c.precision().expect("precision defined");
        let r = c.recall().expect("recall defined");
        assert!((p - 0.75).abs() < EPS, "precision {p}");
        assert!((r - 0.9).abs() < EPS, "recall {r}");

        let f1 = c.f1().expect("f1 defined");
        let expected_f1 = 2.0 * 0.75 * 0.9 / (0.75 + 0.9);
        assert!((f1 - expected_f1).abs() < EPS, "f1 {f1}");
    }

    #[test]
    fn legit_fp_rate_divides_by_the_ground_truth_clean_population() {
        // 3 of 90 genuinely-fine messages were badged.
        let c = AxisConfusion {
            true_pos: 9,
            false_pos: 3,
            true_neg: 87,
            false_neg: 1,
        };
        let fp = c.legit_fp_rate().expect("fp rate defined");
        assert!((fp - 3.0 / 90.0).abs() < EPS, "fp rate {fp}");
    }

    // ── confusion_for ─────────────────────────────────────────────────────

    #[test]
    fn confusion_counts_each_quadrant_from_the_detector_band() {
        let outcomes = vec![
            spam_case("tp", true, 0.9, Band::Junk),
            spam_case("fp", false, 0.9, Band::Junk),
            spam_case("fn", true, 0.1, Band::Clean),
            spam_case("tn", false, 0.1, Band::Clean),
        ];
        let c = confusion_for(&outcomes, JunkAxis::Spam);
        assert_eq!(
            c,
            AxisConfusion {
                true_pos: 1,
                false_pos: 1,
                true_neg: 1,
                false_neg: 1,
            }
        );
    }

    #[test]
    fn uncertain_band_is_not_counted_as_a_flag() {
        // Uncertain exists so the LLM layer has somewhere to be consulted. The
        // user never sees it, so it must never count as a positive.
        let outcomes = vec![
            spam_case("uncertain-on-clean", false, 0.5, Band::Uncertain),
            spam_case("uncertain-on-spam", true, 0.5, Band::Uncertain),
        ];
        let c = confusion_for(&outcomes, JunkAxis::Spam);
        assert_eq!(c.false_pos, 0, "uncertain must not be a false positive");
        assert_eq!(c.true_neg, 1);
        assert_eq!(c.false_neg, 1);
    }

    #[test]
    fn cases_without_an_expectation_for_the_axis_are_skipped() {
        // A phishing case says nothing about the graymail axis; counting it as
        // an implicit "clean" would inflate the true-negative population and
        // silently deflate every graymail rate.
        let outcomes = vec![outcome(
            "phish-only",
            &[(JunkAxis::Phishing, true)],
            &[(JunkAxis::Phishing, 0.9, Band::Junk)],
            Method::Deterministic,
        )];
        let c = confusion_for(&outcomes, JunkAxis::Graymail);
        assert_eq!(c.total(), 0);
    }

    #[test]
    fn axes_are_scored_independently() {
        // A legitimate newsletter: graymail yes, spam no. Flagging graymail must
        // not register anywhere on the spam axis.
        let outcomes = vec![outcome(
            "newsletter",
            &[(JunkAxis::Graymail, true), (JunkAxis::Spam, false)],
            &[
                (JunkAxis::Graymail, 0.8, Band::Junk),
                (JunkAxis::Spam, 0.1, Band::Clean),
            ],
            Method::Deterministic,
        )];

        let gray = confusion_for(&outcomes, JunkAxis::Graymail);
        assert_eq!(gray.true_pos, 1);

        let spam = confusion_for(&outcomes, JunkAxis::Spam);
        assert_eq!(spam.false_pos, 0);
        assert_eq!(spam.true_neg, 1);
    }

    // ── The stub baseline ─────────────────────────────────────────────────

    #[test]
    fn the_all_clean_stub_has_zero_false_positives_and_passes_every_gate() {
        // This is the Stage 0 contract: the harness ships green against a
        // detector that claims nothing, so every later stage is a measurable
        // diff rather than a fresh set of numbers with nothing to compare to.
        let outcomes = vec![
            spam_case("legit-1", false, 0.0, Band::Clean),
            spam_case("legit-2", false, 0.0, Band::Clean),
            spam_case("spam-1", true, 0.0, Band::Clean),
        ];

        let c = confusion_for(&outcomes, JunkAxis::Spam);
        assert_eq!(c.false_pos, 0);
        let fp = c.legit_fp_rate().expect("fp rate defined");
        assert!(fp.abs() < EPS);
        assert_eq!(c.precision(), None, "nothing flagged → precision undefined");

        let gates = evaluate_gates(&outcomes, &Gates::default());
        assert!(all_gates_passed(&gates), "stub must pass the gates: {gates:?}");
    }

    // ── Gates ─────────────────────────────────────────────────────────────

    #[test]
    fn a_single_phishing_false_positive_fails_the_build() {
        let outcomes = vec![
            outcome(
                "legit",
                &[(JunkAxis::Phishing, false)],
                &[(JunkAxis::Phishing, 0.9, Band::Junk)],
                Method::Deterministic,
            ),
            outcome(
                "legit-2",
                &[(JunkAxis::Phishing, false)],
                &[(JunkAxis::Phishing, 0.0, Band::Clean)],
                Method::Deterministic,
            ),
        ];

        let gates = evaluate_gates(&outcomes, &Gates::default());
        assert!(!all_gates_passed(&gates));

        let phishing = gates
            .iter()
            .find(|g| g.name.contains("phishing"))
            .expect("phishing gate present");
        assert!(!phishing.passed);
        assert!((phishing.actual - 0.5).abs() < EPS, "actual {}", phishing.actual);
    }

    #[test]
    fn missed_spam_alone_does_not_fail_the_build() {
        // Recall 0% with zero false positives is a warning, not a failure.
        let outcomes = vec![
            spam_case("spam-1", true, 0.0, Band::Clean),
            spam_case("spam-2", true, 0.0, Band::Clean),
            spam_case("legit", false, 0.0, Band::Clean),
        ];
        let gates = evaluate_gates(&outcomes, &Gates::default());
        assert!(all_gates_passed(&gates), "{gates:?}");
    }

    #[test]
    fn routing_too_much_mail_to_the_llm_fails_the_build() {
        let mut outcomes = vec![outcome(
            "llm",
            &[(JunkAxis::Spam, false)],
            &[(JunkAxis::Spam, 0.0, Band::Clean)],
            Method::Llm,
        )];
        for i in 0..9 {
            outcomes.push(spam_case(&format!("cheap-{i}"), false, 0.0, Band::Clean));
        }

        // 1 of 10 = 10%, over the 5% budget.
        let fraction = llm_routed_fraction(&outcomes);
        assert!((fraction - 0.1).abs() < EPS, "fraction {fraction}");

        let gates = evaluate_gates(&outcomes, &Gates::default());
        assert!(!all_gates_passed(&gates));
    }

    // ── Threshold sweep ───────────────────────────────────────────────────

    #[test]
    fn sweep_covers_the_configured_cutoff_range() {
        let outcomes = vec![spam_case("a", true, 0.5, Band::Junk)];
        let sweep = threshold_sweep(&outcomes, JunkAxis::Spam);

        assert_eq!(sweep.len(), 19, "0.05..=0.95 in 0.05 steps");
        let first = sweep.first().expect("non-empty sweep");
        let last = sweep.last().expect("non-empty sweep");
        assert!((first.cutoff - SWEEP_MIN).abs() < EPS);
        assert!((last.cutoff - SWEEP_MAX).abs() < EPS);
    }

    #[test]
    fn sweep_uses_raw_scores_not_the_production_band() {
        // At cutoff 0.10 a score of 0.50 is a positive even though the detector
        // banded it Clean — that is the entire point of sweeping.
        let outcomes = vec![spam_case("a", true, 0.5, Band::Clean)];
        let sweep = threshold_sweep(&outcomes, JunkAxis::Spam);

        let low = sweep.first().expect("first point");
        assert_eq!(low.confusion.true_pos, 1);

        let high = sweep.last().expect("last point");
        assert_eq!(high.confusion.false_neg, 1);
    }

    #[test]
    fn raising_the_cutoff_never_increases_recall() {
        let outcomes = vec![
            spam_case("a", true, 0.9, Band::Junk),
            spam_case("b", true, 0.5, Band::Uncertain),
            spam_case("c", true, 0.2, Band::Clean),
            spam_case("d", false, 0.3, Band::Clean),
        ];
        let sweep = threshold_sweep(&outcomes, JunkAxis::Spam);

        let mut prev = f64::INFINITY;
        for point in &sweep {
            let recall = point.confusion.recall().expect("positives exist");
            assert!(
                recall <= prev + EPS,
                "recall rose from {prev} to {recall} at cutoff {}",
                point.cutoff
            );
            prev = recall;
        }
    }
}
