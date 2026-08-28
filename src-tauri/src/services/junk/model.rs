//! Naive Bayes over hashed features — the personal layer.
//!
//! The deterministic rules know what bulk mail *is*; they cannot know which
//! bulk mail *this user* reads. That judgement is personal and has to be learnt
//! from their own mailbox.
//!
//! Chosen over logistic regression because the whole model is two arrays of
//! counters: the update is closed-form, there is no learning rate to tune, it
//! behaves sanely with very few labels, and it needs no dependency.
//!
//! **The prior is not learnt.** See [`score`] — it is the single most important
//! decision in this module.

use serde::{Deserialize, Serialize};

use super::tokens::BUCKETS;

/// Laplace smoothing. Without it, a token never seen in one class makes the
/// whole product collapse to zero on a single unlucky word.
const ALPHA: f32 = 1.0;

/// Per-token log-likelihood-ratio clamp.
///
/// A token seen a handful of times in one class and never in the other produces
/// an enormous ratio that is mostly noise. Capping each token's vote stops one
/// accidental word from deciding the verdict on its own.
const MAX_TOKEN_LOGIT: f32 = 2.0;

/// How many tokens vote.
///
/// Only the most discriminating ones are counted. Summing every token in a
/// message drowns the signal in hundreds of near-neutral words and saturates
/// the result at 0 or 1 — the classic failure of textbook multinomial NB on
/// real text. Real-world Bayesian spam filters have always used a small set of
/// most-significant tokens for exactly this reason.
const TOP_TOKENS: usize = 30;

/// Minimum labels before a model is allowed to influence anything.
///
/// Below this the counts are noise, and a confidently wrong personal model is
/// worse than none: the deterministic layer alone is at least predictable.
pub const MIN_SAMPLES_PER_CLASS: u32 = 40;

/// Which axis a model was trained for.
///
/// Phishing is deliberately absent. A mailbox yields a handful of phishing
/// examples at best, and a model trained on that produces noise rather than
/// signal — the axis stays deterministic. Encoding this as a two-variant enum
/// makes "train a phishing model" unrepresentable rather than merely discouraged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAxis {
    Spam,
    Graymail,
}

impl ModelAxis {
    pub const ALL: [ModelAxis; 2] = [ModelAxis::Spam, ModelAxis::Graymail];

    pub fn as_str(self) -> &'static str {
        match self {
            ModelAxis::Spam => "spam",
            ModelAxis::Graymail => "graymail",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "spam" => Some(ModelAxis::Spam),
            "graymail" => Some(ModelAxis::Graymail),
            _ => None,
        }
    }
}

/// One labelled example.
pub struct Sample {
    pub features: Vec<u32>,
    pub positive: bool,
    /// Repeat count. The user's own corrections are weighted far above
    /// automatically-derived labels.
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NaiveBayes {
    pub pos: Vec<u32>,
    pub neg: Vec<u32>,
    pub n_pos: u32,
    pub n_neg: u32,
}

impl Default for NaiveBayes {
    fn default() -> Self {
        Self {
            pos: vec![0; BUCKETS],
            neg: vec![0; BUCKETS],
            n_pos: 0,
            n_neg: 0,
        }
    }
}

impl NaiveBayes {
    /// Is there enough evidence for this model to be trusted at all?
    pub fn is_usable(&self) -> bool {
        self.n_pos >= MIN_SAMPLES_PER_CLASS && self.n_neg >= MIN_SAMPLES_PER_CLASS
    }
}

/// Train from scratch.
///
/// A full retrain rather than an incremental update: one pass over ≤20k rows of
/// subject + snippet + sender is seconds, it is reproducible, and it eliminates
/// a whole class of drift bugs where the counters and the corpus disagree.
pub fn train(samples: &[Sample]) -> NaiveBayes {
    let mut model = NaiveBayes::default();
    for sample in samples {
        let weight = sample.weight.max(1);
        let counts = if sample.positive {
            &mut model.pos
        } else {
            &mut model.neg
        };
        for &bucket in &sample.features {
            if let Some(slot) = counts.get_mut(bucket as usize) {
                *slot = slot.saturating_add(weight);
            }
        }
        if sample.positive {
            model.n_pos = model.n_pos.saturating_add(weight);
        } else {
            model.n_neg = model.n_neg.saturating_add(weight);
        }
    }
    model
}

/// Probability that a message belongs to the positive class.
///
/// `prior` is the base rate — the belief before any evidence is examined — and
/// it is supplied by the caller rather than derived from `n_pos / n_neg`.
///
/// That distinction is the crux. The free training labels come from the
/// provider's spam folder, which is not a random sample of the inbox: it
/// deliberately concentrates months of junk next to a slice of ordinary mail. On
/// a real mailbox the empirical ratio can read ~25% while the inbox's actual
/// spam rate is ~1%. Learning the prior from those counts would inflate it by a
/// factor of twenty-five — and at 25% the classifier needs roughly thirty times
/// less evidence to accuse than it does at 1%. The bias would be invisible in
/// training metrics and devastating on the inbox.
pub fn score(model: &NaiveBayes, features: &[u32], prior: f32) -> f32 {
    if !model.is_usable() || features.is_empty() {
        return prior;
    }

    let prior = prior.clamp(0.0001, 0.9999);
    let n_pos = model.n_pos as f32;
    let n_neg = model.n_neg as f32;

    // Deduplicate here rather than trusting the caller. `tokens::features`
    // already returns a unique set, but a repeated bucket reaching this loop
    // would let one word vote as many times as it appears and drive the result
    // straight to a hard 0 or 1 — a precondition too dangerous to leave implicit.
    let mut unique: Vec<u32> = features.to_vec();
    unique.sort_unstable();
    unique.dedup();

    // Per-token log(P(token | positive) / P(token | negative)).
    let mut logits: Vec<f32> = unique
        .iter()
        .filter_map(|&bucket| {
            let idx = bucket as usize;
            let pos = *model.pos.get(idx)? as f32;
            let neg = *model.neg.get(idx)? as f32;
            // A token absent from both classes carries no information; letting
            // it through would just add smoothing noise.
            if pos == 0.0 && neg == 0.0 {
                return None;
            }
            let p_pos = (pos + ALPHA) / (n_pos + 2.0 * ALPHA);
            let p_neg = (neg + ALPHA) / (n_neg + 2.0 * ALPHA);
            Some((p_pos / p_neg).ln().clamp(-MAX_TOKEN_LOGIT, MAX_TOKEN_LOGIT))
        })
        .collect();

    if logits.is_empty() {
        return prior;
    }

    // Only the most discriminating tokens vote.
    logits.sort_by(|a, b| b.abs().partial_cmp(&a.abs()).unwrap_or(std::cmp::Ordering::Equal));
    logits.truncate(TOP_TOKENS);

    let evidence: f32 = logits.iter().sum();
    let prior_logit = (prior / (1.0 - prior)).ln();
    let total = prior_logit + evidence;

    // Logistic transform back to a probability.
    (1.0 / (1.0 + (-total).exp())).clamp(0.0, 1.0)
}

// ── Serialization ────────────────────────────────────────────────────────────

/// Pack the two count arrays into a little-endian blob for the DB.
pub fn to_blob(model: &NaiveBayes) -> Vec<u8> {
    let mut out = Vec::with_capacity(BUCKETS * 8);
    for counts in [&model.pos, &model.neg] {
        for value in counts {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// Unpack a blob. Returns `None` when the length does not match the current
/// bucket count — a bucket-size change makes old blobs meaningless rather than
/// merely stale, so they must be discarded and retrained, never reinterpreted.
pub fn from_blob(blob: &[u8], n_pos: u32, n_neg: u32) -> Option<NaiveBayes> {
    if blob.len() != BUCKETS * 8 {
        return None;
    }
    let mut pos = Vec::with_capacity(BUCKETS);
    let mut neg = Vec::with_capacity(BUCKETS);
    for (i, chunk) in blob.as_chunks::<4>().0.iter().enumerate() {
        let value = u32::from_le_bytes(*chunk);
        if i < BUCKETS {
            pos.push(value);
        } else {
            neg.push(value);
        }
    }
    Some(NaiveBayes { pos, neg, n_pos, n_neg })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(features: &[u32], positive: bool) -> Sample {
        Sample {
            features: features.to_vec(),
            positive,
            weight: 1,
        }
    }

    /// A model with enough evidence on both sides to be usable, where bucket 1
    /// is a strong positive marker and bucket 2 a strong negative one.
    fn trained() -> NaiveBayes {
        let mut samples = Vec::new();
        for _ in 0..60 {
            samples.push(sample(&[1, 10], true));
            samples.push(sample(&[2, 10], false));
        }
        train(&samples)
    }

    #[test]
    fn an_untrained_model_returns_the_prior_unchanged() {
        // No evidence means no opinion. Returning the prior is the honest
        // answer; returning 0.5 would be a claim the model cannot support.
        let model = NaiveBayes::default();
        assert_eq!(score(&model, &[1, 2, 3], 0.01), 0.01);
    }

    #[test]
    fn a_model_below_the_sample_floor_is_not_used() {
        let samples: Vec<Sample> = (0..10)
            .flat_map(|_| [sample(&[1], true), sample(&[2], false)])
            .collect();
        let model = train(&samples);
        assert!(!model.is_usable());
        assert_eq!(score(&model, &[1], 0.01), 0.01, "an under-trained model must not vote");
    }

    #[test]
    fn evidence_moves_the_score_in_the_right_direction() {
        let model = trained();
        let positive_evidence = score(&model, &[1], 0.5);
        let negative_evidence = score(&model, &[2], 0.5);
        assert!(positive_evidence > 0.5, "got {positive_evidence}");
        assert!(negative_evidence < 0.5, "got {negative_evidence}");
    }

    #[test]
    fn a_token_seen_equally_in_both_classes_barely_moves_the_score() {
        let model = trained();
        let neutral = score(&model, &[10], 0.5);
        assert!((neutral - 0.5).abs() < 0.05, "got {neutral}");
    }

    #[test]
    fn an_unseen_token_leaves_the_prior_untouched() {
        let model = trained();
        assert_eq!(score(&model, &[9999], 0.2), 0.2);
    }

    // ── The prior ─────────────────────────────────────────────────────────

    #[test]
    fn the_prior_is_supplied_not_derived_from_the_training_ratio() {
        // The whole point. This model saw 60 positives and 60 negatives — an
        // empirical rate of 50% — yet at a 1% prior identical evidence must
        // yield a far lower probability. Deriving the base rate from the
        // training counts is exactly the bug this guards against.
        let model = trained();
        let at_one_percent = score(&model, &[1], 0.01);
        let at_fifty_percent = score(&model, &[1], 0.50);
        assert!(
            at_one_percent < at_fifty_percent,
            "prior must anchor the result: {at_one_percent} vs {at_fifty_percent}"
        );
    }

    #[test]
    fn an_inflated_prior_accuses_on_far_weaker_evidence() {
        // Quantifies the damage. The distorted prior a spam-folder ratio would
        // produce (~25%) crosses the accusation line on evidence that the true
        // inbox rate (~1%) correctly rejects.
        let model = trained();
        let true_rate = score(&model, &[1], 0.01);
        let distorted = score(&model, &[1], 0.25);
        assert!(true_rate < 0.5, "at the real base rate this evidence is not enough");
        assert!(distorted > true_rate);
    }

    #[test]
    fn scores_stay_inside_probability_bounds() {
        let model = trained();
        for prior in [0.0, 0.001, 0.5, 0.999, 1.0] {
            for features in [vec![], vec![1; 200], vec![2; 200]] {
                let s = score(&model, &features, prior);
                assert!((0.0..=1.0).contains(&s), "prior {prior} → {s}");
            }
        }
    }

    #[test]
    fn repeating_a_token_adds_no_extra_evidence() {
        // Without deduplication inside `score`, five hundred copies of one word
        // cast five hundred votes and the result saturates to a hard 1.0,
        // whatever the prior says.
        let model = trained();
        assert_eq!(score(&model, &vec![1; 500], 0.01), score(&model, &[1], 0.01));
    }

    // ── Weighting and training ────────────────────────────────────────────

    #[test]
    fn user_corrections_outweigh_derived_labels() {
        // One deliberate correction should count for more than one guess made
        // from the provider's spam folder.
        let mut samples: Vec<Sample> = (0..60)
            .flat_map(|_| [sample(&[1], true), sample(&[2], false)])
            .collect();
        samples.push(Sample {
            features: vec![7],
            positive: false,
            weight: 5,
        });
        let model = train(&samples);
        assert_eq!(model.neg[7], 5);
        assert_eq!(model.n_neg, 65, "the weight counts toward the class total too");
    }

    #[test]
    fn phishing_cannot_be_trained() {
        // A design rule made unrepresentable: a mailbox yields a handful of
        // phishing examples at best, and a model built on that emits noise.
        assert_eq!(ModelAxis::parse("phishing"), None);
        assert_eq!(ModelAxis::ALL.len(), 2);
    }

    // ── Serialization ─────────────────────────────────────────────────────

    #[test]
    fn a_model_survives_a_blob_round_trip() {
        let model = trained();
        let blob = to_blob(&model);
        let restored = from_blob(&blob, model.n_pos, model.n_neg).expect("round trip");
        assert_eq!(restored, model);
    }

    #[test]
    fn a_blob_of_the_wrong_size_is_rejected_rather_than_reinterpreted() {
        // A bucket-count change makes old blobs meaningless, not merely stale.
        // Reading one anyway would silently score against garbage.
        assert!(from_blob(&[0u8; 16], 100, 100).is_none());
    }
}
