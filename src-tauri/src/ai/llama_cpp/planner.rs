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

/// Lower bound on the context window. A window below this leaves no room for a
/// useful RAG prompt + generation reserve, so we floor every model (and every
/// user override) here.
pub(crate) const N_CTX_FLOOR: u32 = 1024;

/// Default upper bound on the context window when the user has NOT set an
/// explicit override. Even on models trained for 32k+ tokens, prompt-eval time
/// on M1 CPU/Metal scales roughly linearly with the number of attended tokens,
/// and the KV allocation scales with n_ctx — 8k is plenty for the RAG chat
/// pipeline and keeps the resident cache bounded by default. A user who needs
/// more can override it up to the model's own trained context (see
/// [`effective_n_ctx`]); this is only the auto/default ceiling, not a hard cap.
pub(crate) const DEFAULT_N_CTX_CAP: u32 = 8192;

/// RAM-aware cap for the AUTOMATIC context window (`override_ctx == 0`).
///
/// Starts from the machine's RAM tier (`util::system::auto_n_ctx_tier`:
/// 8192 / 16384 / 32768, hard-capped at 32k — larger windows stay opt-in via
/// the `chat.n_ctx` preference) and shrinks it when the KV buffer for the
/// tier would not fit next to the model weights inside the GPU working set
/// (~2/3 of unified RAM on Apple Silicon, minus ~1.5 GiB of compute/embed/app
/// overhead). Never returns below [`DEFAULT_N_CTX_CAP`] — if even that KV
/// buffer doesn't fit, behaviour degrades exactly like today's fixed default.
///
/// `kv_bytes_per_token == 0` (unknown model geometry) skips the fit check.
pub(crate) fn plan_auto_n_ctx_cap(total_ram_bytes: Option<u64>, model_bytes: u64, kv_bytes_per_token: u64) -> u32 {
    let tier = crate::util::system::auto_n_ctx_tier(total_ram_bytes);
    let Some(ram) = total_ram_bytes else {
        return tier;
    };
    if kv_bytes_per_token == 0 {
        return tier;
    }
    // GPU working set ≈ 2/3 of unified RAM on Apple Silicon; reserve ~1.5 GiB
    // for compute graphs, the embed model, and the app itself.
    const OVERHEAD_BYTES: u64 = 3 * 1024 * 1024 * 1024 / 2;
    let usable = ram / 3 * 2;
    let kv_budget = usable.saturating_sub(model_bytes).saturating_sub(OVERHEAD_BYTES);
    // Round DOWN to a 1024 boundary so the allocation stays inside the budget.
    let fits = u32::try_from(kv_budget / kv_bytes_per_token).unwrap_or(u32::MAX) / 1024 * 1024;
    // Never sink below the historical fixed default: if even 8k of KV doesn't
    // fit next to the weights, the model is oversized for the machine anyway
    // and a smaller window would only cripple prompts further.
    tier.min(fits).max(DEFAULT_N_CTX_CAP)
}

/// When a prompt had to be front-truncated, work out the context window worth
/// suggesting to the user: the smallest 1024-multiple that would have fit
/// `orig_prompt_tokens + reserve`. Returns `None` when there is nothing
/// actionable — the target isn't larger than the current window, or the model
/// wasn't trained for it (raising past `n_ctx_train` degrades quality).
pub(crate) fn plan_n_ctx_suggestion(
    n_ctx: usize,
    n_ctx_train: u32,
    orig_prompt_tokens: usize,
    reserve: usize,
) -> Option<u32> {
    let needed = orig_prompt_tokens.checked_add(reserve)?;
    if needed <= n_ctx {
        return None; // fits already — the truncation had another cause
    }
    let target = u32::try_from(needed.div_ceil(1024).checked_mul(1024)?).ok()?;
    if target > n_ctx_train {
        return None; // raising past the trained context degrades quality
    }
    Some(target)
}

/// Resolve the context window the actor should create its `LlamaContext` with.
///
/// - `override_ctx == 0` (unset / "auto"): the model's trained context clamped
///   into `[N_CTX_FLOOR, auto_cap]`, where `auto_cap` comes from
///   [`plan_auto_n_ctx_cap`] (RAM-aware, ≥ [`DEFAULT_N_CTX_CAP`], ≤ 32k).
/// - `override_ctx > 0` honours the user's explicit choice, clamped into
///   `[N_CTX_FLOOR, n_ctx_train]` — it may exceed `auto_cap`, but never the
///   window the model was actually trained for (going past that degrades
///   quality via RoPE over-extension and wastes KV memory).
pub(crate) fn effective_n_ctx(override_ctx: u32, n_ctx_train: u32, auto_cap: u32) -> u32 {
    // The model's own ceiling can never go below the floor, even for a model
    // reporting a tiny trained context — the floor wins so the context is
    // always usable.
    let model_cap = n_ctx_train.max(N_CTX_FLOOR);
    if override_ctx == 0 {
        n_ctx_train.clamp(N_CTX_FLOOR, auto_cap.max(N_CTX_FLOOR))
    } else {
        override_ctx.clamp(N_CTX_FLOOR, model_cap)
    }
}

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

/// What to do with the working prompt sequence (seq 0) for a `cache_prompt`
/// request, given its current token mirror, the system-anchor mirror (seq 2),
/// and the new prompt's tokens. The anchor lets a DIVERGING prompt still reuse
/// the invariant system prefix without a partial mid-sequence eviction (which
/// hybrid-attention caches reject) — we fully evict seq 0 and rebuild its head
/// by copying the anchor, then decode only the divergent suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefixPlan {
    /// `new` purely extends the resident seq-0 prefix: keep seq 0, decode
    /// tokens `[reuse..]` on it. No eviction (the within-conversation case).
    Extend { reuse: usize },
    /// `new` diverges from seq 0 but the system anchor (seq 2) is a prefix of
    /// it: fully evict seq 0, copy the anchor back into seq 0, then decode
    /// `[reuse..]`. `reuse` == the anchor length (the new-conversation case).
    RestartFromAnchor { reuse: usize },
    /// No reuse possible (anchor absent or itself diverged): full re-prefill.
    ColdPrefill,
}

/// Decide the seq-0 strategy for a `cache_prompt` request.
///
/// Pure extension wins first (it reuses the MOST, including within-turn
/// history); only on divergence do we fall back to the system anchor, and only
/// to a cold prefill when even the anchor's system prefix no longer matches
/// (e.g. a route flip that re-renders the system message).
pub(crate) fn plan_cached_prefix(
    cached: &[LlamaToken],
    cached_system: &[LlamaToken],
    new: &[LlamaToken],
) -> PrefixPlan {
    let reuse = plan_prefix_reuse(cached, new);
    if reuse == cached.len() {
        // All resident seq-0 tokens matched and at least one new token remains
        // to decode (the cap guarantees the latter) — a pure suffix-append.
        return PrefixPlan::Extend { reuse };
    }
    let anchor = plan_prefix_reuse(cached_system, new);
    if !cached_system.is_empty() && anchor == cached_system.len() {
        return PrefixPlan::RestartFromAnchor { reuse: anchor };
    }
    PrefixPlan::ColdPrefill
}

/// Whether applying `plan` to seq 0 leaves the system anchor (seq 2) sharing its
/// cells with seq 0. The anchor overlaps seq 0 only while seq 0 still holds the
/// system prefix: `Extend` keeps it, `RestartFromAnchor` rebuilds from it. On
/// `ColdPrefill` seq 0 is re-prefilled with content that diverges at position 0,
/// so on a unified KV cache the anchor's cells become DISJOINT and count against
/// `n_ctx` — a long cold prompt then overflows with `NoKvCacheSlot`. The anchor
/// is also stale in that case (the system prefix itself diverged), so the actor
/// drops it before the cold prefill and reseeds it afterwards.
pub(crate) fn anchor_shares_cells_with_seq0(plan: PrefixPlan) -> bool {
    !matches!(plan, PrefixPlan::ColdPrefill)
}

/// Whether the seq-2 system anchor must be (re)seeded for this prompt, and the
/// token length of the system prefix it should hold.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AnchorSeed {
    /// Copy the system prefix `[0, sys_len)` from seq 0 onto seq 2. `false`
    /// when the anchor already holds exactly this system prefix.
    pub reseed: bool,
    /// Token length of the system prefix (clamped to the prompt length).
    pub sys_len: usize,
}

/// Plan system-anchor maintenance: reseed when the anchor is empty or holds a
/// different system prefix than the current prompt (the route-flip case), so
/// the never-evicted anchor always mirrors the live system message.
pub(crate) fn plan_anchor_seed(cached_system: &[LlamaToken], new: &[LlamaToken], sys_len: usize) -> AnchorSeed {
    let sys_len = sys_len.min(new.len());
    let matches = cached_system.len() == sys_len
        && cached_system.iter().zip(new.iter()).take_while(|(a, b)| a == b).count() == sys_len;
    AnchorSeed {
        reseed: !matches,
        sys_len,
    }
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
    fn effective_n_ctx_table() {
        // (override_ctx, n_ctx_train, auto_cap, expected, label)
        let cases: &[(u32, u32, u32, u32, &str)] = &[
            (0, 32768, 8192, 8192, "auto: large model clamps down to the auto cap"),
            (0, 32768, 16384, 16384, "auto: 16GB-tier auto cap unlocks 16k"),
            (0, 32768, 32768, 32768, "auto: 24GB-tier auto cap unlocks 32k"),
            (0, 4096, 8192, 4096, "auto: small model uses its own trained context"),
            (0, 4096, 32768, 4096, "auto: big auto cap never exceeds trained context"),
            (0, 512, 8192, 1024, "auto: tiny model floored to N_CTX_FLOOR"),
            (16384, 32768, 8192, 16384, "override above the auto cap is honoured"),
            (
                40000,
                32768,
                8192,
                32768,
                "override above trained context clamps to the model cap",
            ),
            (
                200,
                32768,
                8192,
                1024,
                "override below the floor clamps up to N_CTX_FLOOR",
            ),
            (
                8192,
                8192,
                8192,
                8192,
                "override equal to trained context passes through",
            ),
            (4096, 512, 8192, 1024, "override on a tiny model floored to N_CTX_FLOOR"),
        ];
        for &(override_ctx, train, auto_cap, expected, label) in cases {
            assert_eq!(effective_n_ctx(override_ctx, train, auto_cap), expected, "{label}");
        }
    }

    #[test]
    fn auto_n_ctx_cap_table() {
        const GIB: u64 = 1024 * 1024 * 1024;
        // KV per token for the qwen3.5-4b/9b class (36 layers × 2 × 8 kv-heads
        // × 128 head-dim × 2 bytes f16) ≈ 144 KiB.
        const KV_9B: u64 = 147_456;
        // (total_ram, model_bytes, kv_per_token, expected, label)
        let cases: &[(Option<u64>, u64, u64, u32, &str)] = &[
            (None, 3 * GIB, KV_9B, 8192, "unknown RAM → conservative baseline"),
            (Some(8 * GIB), 3 * GIB, KV_9B, 8192, "8GB machine stays at 8k"),
            (
                Some(16 * GIB),
                27 * GIB / 10,
                KV_9B,
                16384,
                "16GB + 4b-q4 weights: 16k KV (~2.3GB) fits the working set",
            ),
            (
                Some(16 * GIB),
                54 * GIB / 10,
                KV_9B,
                16384,
                "16GB + 9b-q4 weights: 16k still fits (~3.7GB KV budget)",
            ),
            (
                Some(24 * GIB),
                54 * GIB / 10,
                KV_9B,
                32768,
                "24GB + 9b-q4: 32k KV (~4.6GB) fits",
            ),
            (
                Some(32 * GIB),
                168 * GIB / 10,
                265_000,
                12288,
                "32GB + 27b-q4 weights: tier 32k shrinks to what the KV budget allows",
            ),
            (
                Some(24 * GIB),
                21 * GIB,
                265_000,
                8192,
                "weights alone blow the working set: floor at the 8k baseline, never below",
            ),
            (
                Some(64 * GIB),
                54 * GIB / 10,
                KV_9B,
                32768,
                "big RAM never exceeds the 32k auto cap — larger is user opt-in",
            ),
            (
                Some(16 * GIB),
                54 * GIB / 10,
                0,
                16384,
                "unknown model geometry skips the fit check and trusts the tier",
            ),
        ];
        for (ram, weights, kv, want, label) in cases {
            assert_eq!(plan_auto_n_ctx_cap(*ram, *weights, *kv), *want, "{label}");
        }
    }

    #[test]
    fn n_ctx_suggestion_table() {
        // (n_ctx, n_ctx_train, orig_prompt_tokens, reserve, expected, label)
        type Case<'a> = (usize, u32, usize, usize, Option<u32>, &'a str);
        let cases: &[Case] = &[
            (
                8192,
                32768,
                7592,
                1024,
                Some(9216),
                "truncated 7.6k prompt + 1k reserve → suggest the next 1024 step that fits",
            ),
            (
                8192,
                8192,
                9000,
                1024,
                None,
                "model already at its trained max: nothing to suggest",
            ),
            (
                16384,
                32768,
                12000,
                1024,
                None,
                "prompt fits the current window: nothing to suggest",
            ),
            (
                8192,
                32768,
                40000,
                1024,
                None,
                "needed window exceeds the trained context: nothing actionable",
            ),
            (
                8192,
                32768,
                8192,
                0,
                None,
                "prompt+reserve exactly fills the window: not truncated, nothing to suggest",
            ),
            (
                8192,
                32768,
                8193,
                0,
                Some(9216),
                "one token over the window rounds up to the next 1024 step",
            ),
        ];
        for (n_ctx, train, prompt, reserve, want, label) in cases {
            assert_eq!(
                plan_n_ctx_suggestion(*n_ctx, *train, *prompt, *reserve),
                *want,
                "{label}"
            );
        }
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
    fn cached_prefix_table() {
        // (cached, cached_system, new, expected, label)
        type Case<'a> = (&'a [i32], &'a [i32], &'a [i32], PrefixPlan, &'a str);
        let cases: &[Case] = &[
            (
                &[1, 2, 3],
                &[1, 2],
                &[1, 2, 3, 4, 5],
                PrefixPlan::Extend { reuse: 3 },
                "pure extension reuses the whole seq-0 prefix (within-conversation)",
            ),
            (
                &[],
                &[],
                &[1, 2, 3],
                PrefixPlan::Extend { reuse: 0 },
                "first ever call: empty seq 0 'extends' from nothing",
            ),
            (
                &[1, 2, 8, 9],
                &[1, 2],
                &[1, 2, 3, 4, 5],
                PrefixPlan::RestartFromAnchor { reuse: 2 },
                "diverges after the system prefix: restart from the anchor (new conversation)",
            ),
            (
                &[1, 2, 8, 9],
                &[1, 2],
                &[1, 2],
                PrefixPlan::ColdPrefill,
                "anchor equals the whole new prompt: no token left to decode, cannot reuse",
            ),
            (
                &[7, 7, 7],
                &[7, 7],
                &[1, 2, 3, 4],
                PrefixPlan::ColdPrefill,
                "system prefix itself diverged (route flip): cold",
            ),
            (
                &[1, 2, 8, 9],
                &[],
                &[1, 2, 3, 4],
                PrefixPlan::ColdPrefill,
                "no anchor seeded yet and seq 0 diverges: cold",
            ),
            (
                &[1, 2, 3, 4],
                &[1, 2],
                &[1, 2, 3],
                PrefixPlan::RestartFromAnchor { reuse: 2 },
                "new is a strict prefix of seq 0 (shorter): not an extension, fall to anchor",
            ),
        ];
        for (cached, cached_system, new, want, label) in cases {
            let got = plan_cached_prefix(&toks(cached), &toks(cached_system), &toks(new));
            assert_eq!(&got, want, "{label}");
        }
    }

    #[test]
    fn anchor_shares_cells_table() {
        // The never-evicted system anchor (seq 2) only overlaps seq 0's cells
        // while seq 0 still holds the system prefix — true for Extend (keeps it)
        // and RestartFromAnchor (rebuilds it by copying the anchor). On
        // ColdPrefill seq 0 is re-prefilled with content that diverges at
        // position 0, so on a unified KV cache the anchor's cells become
        // disjoint and count against n_ctx: a long cold prompt overflows with
        // NoKvCacheSlot. The actor must therefore drop the (now-stale) anchor
        // before a cold prefill and reseed it afterward.
        assert!(anchor_shares_cells_with_seq0(PrefixPlan::Extend { reuse: 5 }));
        assert!(anchor_shares_cells_with_seq0(PrefixPlan::Extend { reuse: 0 }));
        assert!(anchor_shares_cells_with_seq0(PrefixPlan::RestartFromAnchor {
            reuse: 3
        }));
        assert!(!anchor_shares_cells_with_seq0(PrefixPlan::ColdPrefill));
    }

    #[test]
    fn anchor_seed_table() {
        // (cached_system, new, sys_len, expected, label)
        type Case<'a> = (&'a [i32], &'a [i32], usize, AnchorSeed, &'a str);
        let cases: &[Case] = &[
            (
                &[],
                &[1, 2, 3, 4],
                2,
                AnchorSeed {
                    reseed: true,
                    sys_len: 2,
                },
                "empty anchor: seed it with the system prefix",
            ),
            (
                &[1, 2],
                &[1, 2, 3, 4],
                2,
                AnchorSeed {
                    reseed: false,
                    sys_len: 2,
                },
                "anchor already holds this system prefix: no-op",
            ),
            (
                &[9, 9],
                &[1, 2, 3, 4],
                2,
                AnchorSeed {
                    reseed: true,
                    sys_len: 2,
                },
                "anchor holds a different system prefix (route flip): reseed",
            ),
            (
                &[1, 2],
                &[1, 2, 3, 4],
                3,
                AnchorSeed {
                    reseed: true,
                    sys_len: 3,
                },
                "system prefix grew: anchor too short, reseed",
            ),
            (
                &[1, 2, 3, 4],
                &[1, 2],
                3,
                AnchorSeed {
                    reseed: true,
                    sys_len: 2,
                },
                "sys_len clamped to the (shorter) prompt length",
            ),
        ];
        for (cached_system, new, sys_len, want, label) in cases {
            let got = plan_anchor_seed(&toks(cached_system), &toks(new), *sys_len);
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
        type Case<'a> = (&'a [i32], &'a [i32], usize, usize, &'a str);
        let cases: &[Case] = &[
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
                2500,
                0,
                8192,
                PromptBudget {
                    drop_front: 0,
                    max_gen: 0,
                },
                "prefill-only (max_tokens=0): decode the prompt, sample nothing — the prewarm contract",
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
