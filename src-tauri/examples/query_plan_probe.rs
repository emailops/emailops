// `query_plan_probe` — measure how a tiny, focused prompt performs at turning a
// mailbox question into a single `search_emails` filter (the "planner" idea from
// the chat design discussion).
//
// Unlike `chat_probe` (which runs the whole chat turn — full system prompt, all
// tool schemas, history, multi-round tool loop), this sends ONE small prompt
// straight to `AIProvider::complete` and parses the JSON it returns. That isolates
// two questions:
//   1. Correctness — does the model emit the right from/to/limit filter (and
//      `defer` for non-search asks) from a ~150-token prompt?
//   2. Latency — how fast is a small-prompt completion vs. the ~40s cold-prefill
//      rounds the full chat path pays when its prompt overflows the context?
//
// This is an exploratory diagnostic, not a shipped feature — hence an example
// (cargo's home for ad-hoc tools), not a CLI subcommand. The planner only becomes
// a real capability once wired INTO chat as the tool-choice fast path, at which
// point chat --trace / chat_eval measure it with no standalone surface.
//
// NOTE: there is no grammar (GBNF) here — `CompletionOptions` doesn't expose one
// yet. Testing the prompt *without* a grammar is deliberate: it tells us whether
// a grammar is even needed (i.e. how often the bare prompt yields invalid JSON).
//
// Usage:
//   cargo run --features eval --example query_plan_probe -- \
//       --prod-db .emailops-demo-data/emailops.db --account ulises@emailopslabs.dev \
//       --model qwen3.5-9b-q4_k_m \
//       "last 3 emails I sent to alex" "emails sent to me today" "thanks!"
//
// With no positional queries a synthetic default set runs. Pass your own queries
// as positional args to try real phrasings (these are NOT written to disk).
//
// Requirements: the configured local backend (llama.cpp / Ollama) must be
// reachable; the model file must exist locally.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;

use emailops_lib::ai::provider::CompletionOptions;
use emailops_lib::db::Database;
use emailops_lib::evals::db_source::{prepare_eval_db, EvalDbMode};
use emailops_lib::services::ai::AiService;

/// The experimental planner prompt. Kept here (not in the prompt registry) — this
/// is a probe, not production. `{email}` / `{today}` are filled per run; `{query}`
/// per question.
const PLANNER_TEMPLATE: &str = r#"You convert ONE mailbox question into a single search_emails filter, as JSON.
The user's own address is {email}. Today is {today} (UTC).

Fields (use null when the question does not imply them):
  query   : topic / keywords
  from    : sender filter
  to      : recipient filter
  subject : subject keywords
  since   : ISO date YYYY-MM-DD (range start)
  until   : ISO date YYYY-MM-DD (range end)
  limit   : integer 1-25

Rules:
- "emails I sent" / "sent by me" -> the user is the AUTHOR -> from = {email}.
- "sent to me" / "my inbox" / "I received" -> the user is the RECIPIENT -> to = {email}.
- A named third party ("to alex", "from marta") -> put that name in the matching
  from/to field (NOT in query).
- "last" / "latest" -> small limit (e.g. 3-5).
- If the question is NOT a single search (it asks to write/draft/summarize, or is
  not about finding mail), output exactly {"defer": true} and nothing else.

Output ONLY the JSON object — no prose, no markdown fences.

Question: {query}
JSON:"#;

const DEFAULT_QUERIES: &[&str] = &[
    "last 3 emails I sent to alex",
    "emails I sent",
    "what did marta send me last week",
    "emails sent to me today",
    "invoices from acme over the last month",
    "draft a reply to the budget thread",
    "thanks!",
];

#[derive(Parser, Debug)]
#[command(name = "query_plan_probe", about = "Probe a tiny planner prompt: query -> search_emails filter.", long_about = None)]
struct Args {
    /// Questions to plan. Defaults to a synthetic set when none are given.
    #[arg(value_name = "QUERY")]
    queries: Vec<String>,

    /// Account id or email. Defaults to the single enabled account.
    #[arg(long)]
    account: Option<String>,

    /// Model override (sets `ai_model` on the throwaway DB copy).
    #[arg(long)]
    model: Option<String>,

    /// Path to the SQLite DB to read account/model config from.
    #[arg(long)]
    prod_db: Option<PathBuf>,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }
    let args = Args::parse();
    let prod_db = args
        .prod_db
        .clone()
        .or_else(default_prod_db)
        .unwrap_or_else(|| PathBuf::from("emailops.db"));
    if !prod_db.exists() {
        eprintln!("[query_plan_probe] DB not found at {}", prod_db.display());
        std::process::exit(2);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    if let Err(e) = rt.block_on(run(args, prod_db)) {
        eprintln!("[query_plan_probe] ERROR: {e}");
        std::process::exit(1);
    }
}

async fn run(args: Args, prod_db: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_eval_db(&prod_db, EvalDbMode::CopyToTemp, "query-plan-probe")?;
    let db = Arc::new(Database::new(prepared.db_dir().to_path_buf())?);

    let accounts = db.list_accounts()?;
    let email = match args.account.as_deref() {
        Some(hint) => accounts
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(hint.trim()) || a.email.eq_ignore_ascii_case(hint.trim()))
            .map(|a| a.email.clone())
            .ok_or_else(|| format!("account '{hint}' not found"))?,
        None => {
            let enabled: Vec<_> = accounts.iter().filter(|a| a.enabled).collect();
            match enabled.as_slice() {
                [one] => one.email.clone(),
                [] => return Err("no enabled accounts".into()),
                _ => return Err("multiple enabled accounts — pass --account <id|email>".into()),
            }
        }
    };

    // Model override goes onto the throwaway copy, never the source DB.
    if let Some(m) = args.model.as_deref() {
        db.set_preference("ai_model", m)?;
    }
    let provider = AiService::load_provider(&db)?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let queries: Vec<String> = if args.queries.is_empty() {
        DEFAULT_QUERIES.iter().map(|s| s.to_string()).collect()
    } else {
        args.queries.clone()
    };

    eprintln!(
        "[query_plan_probe] model={} account_email={} queries={}",
        provider.model_name(),
        email,
        queries.len()
    );

    let base = PLANNER_TEMPLATE.replace("{email}", &email).replace("{today}", &today);
    let prompt_tokens_hint = base.len() / 4; // rough char/4 estimate, sans query
    println!("\nPlanner prompt ≈ {prompt_tokens_hint} tokens (template only)\n");

    let mut latencies = Vec::new();
    let mut valid = 0usize;
    for q in &queries {
        let prompt = base.replace("{query}", q);
        let opts = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(128),
            think: Some(false),
        };
        let t = Instant::now();
        let result = provider.complete(&prompt, opts).await;
        let ms = t.elapsed().as_millis();
        latencies.push(ms);

        match result {
            Ok(r) => {
                let parsed = extract_json(&r.text);
                let ok = parsed.is_some();
                if ok {
                    valid += 1;
                }
                println!("─────────────────────────────────────────────");
                println!("Q: {q}");
                println!(
                    "  {ms}ms  ({} prompt / {} completion tok)",
                    r.prompt_tokens, r.completion_tokens
                );
                match parsed {
                    Some(v) => println!("  filter: {}", compact(&v)),
                    None => println!("  ⚠ INVALID JSON, raw: {}", r.text.trim().replace('\n', " ⏎ ")),
                }
            }
            Err(e) => println!("Q: {q}\n  ERROR: {e}"),
        }
    }

    if !latencies.is_empty() {
        latencies.sort_unstable();
        let sum: u128 = latencies.iter().sum();
        let med = latencies[latencies.len() / 2];
        println!("\n═════════════════════════════════════════════");
        println!(
            "{}/{} valid JSON | latency: min {}ms / median {}ms / max {}ms / avg {}ms",
            valid,
            queries.len(),
            latencies.first().copied().unwrap_or(0),
            med,
            latencies.last().copied().unwrap_or(0),
            sum / latencies.len() as u128,
        );
    }
    Ok(())
}

/// Lenient JSON extraction: strip ``` fences, then take the first balanced-looking
/// {...} slice and parse it.
fn extract_json(text: &str) -> Option<serde_json::Value> {
    let cleaned = text.replace("```json", "").replace("```", "");
    let start = cleaned.find('{')?;
    let end = cleaned.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&cleaned[start..=end]).ok()
}

/// Render a filter object with null fields dropped, for a compact one-liner.
fn compact(v: &serde_json::Value) -> String {
    match v.as_object() {
        Some(map) => {
            let kept: Vec<String> = map
                .iter()
                .filter(|(_, val)| !val.is_null())
                .map(|(k, val)| format!("{k}={val}"))
                .collect();
            format!("{{ {} }}", kept.join(", "))
        }
        None => v.to_string(),
    }
}

#[cfg(target_os = "macos")]
fn default_prod_db() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("com.emailops.app")
            .join("emailops.db")
    })
}

#[cfg(not(target_os = "macos"))]
fn default_prod_db() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("com.emailops.app").join("emailops.db"))
}
