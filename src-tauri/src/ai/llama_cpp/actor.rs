// Persistent inference actor — owns one `LlamaContext` and reuses its KV
// cache across requests via longest-common-prefix planning (planner.rs).
//
// WHY A DEDICATED THREAD
// ──────────────────────
// `LlamaContext<'model>` is `!Send` and borrows the model, so it cannot hop
// between `spawn_blocking` workers. A dedicated OS thread owns the
// `Arc<LlamaModel>` plus one fixed-`n_ctx` context for as long as the model
// stays loaded; requests arrive over an mpsc channel and are answered over
// per-request oneshots. Streaming callbacks run on this thread.
//
// KV REUSE — THREE SEQUENCES
// ──────────────────────────
// `cached_tokens` mirrors exactly the PROMPT tokens materialised in sequence
// 0 (the working prompt prefix). Generation happens on sequence 1 (a copy of
// seq 0), dropped wholesale at the start of the next request. Sequence 2 is a
// never-evicted ANCHOR holding only the invariant system prefix.
//
// Why no partial eviction: each tool round's prompt is a strict extension of
// the previous round's PROMPT, but a NEW conversation's prompt only shares the
// system prefix and then diverges. Rolling back a partially-divergent sequence
// needs a partial KV removal, which llama.cpp does not support for
// hybrid-attention / SWA / recurrent caches (clear_kv_cache_seq returns
// Ok(false)) — a failed rollback forces a full multi-second re-prefill. So we
// never evict mid-sequence; instead, per `plan_cached_prefix`:
//   1. evict all of seq 1 (full-sequence removal always succeeds)
//   2. Extend: prompt purely extends seq 0 → keep it (the tool-round case)
//      RestartFromAnchor: prompt diverges but shares the system prefix → fully
//        evict seq 0, copy the seq-2 anchor back into it (the new-conversation
//        case — reuses the system prefix with no partial eviction)
//      ColdPrefill: even the system prefix diverged (route flip) → full evict
//   3. decode the STABLE prompt suffix on seq 0 (see below)
//   4. (re)seed seq 2 from seq 0's system prefix when it changed
//   5. copy seq 0 → seq 1, decode the volatile tail on seq 1, generate on seq 1
//
// VOLATILE TAIL
// ─────────────
// The rendered prompt ends with a generation header that is NOT stable
// across turns: e.g. Qwen under `enable_thinking=false` appends
// `<think>\n\n</think>\n\n` after `<|im_start|>assistant\n`, but the next
// turn re-renders that assistant slot as a history message (header +
// content, no empty think block). Persisting the tail in seq 0 leaves a
// stale suffix the next turn cannot evict on hybrid caches — collapsing
// reuse to zero. So callers pass `stable_prompt_bytes` (the byte length of
// the prompt rendered with `add_generation_prompt=false`) and the tail past
// the stable boundary is decoded on seq 1 only, after the seq-0 copy.
// The cache lives in RAM only and dies with the actor — it derives from
// email plaintext, so it is never persisted to disk.

use std::num::NonZeroU32;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

// `Special` and `token_to_str` are deprecated in llama-cpp-2 — the new
// `token_to_piece` API is more flexible but not yet migrated here.
#[allow(deprecated)]
use llama_cpp_2::model::Special;
use llama_cpp_2::{
    context::{params::LlamaContextParams, LlamaContext},
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel},
    sampling::LlamaSampler,
    token::LlamaToken,
};

use super::planner::{
    anchor_shares_cells_with_seq0, effective_n_ctx, plan_anchor_seed, plan_auto_n_ctx_cap, plan_cached_prefix,
    plan_n_ctx_suggestion, plan_prompt_budget, plan_stable_boundary, plan_uncached_budget, PrefixPlan,
};
use super::runtime::backend;

/// Physical batch size. llama.cpp splits submitted batches into ubatch-sized
/// chunks internally; sizing this to n_ctx makes Metal allocate huge per-graph
/// buffers and is a known cause of fatal `llama_decode` failures on Apple
/// Silicon under memory pressure. 512 is llama.cpp's own default.
const N_UBATCH: u32 = 512;

/// Result of one generation pass.
pub(crate) struct GenOutcome {
    /// Generated text (all sampled pieces concatenated).
    pub text: String,
    /// Prompt tokens in the final (possibly tail-truncated) prompt.
    pub prompt_tokens: u32,
    /// Tokens sampled during generation.
    pub gen_tokens: u32,
    /// Wall-clock ms from tokenisation start until the prompt decode finished,
    /// i.e. the latency before the first token can be sampled.
    pub prefill_ms: i64,
    /// Leading prompt tokens served from the KV cache instead of re-decoded.
    pub cached_prompt_tokens: u32,
    /// Which `PrefixPlan` the actor picked for this call. `None` for one-shot
    /// uncached calls (`cache_prompt=false`); cache_prompt=true calls always
    /// populate this. Stable strings for serialisation:
    /// `"Extend"` | `"RestartFromAnchor"` | `"ColdPrefill"`.
    pub prefix_plan: Option<&'static str>,
    /// Token length of the system anchor (seq 2) BEFORE this call ran. Tells
    /// the reasoning trace whether a populated anchor was wiped mid-call
    /// (`prev_sys_cached > 0 && plan == ColdPrefill`).
    pub sys_cached_before: u32,
    /// Token length of the system anchor AFTER this call ran. May differ from
    /// `sys_cached_before` on ColdPrefill (anchor wiped then reseeded) or when
    /// the system prefix grew/shrank.
    pub sys_cached_after: u32,
    /// Token length of the invariant system prefix used for this call (the
    /// `sys_tok` boundary the actor computed from `system_prefix_bytes`).
    /// `0` when the caller didn't pass `system_prefix_bytes` (one-shots).
    pub system_prefix_tokens: u32,
    /// Token boundary up to which seq-0 holds the stable prompt prefix after
    /// this call. Everything past it was decoded on seq 1 only (volatile gen
    /// header).
    pub stable_tokens: u32,
    /// Tokens dropped from the FRONT of the prompt to fit `n_ctx`. Non-zero
    /// when the prompt exceeded the prompt budget (`n_ctx − generation
    /// reserve`). When this fires, the leading bytes of the prompt change
    /// between turns → cache reuse is impossible across truncated rounds,
    /// regardless of whether the rest of the cache logic is healthy. Surface
    /// it in the trace so a user staring at a wall of cold prefills can tell
    /// "out of context" from "actual cache bug".
    pub dropped_front_tokens: u32,
}

/// Streaming callback: receives each generated piece; return `false` to stop.
pub(crate) type OnToken = Box<dyn FnMut(String) -> bool + Send>;

struct GenRequest {
    prompt: String,
    temperature: f32,
    max_tokens: usize,
    /// `false` for one-shot prompts (rewrite/rerank/classification/warmup):
    /// prefill + generate entirely on seq 1, leaving the seq-0 chat-prompt
    /// prefix warm for the next chat turn. Their next prompt never extends
    /// the previous one, so caching them only evicts what IS reusable.
    cache_prompt: bool,
    /// Byte length of the prompt prefix that is stable across turns (the
    /// render without the generation header). Tokens past it stay out of the
    /// persistent seq-0 cache — see "VOLATILE TAIL" above. `None` caches the
    /// whole prompt.
    stable_prompt_bytes: Option<usize>,
    /// Byte length of the invariant SYSTEM prefix (the system message rendered
    /// alone, with no generation header). Its tokens are pinned on the
    /// never-evicted anchor sequence so a NEW conversation that shares only the
    /// system message can still reuse it. `None` disables anchoring.
    system_prefix_bytes: Option<usize>,
    on_token: Option<OnToken>,
    reply: tokio::sync::oneshot::Sender<std::result::Result<GenOutcome, String>>,
}

/// Cloneable handle to the actor thread. Dropping every handle closes the
/// channel, which makes the thread exit and release the context + model Arc.
#[derive(Clone)]
pub(crate) struct InferenceActorHandle {
    tx: Sender<GenRequest>,
}

impl InferenceActorHandle {
    /// Spawn the actor thread for `model`. The context is created lazily on
    /// the thread itself (it cannot be sent across). `n_ctx_override` is the
    /// user's configured context window (`0` = auto); the actor resolves the
    /// effective window via [`effective_n_ctx`] once the model is known.
    pub(crate) fn spawn(model: Arc<LlamaModel>, n_ctx_override: u32) -> std::result::Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel::<GenRequest>();
        std::thread::Builder::new()
            .name("llama-inference".into())
            .spawn(move || actor_loop(&model, &rx, n_ctx_override))
            .map_err(|e| format!("Failed to spawn inference thread: {}", e))?;
        Ok(Self { tx })
    }

    /// Run one generation pass on the actor thread. `cache_prompt: false`
    /// keeps the request out of the persistent seq-0 prompt cache (one-shot
    /// prompts that would otherwise clobber the reusable chat prefix).
    pub(crate) async fn generate(
        &self,
        prompt: String,
        temperature: f32,
        max_tokens: usize,
        cache_prompt: bool,
        stable_prompt_bytes: Option<usize>,
        system_prefix_bytes: Option<usize>,
        on_token: Option<OnToken>,
    ) -> std::result::Result<GenOutcome, String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(GenRequest {
                prompt,
                temperature,
                max_tokens,
                cache_prompt,
                stable_prompt_bytes,
                system_prefix_bytes,
                on_token,
                reply: reply_tx,
            })
            .map_err(|_| "Inference thread is no longer running".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Inference thread dropped the request".to_string())?
    }
}

fn actor_loop(model: &LlamaModel, rx: &Receiver<GenRequest>, n_ctx_override: u32) {
    // KV bytes per token from the model's real geometry (f16 K+V per layer).
    // Hybrid/SWA layers cap their own KV, so this is a safe upper bound.
    let kv_bytes_per_token = {
        let n_head = u64::from(model.n_head().max(1));
        let head_dim = (model.n_embd().max(0) as u64) / n_head;
        u64::from(model.n_layer()) * 2 * u64::from(model.n_head_kv()) * head_dim * 2
    };
    let auto_cap = plan_auto_n_ctx_cap(crate::util::system::total_ram_bytes(), model.size(), kv_bytes_per_token);
    let n_ctx = effective_n_ctx(n_ctx_override, model.n_ctx_train(), auto_cap);
    crate::services::logger::log(
        "info",
        "ai",
        format!(
            "llamacpp: context window {} tokens (override={}, trained={}, auto_cap={}, kv/token={}B)",
            n_ctx,
            n_ctx_override,
            model.n_ctx_train(),
            auto_cap,
            kv_bytes_per_token,
        ),
    );
    // n_batch = n_ctx so a full-window prompt fits in a single decode call
    // (GGML_ASSERT(n_tokens_all <= cparams.n_batch) trips otherwise). n_ubatch
    // stays pinned at N_UBATCH so a large window doesn't blow up the per-graph
    // Metal buffers — that cap is the real memory mitigation, not n_batch.
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(n_ctx))
        .with_n_batch(n_ctx)
        .with_n_ubatch(n_ctx.min(N_UBATCH))
        // Seq 0 = working prompt prefix, seq 1 = per-request generation,
        // seq 2 = never-evicted system-prefix anchor. Unified buffer: without
        // it llama.cpp splits the n_ctx cell budget per sequence (→
        // NoKvCacheSlot on long prompts). The sequences share cells via
        // tagging, so unified costs nothing.
        .with_n_seq_max(3)
        .with_kv_unified(true);

    let mut ctx = match model.new_context(backend(), ctx_params) {
        Ok(ctx) => ctx,
        Err(e) => {
            let msg = format!("Context creation failed: {}", e);
            while let Ok(req) = rx.recv() {
                let _ = req.reply.send(Err(msg.clone()));
            }
            return;
        }
    };

    let mut cached_tokens: Vec<LlamaToken> = Vec::new();
    // Token mirror of the seq-2 system-prefix anchor (never evicted except on
    // a route flip that re-renders the system message, or a hard error).
    let mut cached_system: Vec<LlamaToken> = Vec::new();
    // The "raise your context window" hint fires at most once per actor
    // lifetime — long multi-turn chats truncate every turn once they overflow
    // and repeating the same advice would spam the output panel.
    let mut n_ctx_suggested = false;
    while let Ok(req) = rx.recv() {
        let GenRequest {
            prompt,
            temperature,
            max_tokens,
            cache_prompt,
            stable_prompt_bytes,
            system_prefix_bytes,
            mut on_token,
            reply,
        } = req;
        let result = generate_with_cache(
            model,
            &mut ctx,
            &mut cached_tokens,
            &mut cached_system,
            &prompt,
            temperature,
            max_tokens,
            cache_prompt,
            stable_prompt_bytes,
            system_prefix_bytes,
            on_token.as_mut(),
            &mut n_ctx_suggested,
        );
        if result.is_err() {
            // The decode state is unknown after a failure — drop everything so
            // the mirrors never disagree with the real KV contents.
            ctx.clear_kv_cache();
            cached_tokens.clear();
            cached_system.clear();
        }
        let _ = reply.send(result);
    }
}

/// One generation pass against the persistent context.
///
/// `cached` is the token mirror of the seq-0 (working prompt-prefix) KV
/// contents and `cached_system` the mirror of the seq-2 system anchor; both are
/// updated in lockstep with every successful KV mutation, so they stay accurate
/// even when the pass fails midway (the caller still resets on error for
/// defence in depth).
#[allow(clippy::too_many_arguments)]
fn generate_with_cache(
    model: &LlamaModel,
    ctx: &mut LlamaContext,
    cached: &mut Vec<LlamaToken>,
    cached_system: &mut Vec<LlamaToken>,
    prompt: &str,
    temperature: f32,
    max_tokens: usize,
    cache_prompt: bool,
    stable_prompt_bytes: Option<usize>,
    system_prefix_bytes: Option<usize>,
    mut on_token: Option<&mut OnToken>,
    n_ctx_suggested: &mut bool,
) -> std::result::Result<GenOutcome, String> {
    // Prefill clock starts before tokenisation: everything up to the first
    // sampled token is latency the user perceives as "thinking".
    let t_prefill = std::time::Instant::now();
    // Snapshot the cache mirrors BEFORE any in-call mutation so the kv:
    // log line can show an explicit BEFORE/AFTER transition (the visualizer
    // in `tools/kv_viz/` reads these to render the per-call cache strip).
    let prev_sys_cached = cached_system.len();
    let mut tokens = model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|e| format!("Tokenisation failed: {}", e))?;

    if tokens.is_empty() {
        return Ok(GenOutcome {
            text: String::new(),
            prompt_tokens: 0,
            gen_tokens: 0,
            prefill_ms: 0,
            cached_prompt_tokens: 0,
            prefix_plan: None,
            sys_cached_before: prev_sys_cached as u32,
            sys_cached_after: prev_sys_cached as u32,
            system_prefix_tokens: 0,
            stable_tokens: 0,
            dropped_front_tokens: 0,
        });
    }

    let n_ctx = ctx.n_ctx() as usize;

    // Drop the previous request's generation sequence. Full-sequence
    // removals always succeed, including on hybrid/SWA/recurrent caches.
    ctx.clear_kv_cache_seq(Some(1), None, None)
        .map_err(|e| format!("KV generation-seq eviction failed: {}", e))?;

    let max_gen;
    let lcp;
    let stable_tok;
    // Token boundary of the invariant system prefix within `tokens`; 0 disables
    // anchoring for this pass (one-shots, dropped-front prompts, no hint).
    let sys_tok;
    let prefill_seq: i32;
    // Name of the PrefixPlan the actor picked for this call — flows back into
    // the chat reasoning trace so the UI can show "ColdPrefill 🔥 wiped"
    // etc. instead of just a cached-token count. None for cache_prompt=false.
    let plan_name: Option<&'static str>;
    // Tokens dropped from the front of the prompt to fit n_ctx (0 if no
    // truncation was needed). Reported back via GenOutcome so the trace can
    // distinguish "cold because of context overflow" from "cold because of
    // anchor / plan failures".
    let mut dropped_front: usize = 0;
    if cache_prompt {
        let budget = plan_prompt_budget(tokens.len(), max_tokens, n_ctx);
        if budget.drop_front > 0 {
            tokens.drain(0..budget.drop_front);
            dropped_front = budget.drop_front;
        }
        max_gen = budget.max_gen;

        // Decide the seq-0 strategy WITHOUT any partial mid-sequence eviction
        // (unsupported on hybrid caches): extend the resident prefix, restart
        // from the system anchor, or cold-prefill — see `plan_cached_prefix`.
        let prev_cached = cached.len();
        let plan = plan_cached_prefix(cached, cached_system, &tokens);
        plan_name = Some(match plan {
            PrefixPlan::Extend { .. } => "Extend",
            PrefixPlan::RestartFromAnchor { .. } => "RestartFromAnchor",
            PrefixPlan::ColdPrefill => "ColdPrefill",
        });
        match plan {
            PrefixPlan::Extend { reuse } => {
                // seq 0 already holds [0, reuse); keep it as-is.
                cached.truncate(reuse);
                lcp = reuse;
            }
            PrefixPlan::RestartFromAnchor { reuse } => {
                // Fully evict seq 0 (always succeeds), then rebuild its head by
                // copying the system anchor (seq 2) back in — system-prefix
                // reuse with no partial eviction.
                ctx.clear_kv_cache_seq(Some(0), None, None)
                    .map_err(|e| format!("KV seq-0 eviction failed: {}", e))?;
                ctx.copy_kv_cache_seq(2, 0, None, Some(reuse as u32))
                    .map_err(|e| format!("KV anchor→seq-0 copy failed: {}", e))?;
                cached.clear();
                cached.extend_from_slice(&cached_system[..reuse]);
                lcp = reuse;
            }
            PrefixPlan::ColdPrefill => {
                ctx.clear_kv_cache_seq(Some(0), None, None)
                    .map_err(|e| format!("KV seq-0 eviction failed: {}", e))?;
                cached.clear();
                lcp = 0;
            }
        }

        // The system anchor (seq 2) shares its cells with seq 0 only while seq 0
        // still holds the system prefix. On a ColdPrefill seq 0 diverges at
        // position 0, so on the unified KV cache the anchor's cells become
        // disjoint and count against n_ctx — a long cold prompt would overflow
        // with NoKvCacheSlot. The anchor is also stale then, so drop it here and
        // let `plan_anchor_seed` reseed it after the fresh prefill.
        if !anchor_shares_cells_with_seq0(plan) {
            ctx.clear_kv_cache_seq(Some(2), None, None)
                .map_err(|e| format!("KV anchor eviction failed: {}", e))?;
            cached_system.clear();
        }

        // Keep the volatile generation-header tail out of the persistent
        // prefix. Token-level boundary: a separate tokenisation of the stable
        // byte prefix absorbs merges at the seam.
        //
        // CRITICAL: also gate on `budget.drop_front == 0`. When the prompt was
        // front-truncated to fit n_ctx, `tokens` no longer starts with BOS —
        // it starts mid-sequence at whatever was at byte-offset
        // `drop_front`. But `&prompt[..b]` still tokenizes with a BOS at
        // position 0 (it sees the ORIGINAL bytes), so plan_stable_boundary's
        // LCP collapses to 0 → stable_tok = 0 → the actor decodes zero
        // tokens on seq 0 and the entire prompt lands on seq 1 (throwaway).
        // After that, cached stays empty, every subsequent round goes cold,
        // and the chat loops forever cold-prefilling — the exact regression
        // reproduced in `reports/bench/kv_personal_20260615_153947.stderr.log`.
        // Pathological prompts that need front-truncation are already
        // sacrificing cache warmth; just decode the whole thing on seq 0.
        stable_tok = match stable_prompt_bytes {
            Some(b) if budget.drop_front == 0 && b < prompt.len() && prompt.is_char_boundary(b) => {
                let stable_tokens = model
                    .str_to_token(&prompt[..b], AddBos::Always)
                    .map_err(|e| format!("Stable-prefix tokenisation failed: {}", e))?;
                plan_stable_boundary(&tokens, &stable_tokens, lcp)
            }
            _ => tokens.len(),
        };
        if budget.drop_front > 0 {
            // Surface the truncation so the chat trace explains the cache
            // regression cleanly (matches the "system prefix not detected
            // this call" hint the FE shows when sys_tok=0).
            crate::services::logger::log(
                "info",
                "ai",
                format!(
                    "llamacpp: prompt front-truncated by {} tokens (orig {} → kept {}, max_tokens={}, n_ctx={}) — \
                     stable boundary disabled, whole prompt decodes on seq 0 (anchor not seeded)",
                    budget.drop_front,
                    budget.drop_front + tokens.len(),
                    tokens.len(),
                    max_tokens,
                    n_ctx,
                ),
            );
            // Actionable follow-up, once per actor lifetime: when the model
            // was trained for a bigger window, tell the user what to raise
            // the Context window setting to (and what it costs in memory).
            if !*n_ctx_suggested {
                let orig_prompt = budget.drop_front + tokens.len();
                if let Some(target) = plan_n_ctx_suggestion(n_ctx, model.n_ctx_train(), orig_prompt, budget.max_gen) {
                    *n_ctx_suggested = true;
                    let n_head = u64::from(model.n_head().max(1));
                    let head_dim = (model.n_embd().max(0) as u64) / n_head;
                    let kv_per_token = u64::from(model.n_layer()) * 2 * u64::from(model.n_head_kv()) * head_dim * 2;
                    let extra_mib = (u64::from(target) - n_ctx as u64) * kv_per_token / (1024 * 1024);
                    crate::services::logger::log(
                        "warn",
                        "ai",
                        format!(
                            "chat prompt ({} tokens) did not fit the {}-token context window and was cut — \
                             answers may miss context and the prompt cache resets each turn. This model supports \
                             up to {} tokens: consider raising \"Context window\" to {} in Settings → AI \
                             (~{} MiB more memory).",
                            orig_prompt,
                            n_ctx,
                            model.n_ctx_train(),
                            target,
                            extra_mib,
                        ),
                    );
                }
            }
        }

        // Token boundary of the system prefix (for anchor seeding). Only when
        // the prompt was not tail-truncated (byte offsets would misalign) and
        // never past the stable region we actually decode on seq 0.
        sys_tok = match system_prefix_bytes {
            Some(b) if budget.drop_front == 0 && b <= prompt.len() && prompt.is_char_boundary(b) => {
                let sys_tokens = model
                    .str_to_token(&prompt[..b], AddBos::Always)
                    .map_err(|e| format!("System-prefix tokenisation failed: {}", e))?;
                plan_stable_boundary(&tokens, &sys_tokens, 0).min(stable_tok)
            }
            _ => 0,
        };
        // Positions only — never token contents (derived from email plaintext).
        crate::services::logger::log(
            "info",
            "ai",
            format!(
                "llamacpp kv: cached={} sys_cached_before={} sys_cached={} prompt={} lcp={} stable={} sys={} plan={:?}",
                prev_cached,
                prev_sys_cached,
                cached_system.len(),
                tokens.len(),
                lcp,
                stable_tok,
                sys_tok,
                plan
            ),
        );
        prefill_seq = 0;
    } else {
        // One-shot request: keep the seq-0 chat prefix warm and run this
        // prompt entirely on the throwaway generation sequence. Evict the
        // resident prefix only when the prompt cannot fit beside it.
        let plan = plan_uncached_budget(tokens.len(), max_tokens, n_ctx, cached.len());
        crate::services::logger::log(
            "info",
            "ai",
            format!(
                "llamacpp kv: uncached prompt={} resident={} sys_cached={} evict={}",
                tokens.len(),
                cached.len(),
                prev_sys_cached,
                plan.evict_resident
            ),
        );
        if plan.evict_resident {
            // Full clear frees the anchor's cells too; drop both mirrors so
            // they never disagree with the real KV. The next chat turn reseeds
            // the anchor (correctness beats keeping it warm).
            ctx.clear_kv_cache();
            cached.clear();
            cached_system.clear();
        }
        if plan.budget.drop_front > 0 {
            tokens.drain(0..plan.budget.drop_front);
            dropped_front = plan.budget.drop_front;
        }
        max_gen = plan.budget.max_gen;
        lcp = 0;
        stable_tok = tokens.len();
        sys_tok = 0;
        prefill_seq = 1;
        plan_name = None;
    }
    let n_prompt = tokens.len();

    // Decode the non-reused STABLE suffix on the prefill sequence. At least
    // one of the two prefill chunks is non-empty (plan_prefix_reuse caps lcp
    // at n_prompt - 1), so the final decode always produces logits for
    // sampling the first generated token.
    let stable_suffix = &tokens[lcp..stable_tok];
    let mut batch = LlamaBatch::new(n_prompt - lcp, 1);
    if !stable_suffix.is_empty() {
        for (i, &token) in stable_suffix.iter().enumerate() {
            let pos = lcp + i;
            batch
                .add(token, pos as i32, &[prefill_seq], pos == n_prompt - 1)
                .map_err(|e| format!("Batch add error during prefill: {}", e))?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| format!("Prefill decode failed: {}", e))?;
        batch.clear();
    }

    if cache_prompt {
        cached.extend_from_slice(stable_suffix);

        // (Re)seed the never-evicted system anchor (seq 2) from seq 0's
        // freshly-decoded system prefix when it changed — first call or a
        // route flip that re-rendered the system message. The anchor shares
        // seq 0's cells (no extra allocation) and lets the next NEW
        // conversation reuse the system prefix without partial eviction.
        if sys_tok > 0 {
            let seed = plan_anchor_seed(cached_system, &tokens, sys_tok);
            if seed.reseed {
                ctx.clear_kv_cache_seq(Some(2), None, None)
                    .map_err(|e| format!("KV anchor eviction failed: {}", e))?;
                ctx.copy_kv_cache_seq(0, 2, None, Some(seed.sys_len as u32))
                    .map_err(|e| format!("KV seq-0→anchor copy failed: {}", e))?;
                cached_system.clear();
                cached_system.extend_from_slice(&tokens[..seed.sys_len]);
            }
        }

        // Generate on a copy so sampled tokens never pollute the seq-0 prompt
        // prefix (KV cells are shared via tagging; only recurrent state, if
        // any, is physically copied).
        ctx.copy_kv_cache_seq(0, 1, None, None)
            .map_err(|e| format!("KV seq copy failed: {}", e))?;
        // Volatile tail (generation header): decoded on seq 1 only, so the
        // persistent seq-0 prefix ends at a boundary the next turn extends.
        let volatile = &tokens[stable_tok..];
        if !volatile.is_empty() {
            for (i, &token) in volatile.iter().enumerate() {
                let pos = stable_tok + i;
                batch
                    .add(token, pos as i32, &[1], pos == n_prompt - 1)
                    .map_err(|e| format!("Batch add error during tail prefill: {}", e))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| format!("Tail prefill decode failed: {}", e))?;
            batch.clear();
        }
    }
    let prefill_ms = t_prefill.elapsed().as_millis() as i64;

    // Sampler chain: temperature → random distribution.
    // temperature=0 → effectively greedy via a near-zero temp.
    let eff_temp = temperature.max(1e-6);
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(eff_temp),
        LlamaSampler::dist(u32::MAX), // LLAMA_DEFAULT_SEED
    ]);

    let mut output = String::new();
    let mut n_gen = 0u32;

    for i in 0..max_gen {
        let token = sampler.sample(ctx, -1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            break;
        }

        let piece = {
            #[allow(deprecated)]
            model.token_to_str(token, Special::Tokenize).unwrap_or_default()
        };

        n_gen += 1;
        output.push_str(&piece);

        if let Some(ref mut cb) = on_token {
            if !cb(piece) {
                break; // caller requested early stop
            }
        }

        batch
            .add(token, (n_prompt + i) as i32, &[1], true)
            .map_err(|e| format!("Batch add error during generation: {}", e))?;
        ctx.decode(&mut batch)
            .map_err(|e| format!("Decode failed during generation: {}", e))?;
        batch.clear();
        // Sampled tokens land only in seq 1 — `cached` stays prompt-only.
    }

    Ok(GenOutcome {
        text: output,
        prompt_tokens: n_prompt as u32,
        gen_tokens: n_gen,
        prefill_ms,
        cached_prompt_tokens: lcp as u32,
        prefix_plan: plan_name,
        sys_cached_before: prev_sys_cached as u32,
        sys_cached_after: cached_system.len() as u32,
        system_prefix_tokens: sys_tok as u32,
        stable_tokens: stable_tok as u32,
        dropped_front_tokens: dropped_front as u32,
    })
}
