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
// KV REUSE — TWO SEQUENCES
// ────────────────────────
// `cached_tokens` mirrors exactly the PROMPT tokens materialised in sequence
// 0 of the context's KV cache. Generation happens on sequence 1 (a copy of
// seq 0), which is dropped wholesale at the start of the next request.
//
// Why: each tool round's prompt is a strict extension of the previous
// round's PROMPT, but not of the previous round's prompt+generation (the
// re-rendered assistant message rarely byte-matches the raw sampled
// tokens). Rolling back a partially-divergent sequence needs a partial KV
// removal, which llama.cpp does not support for hybrid-attention / SWA /
// recurrent caches (clear_kv_cache_seq returns Ok(false)) — and a failed
// rollback forces a full multi-second re-prefill. Keeping generation out of
// seq 0 means seq 0 only ever EXTENDS, which needs no rollback at all:
//   1. evict all of seq 1 (full-sequence removal always succeeds)
//   2. evict seq 0 positions ≥ lcp (a no-op when the prompt purely extends)
//   3. decode the STABLE prompt suffix on seq 0 (see below)
//   4. copy seq 0 → seq 1, decode the volatile tail on seq 1, generate on seq 1
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

use super::planner::{plan_prefix_reuse, plan_prompt_budget, plan_stable_boundary, plan_uncached_budget};
use super::runtime::backend;

/// Cap on the persistent context window. Even on models trained for 32k+
/// tokens, prompt-eval time on M1 CPU/Metal scales roughly linearly with the
/// number of attended tokens, and the KV allocation scales with n_ctx — 8k is
/// plenty for the RAG chat pipeline and keeps the resident cache bounded.
const MAX_N_CTX: u32 = 8192;

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
    /// the thread itself (it cannot be sent across).
    pub(crate) fn spawn(model: Arc<LlamaModel>) -> std::result::Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel::<GenRequest>();
        std::thread::Builder::new()
            .name("llama-inference".into())
            .spawn(move || actor_loop(&model, &rx))
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
                on_token,
                reply: reply_tx,
            })
            .map_err(|_| "Inference thread is no longer running".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Inference thread dropped the request".to_string())?
    }
}

fn actor_loop(model: &LlamaModel, rx: &Receiver<GenRequest>) {
    let n_ctx = model.n_ctx_train().clamp(1024, MAX_N_CTX);
    // n_batch = n_ctx so a full-window prompt fits in a single decode call
    // (GGML_ASSERT(n_tokens_all <= cparams.n_batch) trips otherwise).
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(n_ctx))
        .with_n_batch(n_ctx)
        .with_n_ubatch(n_ctx.min(N_UBATCH))
        // Seq 0 = persistent prompt prefix, seq 1 = per-request generation.
        // Unified buffer: without it llama.cpp splits the n_ctx cell budget
        // per sequence (n_ctx/2 each → NoKvCacheSlot on long prompts). The
        // two sequences share cells via tagging, so unified costs nothing.
        .with_n_seq_max(2)
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
    while let Ok(req) = rx.recv() {
        let GenRequest {
            prompt,
            temperature,
            max_tokens,
            cache_prompt,
            stable_prompt_bytes,
            mut on_token,
            reply,
        } = req;
        let result = generate_with_cache(
            model,
            &mut ctx,
            &mut cached_tokens,
            &prompt,
            temperature,
            max_tokens,
            cache_prompt,
            stable_prompt_bytes,
            on_token.as_mut(),
        );
        if result.is_err() {
            // The decode state is unknown after a failure — drop everything so
            // the mirror never disagrees with the real KV contents.
            ctx.clear_kv_cache();
            cached_tokens.clear();
        }
        let _ = reply.send(result);
    }
}

/// One generation pass against the persistent context.
///
/// `cached` is the token mirror of the seq-0 (prompt-only) KV contents; it
/// is updated in lockstep with every successful prompt decode and truncated
/// on eviction, so it is accurate even when the pass fails midway (the
/// caller still resets on error for defence in depth).
#[allow(clippy::too_many_arguments)]
fn generate_with_cache(
    model: &LlamaModel,
    ctx: &mut LlamaContext,
    cached: &mut Vec<LlamaToken>,
    prompt: &str,
    temperature: f32,
    max_tokens: usize,
    cache_prompt: bool,
    stable_prompt_bytes: Option<usize>,
    mut on_token: Option<&mut OnToken>,
) -> std::result::Result<GenOutcome, String> {
    // Prefill clock starts before tokenisation: everything up to the first
    // sampled token is latency the user perceives as "thinking".
    let t_prefill = std::time::Instant::now();
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
    let prefill_seq: i32;
    if cache_prompt {
        let budget = plan_prompt_budget(tokens.len(), max_tokens, n_ctx);
        if budget.drop_front > 0 {
            tokens.drain(0..budget.drop_front);
        }
        max_gen = budget.max_gen;

        // Reuse the seq-0 prefix shared with the previous prompt, then evict
        // everything past it — positions ≥ lcp hold stale entries.
        let prev_cached = cached.len();
        let mut reuse = plan_prefix_reuse(cached, &tokens);
        let cleared = if reuse < cached.len() {
            ctx.clear_kv_cache_seq(Some(0), Some(reuse as u32), None)
                .map_err(|e| format!("KV eviction failed: {}", e))?
        } else {
            true // pure extension: nothing to evict
        };
        if !cleared {
            // Partial removal unsupported (hybrid-attention / SWA / recurrent
            // caches): fall back to a full re-prefill.
            crate::services::logger::log(
                "debug",
                "ai",
                format!("llamacpp kv: partial eviction unsupported at {reuse} — full re-prefill"),
            );
            ctx.clear_kv_cache();
            cached.clear();
            reuse = 0;
        }
        cached.truncate(reuse);
        lcp = reuse;

        // Keep the volatile generation-header tail out of the persistent
        // prefix. Token-level boundary: a separate tokenisation of the stable
        // byte prefix absorbs merges at the seam.
        stable_tok = match stable_prompt_bytes {
            Some(b) if b < prompt.len() && prompt.is_char_boundary(b) => {
                let stable_tokens = model
                    .str_to_token(&prompt[..b], AddBos::Always)
                    .map_err(|e| format!("Stable-prefix tokenisation failed: {}", e))?;
                plan_stable_boundary(&tokens, &stable_tokens, lcp)
            }
            _ => tokens.len(),
        };
        // Positions only — never token contents (derived from email plaintext).
        crate::services::logger::log(
            "debug",
            "ai",
            format!(
                "llamacpp kv: cached={} prompt={} lcp={} stable={}",
                prev_cached,
                tokens.len(),
                lcp,
                stable_tok
            ),
        );
        prefill_seq = 0;
    } else {
        // One-shot request: keep the seq-0 chat prefix warm and run this
        // prompt entirely on the throwaway generation sequence. Evict the
        // resident prefix only when the prompt cannot fit beside it.
        let plan = plan_uncached_budget(tokens.len(), max_tokens, n_ctx, cached.len());
        crate::services::logger::log(
            "debug",
            "ai",
            format!(
                "llamacpp kv: uncached prompt={} resident={} evict={}",
                tokens.len(),
                cached.len(),
                plan.evict_resident
            ),
        );
        if plan.evict_resident {
            ctx.clear_kv_cache();
            cached.clear();
        }
        if plan.budget.drop_front > 0 {
            tokens.drain(0..plan.budget.drop_front);
        }
        max_gen = plan.budget.max_gen;
        lcp = 0;
        stable_tok = tokens.len();
        prefill_seq = 1;
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
    })
}
