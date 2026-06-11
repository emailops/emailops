// Pure planning functions for KV-prefix reuse (no I/O).
//
// The inference actor is the thin executor that applies these plans against
// the real `LlamaContext`; everything decision-shaped lives here so it can be
// unit-tested exhaustively without loading a model.

use llama_cpp_2::token::LlamaToken;

/// Generation headroom (in tokens) that is always preserved inside the context
/// window before the prompt gets tail-truncated. Cutting the prompt shifts
/// every following token to a new position, which invalidates the whole KV
/// prefix — so we prefer shrinking the generation budget down to this floor
/// over cutting the prompt.
pub(crate) const GEN_RESERVE_TOKENS: usize = 1024;

/// How many leading tokens of `new` can reuse KV entries computed for
/// `cached` (the token sequence currently materialised in the context).
///
/// Returns the longest common prefix, capped at `new.len() - 1` so at least
/// one token is always decoded — llama.cpp needs a fresh decode to produce
/// logits for sampling the first generated token.
pub(crate) fn plan_prefix_reuse(cached: &[LlamaToken], new: &[LlamaToken]) -> usize {
    if new.is_empty() {
        return 0;
    }
    let lcp = cached.iter().zip(new.iter()).take_while(|(a, b)| a == b).count();
    lcp.min(new.len() - 1)
}

/// How many leading tokens of `full` (the rendered prompt) are STABLE — i.e.
/// will re-appear verbatim at the same positions in the next turn's prompt —
/// and may therefore be persisted in the seq-0 prompt cache.
///
/// `stable` is the tokenisation of the prompt rendered WITHOUT the generation
/// header (`add_generation_prompt=false`). The header tail (e.g. Qwen's
/// `<|im_start|>assistant\n<think>\n\n</think>\n\n` under
/// `enable_thinking=false`) is re-rendered as the assistant HISTORY message in
/// the next turn, so caching it leaves a stale suffix that hybrid-attention
/// caches cannot partially evict — which collapses reuse to zero.
///
/// Token-level LCP (not byte length) absorbs tokenisation merges at the seam.
/// The result never drops below `lcp` (those positions already match the
/// resident cache) and never exceeds `full.len()`.
pub(crate) fn plan_stable_boundary(full: &[LlamaToken], stable: &[LlamaToken], lcp: usize) -> usize {
    let common = full.iter().zip(stable.iter()).take_while(|(a, b)| a == b).count();
    common.max(lcp).min(full.len())
}

/// How to fit a prompt plus generation into a fixed `n_ctx` window.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PromptBudget {
    /// Tokens to drop from the FRONT of the prompt (tail bias: keep the most
    /// recent content). 0 in the common case — see `GEN_RESERVE_TOKENS`.
    pub drop_front: usize,
    /// Effective generation budget after fitting the prompt.
    pub max_gen: usize,
}

/// Plan the prompt/generation split for a fixed context window.
///
/// Priority order: (1) keep the full prompt and the full `max_tokens` if both
/// fit; (2) keep the full prompt and shrink generation, but never below
/// `min(max_tokens, GEN_RESERVE_TOKENS)`; (3) only then tail-truncate the
/// prompt (which sacrifices KV-prefix reuse for that call).
pub(crate) fn plan_prompt_budget(prompt_len: usize, max_tokens: usize, n_ctx: usize) -> PromptBudget {
    if prompt_len + max_tokens <= n_ctx {
        return PromptBudget {
            drop_front: 0,
            max_gen: max_tokens,
        };
    }
    // The reserve never exceeds half the window, so a small n_ctx still keeps
    // most of the prompt instead of truncating it to make room for headroom.
    let reserve = max_tokens.min(GEN_RESERVE_TOKENS).min(n_ctx / 2).max(1);
    if prompt_len + reserve <= n_ctx {
        return PromptBudget {
            drop_front: 0,
            max_gen: n_ctx - prompt_len,
        };
    }
    let keep = n_ctx - reserve;
    PromptBudget {
        drop_front: prompt_len - keep,
        max_gen: reserve,
    }
}

/// How to fit an UNCACHED (one-shot) prompt next to the resident seq-0
/// prompt prefix, which we want to keep warm for the next chat turn.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UncachedPlan {
    /// Evict the resident seq-0 prefix first. Only when the one-shot prompt
    /// cannot fit in the cells left over — correctness beats cache warmth.
    pub evict_resident: bool,
    /// Prompt/generation split for the cell budget that remains.
    pub budget: PromptBudget,
}

/// Plan a one-shot (non-cached) request: keep the resident seq-0 prefix if
/// the prompt + a generation reserve fit in the remaining cells, otherwise
/// sacrifice the resident prefix and plan against the full window.
pub(crate) fn plan_uncached_budget(
    prompt_len: usize,
    max_tokens: usize,
    n_ctx: usize,
    resident: usize,
) -> UncachedPlan {
    let avail = n_ctx.saturating_sub(resident);
    let beside = plan_prompt_budget(prompt_len, max_tokens, avail.max(1));
    if avail > 0 && beside.drop_front == 0 {
        return UncachedPlan {
            evict_resident: false,
            budget: beside,
        };
    }
    UncachedPlan {
        evict_resident: true,
        budget: plan_prompt_budget(prompt_len, max_tokens, n_ctx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(ids: &[i32]) -> Vec<LlamaToken> {
        ids.iter().map(|&i| LlamaToken(i)).collect()
    }

    #[test]
    fn uncached_budget_table() {
        // (prompt_len, max_tokens, n_ctx, resident, expected, label)
        let cases: &[(usize, usize, usize, usize, UncachedPlan, &str)] = &[
            (
                1100,
                2048,
                8192,
                3000,
                UncachedPlan {
                    evict_resident: false,
                    budget: PromptBudget {
                        drop_front: 0,
                        max_gen: 2048,
                    },
                },
                "aux prompt fits alongside the resident chat prefix",
            ),
            (
                1500,
                2048,
                8192,
                5500,
                UncachedPlan {
                    evict_resident: false,
                    budget: PromptBudget {
                        drop_front: 0,
                        max_gen: 1192,
                    },
                },
                "fits only by shrinking generation: keep the resident prefix",
            ),
            (
                1500,
                2048,
                8192,
                7000,
                UncachedPlan {
                    evict_resident: true,
                    budget: PromptBudget {
                        drop_front: 0,
                        max_gen: 2048,
                    },
                },
                "cannot fit beside the resident prefix: evict it, then plan on the full window",
            ),
            (
                1100,
                2048,
                8192,
                0,
                UncachedPlan {
                    evict_resident: false,
                    budget: PromptBudget {
                        drop_front: 0,
                        max_gen: 2048,
                    },
                },
                "no resident prefix: same as the plain budget",
            ),
            (
                8500,
                2048,
                8192,
                3000,
                UncachedPlan {
                    evict_resident: true,
                    budget: PromptBudget {
                        drop_front: 1332,
                        max_gen: 1024,
                    },
                },
                "prompt larger than the window even after eviction: tail-truncate",
            ),
        ];
        for (prompt_len, max_tokens, n_ctx, resident, want, label) in cases {
            let got = plan_uncached_budget(*prompt_len, *max_tokens, *n_ctx, *resident);
            assert_eq!(&got, want, "{label}");
        }
    }

    #[test]
    fn prefix_reuse_table() {
        // (cached, new, expected_lcp, label)
        let cases: &[(&[i32], &[i32], usize, &str)] = &[
            (&[], &[1, 2, 3], 0, "empty cache reuses nothing"),
            (&[1, 2, 3], &[], 0, "empty new prompt reuses nothing"),
            (&[], &[], 0, "both empty"),
            (&[9, 8, 7], &[1, 2, 3], 0, "disjoint sequences"),
            (&[1, 2, 9, 9], &[1, 2, 3, 4], 2, "partial shared prefix"),
            (&[1, 2, 3, 4], &[1, 2, 3, 4], 3, "identical: cap at len-1 for logits"),
            (&[1], &[1], 0, "identical single token: still decode it"),
            (
                &[1, 2, 3, 4, 5],
                &[1, 2, 3],
                2,
                "new is strict prefix of cached: cap at len-1",
            ),
            (&[1, 2, 3], &[1, 2, 3, 4, 5], 3, "cached is strict prefix of new"),
            (&[2, 2, 3], &[1, 2, 3], 0, "first token differs"),
        ];
        for (cached, new, want, label) in cases {
            let got = plan_prefix_reuse(&toks(cached), &toks(new));
            assert_eq!(got, *want, "{label}");
        }
    }

    #[test]
    fn stable_boundary_table() {
        // (full, stable, lcp, expected, label)
        let cases: &[(&[i32], &[i32], usize, usize, &str)] = &[
            (
                &[1, 2, 3, 4, 90, 91, 92, 93],
                &[1, 2, 3, 4],
                0,
                4,
                "generation-header tail stays out of the stable prefix",
            ),
            (
                &[1, 2, 3, 4],
                &[1, 2, 3, 4],
                0,
                4,
                "no volatile tail: whole prompt is stable",
            ),
            (&[1, 2, 3], &[], 2, 2, "empty stable render: never go below lcp"),
            (
                &[1, 2, 3, 4, 5],
                &[9, 9],
                3,
                3,
                "divergent stable render: clamp up to lcp",
            ),
            (
                &[1, 2, 3],
                &[1, 2, 3, 4, 5],
                0,
                3,
                "stable render longer than the prompt: cap at prompt length",
            ),
            (&[], &[], 0, 0, "empty prompt"),
            (
                &[1, 2, 7, 8, 90],
                &[1, 2, 3],
                0,
                2,
                "tokenisation merge at the seam: stable ends at the token-level lcp",
            ),
        ];
        for (full, stable, lcp, want, label) in cases {
            let got = plan_stable_boundary(&toks(full), &toks(stable), *lcp);
            assert_eq!(got, *want, "{label}");
        }
    }

    #[test]
    fn prompt_budget_table() {
        // (prompt_len, max_tokens, n_ctx, expected, label)
        let cases: &[(usize, usize, usize, PromptBudget, &str)] = &[
            (
                1000,
                2048,
                8192,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 2048,
                },
                "everything fits: full budget",
            ),
            (
                4096,
                4096,
                8192,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 4096,
                },
                "exactly fills the window",
            ),
            (
                5000,
                4096,
                8192,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 3192,
                },
                "prompt kept whole, generation shrunk (the tool-round case)",
            ),
            (
                7168,
                4096,
                8192,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 1024,
                },
                "generation shrunk exactly to the reserve floor",
            ),
            (
                7800,
                4096,
                8192,
                PromptBudget {
                    drop_front: 632,
                    max_gen: 1024,
                },
                "prompt over the floor line: tail-truncate, keep reserve",
            ),
            (
                8500,
                1,
                8192,
                PromptBudget {
                    drop_front: 309,
                    max_gen: 1,
                },
                "tiny max_tokens caps the reserve (warmup-style call)",
            ),
            (
                0,
                100,
                8192,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 100,
                },
                "empty prompt",
            ),
            (
                500,
                4096,
                1024,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 524,
                },
                "small window: shrink generation below requested",
            ),
        ];
        for (prompt_len, max_tokens, n_ctx, want, label) in cases {
            let got = plan_prompt_budget(*prompt_len, *max_tokens, *n_ctx);
            assert_eq!(&got, want, "{label}");
        }
    }
}
