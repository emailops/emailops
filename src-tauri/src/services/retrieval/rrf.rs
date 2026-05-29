//! Reciprocal Rank Fusion (RRF).
//!
//! Given N ranked lists of `email_id`s, compute a fused score for each id as
//! `sum over lists of weight / (k + rank + 1)`. Higher fused score = better.
//!
//! The caller decides the per-list weight: equal-weight (1.0) for the
//! plain agent-search hybrid; tuned weights (e.g. 0.55/0.50) for chat/search.
//! Top-FTS boosts, recency bonuses, and accent-normalized scorers stay in the
//! consumer because they are not RRF.

use std::collections::HashMap;

pub const DEFAULT_RRF_K: f32 = 60.0;

/// One ranked list of email ids to be fused. `weight` scales the contribution
/// of this list to the fused score. `ids_in_order` must already be sorted by
/// the producer (best first).
pub struct Ranking<'a> {
    pub ids_in_order: &'a [String],
    pub weight: f32,
}

/// Fuse the rankings using Reciprocal Rank Fusion. Returns `(email_id, score)`
/// sorted by score descending. Duplicate ids across rankings have their
/// contributions summed (this is the entire point of RRF).
pub fn fuse_rrf(rankings: &[Ranking<'_>], k: f32) -> Vec<(String, f32)> {
    let mut fused: HashMap<String, f32> = HashMap::new();
    for ranking in rankings {
        for (rank, id) in ranking.ids_in_order.iter().enumerate() {
            let s = ranking.weight / (k + rank as f32 + 1.0);
            *fused.entry(id.clone()).or_insert(0.0) += s;
        }
    }
    let mut out: Vec<(String, f32)> = fused.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Tiebreak on id so the ranking is deterministic when scores are
            // identical (otherwise HashMap iteration order leaks into output).
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fuse_rrf_single_list_preserves_order() {
        let a = ids(&["a", "b", "c"]);
        let out = fuse_rrf(
            &[Ranking {
                ids_in_order: &a,
                weight: 1.0,
            }],
            DEFAULT_RRF_K,
        );
        let order: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn fuse_rrf_combines_two_rankings() {
        // "b" appears in both lists — it should beat "a" (only list 1) and
        // "c" (only list 2), even though both rank #1 in their own lists.
        let a = ids(&["a", "b"]);
        let b = ids(&["c", "b"]);
        let out = fuse_rrf(
            &[
                Ranking {
                    ids_in_order: &a,
                    weight: 1.0,
                },
                Ranking {
                    ids_in_order: &b,
                    weight: 1.0,
                },
            ],
            DEFAULT_RRF_K,
        );
        assert_eq!(out[0].0, "b");
    }

    #[test]
    fn fuse_rrf_weights_affect_order() {
        // Two disjoint lists at the same top rank. The heavier list (10x)
        // pushes its #1 item above the lighter list's #1.
        let a = ids(&["a"]);
        let b = ids(&["b"]);
        let out = fuse_rrf(
            &[
                Ranking {
                    ids_in_order: &a,
                    weight: 10.0,
                }, // a wins
                Ranking {
                    ids_in_order: &b,
                    weight: 1.0,
                },
            ],
            DEFAULT_RRF_K,
        );
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
    }

    #[test]
    fn fuse_rrf_handles_disjoint_ids() {
        let a = ids(&["a", "b"]);
        let b = ids(&["c", "d"]);
        let out = fuse_rrf(
            &[
                Ranking {
                    ids_in_order: &a,
                    weight: 1.0,
                },
                Ranking {
                    ids_in_order: &b,
                    weight: 1.0,
                },
            ],
            DEFAULT_RRF_K,
        );
        // All four show up; a and c are tied at rank 1 (1/61), so sort is
        // deterministic by id ascending → a, c, then b, d at rank 2 (1/62).
        let order: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(order, vec!["a", "c", "b", "d"]);
    }

    #[test]
    fn fuse_rrf_empty_input_yields_empty_output() {
        let out = fuse_rrf(&[], DEFAULT_RRF_K);
        assert!(out.is_empty());
    }
}
