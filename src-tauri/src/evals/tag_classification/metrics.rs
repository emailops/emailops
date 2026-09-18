// Scoring for the tag-classification harness.
//
// Every case names one gold label per axis plus the alternatives a human
// would also accept, so the harness reports two accuracies: strict (the gold
// label exactly) and accepted (any listed alternative). Macro-F1 sits beside
// them because accuracy alone hides a model that answers `notification` to
// everything on a corpus where `notification` is the largest class.

use std::collections::BTreeSet;

use serde::Serialize;

/// One axis (intent / topic / urgency) of one case, after the model answered.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldOutcome {
    /// The single best label — `expect.<field>[0]` in the case file.
    pub gold: String,
    /// Every label the case accepts, gold first.
    pub accepted: Vec<String>,
    /// What the classifier returned. `None` when the call failed before a
    /// label existed, which is scored as a miss rather than dropped.
    pub predicted: Option<String>,
}

impl FieldOutcome {
    /// The prediction is the gold label.
    pub fn is_strict(&self) -> bool {
        self.predicted.as_deref() == Some(self.gold.as_str())
    }

    /// The prediction is one of the labels the case accepts.
    pub fn is_accepted(&self) -> bool {
        match &self.predicted {
            Some(p) => self.accepted.iter().any(|a| a == p),
            None => false,
        }
    }

    /// The prediction as macro-F1 sees it: an accepted alternative collapses
    /// onto the gold label, so a deliberate tie between two labels doesn't
    /// count as a confusion. Anything else is reported as predicted.
    pub fn effective_prediction(&self) -> Option<&str> {
        if self.is_accepted() {
            Some(self.gold.as_str())
        } else {
            self.predicted.as_deref()
        }
    }
}

/// Per-label counts behind the macro average.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelF1 {
    pub label: String,
    pub true_pos: usize,
    pub false_pos: usize,
    pub false_neg: usize,
    pub f1: f64,
}

/// One axis, scored across the corpus.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldScore {
    /// Cases in the corpus for this axis.
    pub total: usize,
    /// Cases that produced a label at all (`total` minus the failures).
    pub scored: usize,
    pub strict: usize,
    pub accepted: usize,
    /// Unweighted mean of the per-label F1 scores. `None` when nothing was
    /// scored.
    pub macro_f1: Option<f64>,
    pub labels: Vec<LabelF1>,
}

impl FieldScore {
    /// Strict accuracy over the whole corpus — a failed call counts against
    /// it, because in the app a failed call means the email stays untagged.
    pub fn strict_accuracy(&self) -> Option<f64> {
        (self.total > 0).then(|| self.strict as f64 / self.total as f64)
    }

    /// Accuracy allowing every alternative the case accepts.
    pub fn accepted_accuracy(&self) -> Option<f64> {
        (self.total > 0).then(|| self.accepted as f64 / self.total as f64)
    }
}

/// Score one axis across every case.
pub fn score_field(outcomes: &[FieldOutcome]) -> FieldScore {
    let total = outcomes.len();
    let scored = outcomes.iter().filter(|o| o.predicted.is_some()).count();
    let strict = outcomes.iter().filter(|o| o.is_strict()).count();
    let accepted = outcomes.iter().filter(|o| o.is_accepted()).count();

    // Labels are the golds present plus anything the model actually answered,
    // so a hallucinated label shows up as its own row with precision 0.
    let mut labels: BTreeSet<&str> = BTreeSet::new();
    for o in outcomes.iter().filter(|o| o.predicted.is_some()) {
        labels.insert(o.gold.as_str());
        if let Some(p) = o.effective_prediction() {
            labels.insert(p);
        }
    }

    let scored_outcomes: Vec<&FieldOutcome> = outcomes.iter().filter(|o| o.predicted.is_some()).collect();
    let per_label: Vec<LabelF1> = labels
        .into_iter()
        .map(|label| {
            let mut true_pos = 0;
            let mut false_pos = 0;
            let mut false_neg = 0;
            for o in &scored_outcomes {
                let is_gold = o.gold == label;
                let is_pred = o.effective_prediction() == Some(label);
                match (is_gold, is_pred) {
                    (true, true) => true_pos += 1,
                    (false, true) => false_pos += 1,
                    (true, false) => false_neg += 1,
                    (false, false) => {}
                }
            }
            let denom = 2 * true_pos + false_pos + false_neg;
            let f1 = if denom == 0 {
                0.0
            } else {
                2.0 * true_pos as f64 / denom as f64
            };
            LabelF1 {
                label: label.to_string(),
                true_pos,
                false_pos,
                false_neg,
                f1,
            }
        })
        .collect();

    let macro_f1 = if per_label.is_empty() {
        None
    } else {
        Some(per_label.iter().map(|l| l.f1).sum::<f64>() / per_label.len() as f64)
    };

    FieldScore {
        total,
        scored,
        strict,
        accepted,
        macro_f1,
        labels: per_label,
    }
}

/// Nearest-rank percentile of an already-sorted sample. `None` when empty.
pub fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank.min(sorted.len()) - 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(gold: &str, accepted: &[&str], predicted: Option<&str>) -> FieldOutcome {
        FieldOutcome {
            gold: gold.to_string(),
            accepted: accepted.iter().map(|s| s.to_string()).collect(),
            predicted: predicted.map(|s| s.to_string()),
        }
    }

    #[test]
    fn a_gold_prediction_is_both_strict_and_accepted() {
        let o = outcome("request", &["request", "question"], Some("request"));
        assert!(o.is_strict());
        assert!(o.is_accepted());
    }

    #[test]
    fn an_alternative_is_accepted_but_not_strict() {
        let o = outcome("request", &["request", "question"], Some("question"));
        assert!(!o.is_strict());
        assert!(o.is_accepted());
    }

    #[test]
    fn a_missing_prediction_scores_neither() {
        let o = outcome("request", &["request"], None);
        assert!(!o.is_strict());
        assert!(!o.is_accepted());
    }

    #[test]
    fn an_accepted_alternative_collapses_onto_gold_for_f1() {
        let o = outcome("request", &["request", "question"], Some("question"));
        assert_eq!(o.effective_prediction(), Some("request"));

        let wrong = outcome("request", &["request"], Some("promotion"));
        assert_eq!(wrong.effective_prediction(), Some("promotion"));
    }

    #[test]
    fn score_field_counts_both_accuracies_over_scored_cases_only() {
        let score = score_field(&[
            outcome("request", &["request"], Some("request")),
            outcome("request", &["request", "question"], Some("question")),
            outcome("billing", &["billing"], Some("sales")),
            outcome("billing", &["billing"], None),
        ]);

        assert_eq!(score.total, 4);
        assert_eq!(score.scored, 3);
        assert_eq!(score.strict, 1);
        assert_eq!(score.accepted, 2);
    }

    #[test]
    fn macro_f1_averages_over_labels_not_cases() {
        // `request` is right twice and over-predicted once (TP 2, FP 1 →
        // F1 0.8); `billing` is never predicted (F1 0.0). The macro average
        // is 0.4 even though 2 of 3 cases are right — which is the point of
        // reporting it next to accuracy.
        let score = score_field(&[
            outcome("request", &["request"], Some("request")),
            outcome("request", &["request"], Some("request")),
            outcome("billing", &["billing"], Some("request")),
        ]);

        assert_eq!(score.macro_f1, Some(0.8 * 0.5));
    }

    #[test]
    fn macro_f1_is_one_when_every_label_is_predicted_exactly() {
        let score = score_field(&[
            outcome("request", &["request"], Some("request")),
            outcome("billing", &["billing"], Some("billing")),
        ]);

        assert_eq!(score.macro_f1, Some(1.0));
    }

    #[test]
    fn macro_f1_is_undefined_without_a_single_scored_case() {
        let score = score_field(&[outcome("request", &["request"], None)]);
        assert_eq!(score.macro_f1, None);
        assert_eq!(score.labels, Vec::new());
    }

    #[test]
    fn percentile_picks_the_nearest_rank() {
        let samples = [10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(percentile(&samples, 0.5), Some(50));
        assert_eq!(percentile(&samples, 0.95), Some(100));
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7], 0.95), Some(7));
    }
}
