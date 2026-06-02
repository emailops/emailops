// `chat_probe` — single-turn end-to-end probe of the chat service.
//
// Runs `services::chat::run_chat_turn` against a temp copy of the real SQLite
// DB (so the model sees the user's actual mail), prints the assistant answer +
// full trace, then deletes the throwaway conversation.
//
// Usage:
//   cargo run --features eval --bin chat_probe -- \
//       --question "que entrevista de trabajo hice en 2007?"
//
// Flags:
//   --question / -q  The user message to send (required).
//   --account        Account id or email (defaults to the single enabled account).
//   --model          Model override (defaults to user_preferences.ai_model).
//   --prod-db        Path to the prod SQLite DB. Default: ~/Library/Application
//                    Support/com.emailops.app/emailops.db (macOS).
//   --json           Emit the whole result as one JSON document on stdout
//                    (otherwise: pretty text — answer, trace, sources).
//
// Requirements:
//   - The Tauri app must be closed (writes lock the DB).
//   - For local models, the configured backend (llama.cpp / Ollama / vLLM)
//     must be reachable.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use emailops_lib::db::Database;
use emailops_lib::evals::case_loader::EvalCase;
use emailops_lib::evals::db_source::{prepare_eval_db, EvalDbMode};
use emailops_lib::evals::harness::run_case;
use emailops_lib::evals::shared::build_mock_app;

#[derive(Parser, Debug)]
#[command(
    name = "chat_probe",
    about = "Run one chat turn end-to-end and dump the trace.",
    long_about = None,
)]
struct Args {
    /// Question to send to the chat.
    #[arg(long, short)]
    question: String,

    /// Account id or email. Defaults to the single enabled account.
    #[arg(long)]
    account: Option<String>,

    /// Model override. Defaults to user_preferences.ai_model.
    #[arg(long)]
    model: Option<String>,

    /// Path to the production SQLite DB.
    #[arg(long)]
    prod_db: Option<PathBuf>,

    /// Open the production DB in place instead of copying it to a temp DB.
    #[arg(long, hide = true)]
    in_place_dangerous: bool,

    /// Print the result as one JSON document instead of pretty text.
    #[arg(long)]
    json: bool,
}

fn main() {
    // Load .env from common locations (matches chat_eval).
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
        eprintln!("[chat_probe] prod DB not found at {}", prod_db.display());
        std::process::exit(2);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let db_mode = if args.in_place_dangerous {
        EvalDbMode::InPlaceDangerous
    } else {
        EvalDbMode::CopyToTemp
    };

    match rt.block_on(run_one(args, prod_db, db_mode)) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[chat_probe] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run_one(args: Args, prod_db: PathBuf, db_mode: EvalDbMode) -> Result<(), Box<dyn std::error::Error>> {
    let prepared_db = prepare_eval_db(&prod_db, db_mode, "chat-probe")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);

    // ── Account resolution: id or email; fall back to the single enabled one ─
    let account_id = match args.account.as_deref() {
        Some(hint) => {
            let h = hint.trim();
            let accounts = db.list_accounts()?;
            accounts
                .iter()
                .find(|a| a.id.eq_ignore_ascii_case(h) || a.email.eq_ignore_ascii_case(h))
                .map(|a| a.id.clone())
                .ok_or_else(|| format!("account '{}' not found in DB", hint))?
        }
        None => {
            let accounts = db.list_accounts()?;
            let enabled: Vec<_> = accounts.into_iter().filter(|a| a.enabled).collect();
            match enabled.len() {
                0 => return Err("no enabled accounts in DB".into()),
                1 => enabled[0].id.clone(),
                _ => {
                    return Err("multiple enabled accounts — pass --account <id|email>".into());
                }
            }
        }
    };

    // ── Model resolution: CLI override → user pref → safe default ────────────
    let model = match args.model.clone() {
        Some(m) => m,
        None => match db.get_preference("ai_model") {
            Ok(Some(v)) if !v.is_empty() => v,
            _ => "gemma4:e2b".to_string(),
        },
    };

    eprintln!(
        "[chat_probe] account={} model={} question={:?}",
        account_id, model, args.question
    );

    // Construct a minimal EvalCase. `run_case` only reads `question`.
    let case = EvalCase {
        id: "probe".into(),
        question: args.question.clone(),
        category: "probe".into(),
        tier: "smoke".into(),
        model: None,
        account: None,
        thread_id: None,
        expected_route: None,
        expected_tools_called: vec![],
        expected_answer_contains: vec![],
        expected_title_pattern: None,
        expected_output: None,
        metrics: vec![],
    };

    let app = build_mock_app()?;
    let outcome = run_case(db.clone(), app, &account_id, &model, &case).await?;

    // ── Output ───────────────────────────────────────────────────────────────
    if args.json {
        let sources: Vec<_> = outcome
            .sources_used
            .iter()
            .map(|s| {
                serde_json::json!({
                    "n": s.citation_number,
                    "email_id": s.email_id,
                    "subject": s.subject,
                    "sender": s.sender,
                    "sender_email": s.sender_email,
                    "relevance": s.relevance_score,
                    "snippet": s.body_snippet,
                })
            })
            .collect();
        let doc = serde_json::json!({
            "question": args.question,
            "answer": outcome.assistant_content,
            "conversation_title": outcome.conversation_title,
            "wall_ms": outcome.wall_elapsed_ms,
            "trace": outcome.assistant_trace,
            "sources_used": sources,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        println!();
        println!("─── QUESTION ────────────────────────────────────────");
        println!("{}", args.question);
        println!();
        println!("─── ANSWER ──────────────────────────────────────────");
        println!("{}", outcome.assistant_content);
        println!();
        println!("─── TRACE ───────────────────────────────────────────");
        match &outcome.assistant_trace {
            Some(t) => println!("{}", serde_json::to_string_pretty(t)?),
            None => println!("(no trace)"),
        }
        println!();
        if !outcome.sources_used.is_empty() {
            println!("─── SOURCES ─────────────────────────────────────────");
            for s in &outcome.sources_used {
                println!(
                    "[{}] {} — {} <{}>",
                    s.citation_number, s.subject, s.sender, s.sender_email
                );
            }
            println!();
        }
        println!(
            "wall: {}ms  (latency_ms from row: {:?})",
            outcome.wall_elapsed_ms, outcome.assistant_latency_ms
        );
    }

    // Clean up the throwaway conversation. FK cascade removes messages + sources.
    if !outcome.conversation_id.is_empty() {
        if let Err(e) = db.delete_chat_conversation(&outcome.conversation_id) {
            eprintln!(
                "[chat_probe] WARN: failed to delete probe conversation {}: {}",
                outcome.conversation_id, e
            );
        }
    }

    Ok(())
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
