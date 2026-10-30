// One-shot vs chat KV-cache bench.
//
// The classifier and the query planner run as one-shot completions with
// `cache_prompt=false`, deliberately, so their prompts never evict the chat
// prefix the KV anchor holds. This bench measures whether that still holds —
// and what a chat turn actually pays — when one-shot traffic is interleaved
// with it, all inside ONE process, because every `make cli-*` invocation
// starts with an empty KV cache and can't see the interaction at all.
//
// It reports, per scenario, what the chat turn's prefill cost and how much of
// its prompt came from the cache. The wrapper script samples RSS and picks the
// llama.cpp buffer sizes out of stderr.

use std::path::PathBuf;

use serde::Serialize;

use crate::ai::provider::{AIProvider, AiMessage};
use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::{EvalError, EvalResult};
use crate::services::classification::{build_classify_prompt, ClassificationConfig, EmailToClassify};

/// Pinned so the one-shot prompts are identical from run to run.
const BENCH_TODAY: &str = "2026-06-15";

#[derive(Debug, Clone)]
pub struct KvBenchConfig {
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
    pub account: Option<String>,
    /// One-shot classifications per interleaved scenario.
    pub classifications: usize,
    /// Also re-run the interleaved scenario with the context window pinned to
    /// 8192 — the tier a 16 GB machine gets.
    pub small_ctx: bool,
}

/// What one chat turn cost, as the llama.cpp backend reported it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProbe {
    pub scenario: String,
    pub note: String,
    pub latency_ms: u64,
    pub prefill_ms: Option<i64>,
    pub prompt_tokens: Option<u32>,
    pub cached_prompt_tokens: Option<u32>,
    pub prefix_plan: Option<String>,
    pub sys_cached_before: Option<u32>,
    pub sys_cached_after: Option<u32>,
    pub system_prefix_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KvBenchReport {
    pub model: String,
    pub account: String,
    pub classifications_per_scenario: usize,
    pub probes: Vec<ChatProbe>,
}

pub async fn run(cfg: KvBenchConfig) -> EvalResult<KvBenchReport> {
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "oneshot-kv")?;
    let db = std::sync::Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;

    let account = match &cfg.account {
        Some(a) => a.clone(),
        None => db
            .list_accounts()?
            .into_iter()
            .find(|a| a.enabled)
            .map(|a| a.email)
            .unwrap_or_default(),
    };
    let account_id = db
        .list_accounts()?
        .into_iter()
        .find(|a| a.email == account)
        .map(|a| a.id)
        .unwrap_or_default();

    let model = db.get_preference("ai_model")?.unwrap_or_default();
    let mut probes = Vec::new();

    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;

    // Scenario 1 — the reference: anchor seeded, nothing else has run.
    let messages = chat_messages(&db, &account_id, "what did the supplier say about the delayed pallet?")?;
    let prewarm = chat_messages(&db, &account_id, "")?;
    provider
        .prewarm_chat_prefix(prewarm.clone())
        .await
        .map_err(|e| EvalError::Config(format!("prewarm failed: {e}")))?;
    probes.push(
        probe(
            provider.as_ref(),
            messages.clone(),
            "at_rest",
            "prewarmed anchor, no one-shot traffic",
        )
        .await?,
    );

    // Scenario 2 — after a burst of classifier one-shots.
    run_classifications(&db, provider.as_ref(), cfg.classifications).await?;
    probes.push(
        probe(
            provider.as_ref(),
            messages.clone(),
            "after_classifications",
            &format!("{} one-shot classifications first", cfg.classifications),
        )
        .await?,
    );

    // Scenario 3 — a planner call in the middle of the classification burst,
    // which is what a chat turn during a backfill actually sees.
    run_classifications(&db, provider.as_ref(), cfg.classifications / 2).await?;
    run_planner(&db, provider.as_ref(), &account).await?;
    run_classifications(&db, provider.as_ref(), cfg.classifications / 2).await?;
    probes.push(
        probe(
            provider.as_ref(),
            messages.clone(),
            "planner_interleaved",
            "classifications with one planner call between them",
        )
        .await?,
    );

    // Scenario 4 — a chat turn issued while a classification batch is still
    // running: the actor is one thread, so this measures the queue, not the
    // cache.
    let queued = {
        let provider_bg = std::sync::Arc::clone(&provider);
        let db_bg = std::sync::Arc::clone(&db);
        let n = cfg.classifications;
        let batch = tokio::spawn(async move { run_classifications(&db_bg, provider_bg.as_ref(), n).await });
        let probe = probe(
            provider.as_ref(),
            messages.clone(),
            "during_classification_batch",
            "chat turn queued behind a running batch",
        )
        .await?;
        let _ = batch.await;
        probe
    };
    probes.push(queued);

    // Scenario 5 — the 8k tier. `plan_uncached_budget` has to evict the chat
    // prefix to fit a planner prompt beside it at this size; this records what
    // that costs rather than arguing about it.
    if cfg.small_ctx {
        db.set_preference("chat.n_ctx", "8192")?;
        let small = crate::services::ai::AiService::load_provider(&db)
            .map_err(|e| EvalError::Config(format!("no AI provider at n_ctx=8192: {e}")))?;
        small
            .prewarm_chat_prefix(prewarm.clone())
            .await
            .map_err(|e| EvalError::Config(format!("prewarm failed at n_ctx=8192: {e}")))?;
        probes.push(
            probe(
                small.as_ref(),
                messages.clone(),
                "n_ctx_8192_at_rest",
                "context pinned to 8192",
            )
            .await?,
        );
        run_classifications(&db, small.as_ref(), cfg.classifications).await?;
        run_planner(&db, small.as_ref(), &account).await?;
        probes.push(
            probe(
                small.as_ref(),
                messages.clone(),
                "n_ctx_8192_after_oneshots",
                "context pinned to 8192, after classifications and a planner call",
            )
            .await?,
        );
        db.set_preference("chat.n_ctx", "0")?;
    }

    Ok(KvBenchReport {
        model,
        account,
        classifications_per_scenario: cfg.classifications,
        probes,
    })
}

/// The messages a real chat turn sends, built by the same `build_prompt` the
/// app and the prewarm path use, so the prefix bytes match.
fn chat_messages(db: &Database, account_id: &str, question: &str) -> EvalResult<Vec<AiMessage>> {
    let ai_language = crate::services::i18n::resolve_ai_language(db)
        .map_err(|e| EvalError::Config(format!("cannot resolve AI language: {e}")))?;
    let system_template = crate::services::prompts::get_template(db, "chat.system")
        .map_err(|e| EvalError::Config(format!("cannot load chat.system: {e}")))?;
    let registry = crate::services::chat::tools::default_registry();
    let tools_section = registry.render_system_prompt_section(db);
    let user_email = db
        .get_account(account_id)
        .ok()
        .flatten()
        .map(|a| a.email)
        .unwrap_or_default();

    Ok(crate::services::chat::build_prompt(
        &[],
        &[],
        question,
        ai_language.english_name(),
        &user_email,
        &system_template,
        &tools_section,
    )
    .into_iter()
    .map(|(role, content)| AiMessage {
        role,
        content,
        tool_calls: None,
    })
    .collect())
}

async fn probe(
    provider: &dyn AIProvider,
    messages: Vec<AiMessage>,
    scenario: &str,
    note: &str,
) -> EvalResult<ChatProbe> {
    let started = std::time::Instant::now();
    let result = provider
        .chat_stream_with_tools(messages, Vec::new(), Box::new(|_| true))
        .await
        .map_err(|e| EvalError::Config(format!("chat probe `{scenario}` failed: {e}")))?;
    Ok(ChatProbe {
        scenario: scenario.to_string(),
        note: note.to_string(),
        latency_ms: started.elapsed().as_millis() as u64,
        prefill_ms: result.prefill_ms,
        prompt_tokens: result.prompt_eval_count,
        cached_prompt_tokens: result.cached_prompt_tokens,
        prefix_plan: result.prefix_plan.map(str::to_string),
        sys_cached_before: result.sys_cached_before,
        sys_cached_after: result.sys_cached_after,
        system_prefix_tokens: result.system_prefix_tokens,
    })
}

/// Fire `n` classifier one-shots on synthetic emails — same shape as a
/// backfill, no mailbox read.
async fn run_classifications(db: &Database, provider: &dyn AIProvider, n: usize) -> EvalResult<()> {
    if n == 0 {
        return Ok(());
    }
    let config = ClassificationConfig::built_in();
    let template = crate::services::prompts::get_template(db, "classify.email")
        .map_err(|e| EvalError::Config(format!("cannot load classify.email: {e}")))?;
    let language = crate::services::i18n::resolve_ai_language(db)
        .map_err(|e| EvalError::Config(format!("cannot resolve AI language: {e}")))?;
    let language_clause = format!("Respond in {}.\n", language.english_name());

    for i in 0..n {
        let subject = format!("Invoice {} is overdue", 1000 + i);
        let snippet = format!(
            "Our records show invoice {} is unpaid. Please arrange payment.",
            1000 + i
        );
        let email = EmailToClassify {
            sender: "Accounts",
            sender_email: "accounts@northwind.test",
            subject: &subject,
            snippet: &snippet,
        };
        let prompt = build_classify_prompt(&template, &config, &language_clause, BENCH_TODAY, &email);
        let _ = crate::services::classification::classify_with_provider(provider, &config, &prompt.full()).await;
    }
    Ok(())
}

async fn run_planner(db: &Database, provider: &dyn AIProvider, user_email: &str) -> EvalResult<()> {
    let template = crate::services::prompts::get_template(db, "chat.query_plan")
        .map_err(|e| EvalError::Config(format!("cannot load chat.query_plan: {e}")))?;
    let glossary = crate::services::classification::TagGlossary::load(db);
    let _ = crate::services::chat::planner::plan_search(
        provider,
        &template,
        user_email,
        BENCH_TODAY,
        "what did the supplier say about the delayed pallet?",
        &glossary,
        // The KV bench measures the cached head; no form is open.
        None,
        &crate::services::forms::registry::catalog(db),
    )
    .await;
    Ok(())
}
