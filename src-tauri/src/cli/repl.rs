//! Interactive REPL (entered on a bare `emailops-cli` invocation).
//!
//! Grammar — **every action is an explicit slash-command** (no bare-text
//! default), so the REPL and the one-shot CLI never diverge:
//!   - **`/chat <question> [--trace]`** → a chat turn in the current
//!     conversation; tokens stream live to stdout via the installed pretty
//!     [`CliEventSink`](super::output::CliEventSink). The conversation id is
//!     carried on [`CliSession`] across turns, so REPL history shows up in the
//!     desktop app's chat sidebar too.
//!   - **other `/`-prefixed** → `/accounts`, `/emails`, `/search`, `/show`,
//!     `/sync`, `/classify`, `/embed`, `/config` map onto the same
//!     [`Command`](super::Command) enum the one-shot CLI uses; `/account`,
//!     `/model`, `/new`, `/help`, `/quit` manage session state. Switching account
//!     via `/account <id|email>` is persisted as the CLI default, so the next
//!     launch resolves it automatically.
//!   - **a bare email id** (a single token matching a real id) → runs
//!     `/show <id>`, so pasting an id from an `/emails` / `/search` listing acts
//!     like "click to open".
//!   - **other bare text** → rejected with a hint pointing at `/chat`.

use std::io::IsTerminal;
use std::sync::Arc;

use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

use crate::models::error::Result;

use super::commands;
use super::output::CliEventSink;
use super::session::CliSession;
use super::{Cli, Command, OutputMode};

/// Run the interactive shell until EOF / `/quit`.
pub async fn run(session: &mut CliSession) -> Result<()> {
    // The REPL always streams chat tokens to stdout, regardless of a stray
    // global `--json` flag — interactive output is inherently human-facing. Drop
    // any `Json` style the same way (recompute from TTY + NO_COLOR), so the REPL
    // is Rich on a terminal and Plain when its stdout is piped.
    session.mode = OutputMode::Pretty;
    session.style = super::resolve_render_style(
        false,
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    );
    crate::services::events::install(Arc::new(CliEventSink::new(session.style)));

    println!(
        "EmailOps interactive shell. Every action is a slash-command (e.g. /chat, /search). \
         Type /help for commands, /quit to exit."
    );

    let mut line_editor = Reedline::create();

    loop {
        let label = session.account.clone().unwrap_or_else(|| "no account".to_string());
        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(format!("emailops ({label})")),
            DefaultPromptSegment::Empty,
        );

        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(buffer)) => {
                let line = buffer.trim();
                if line.is_empty() {
                    continue;
                }
                if line == "/quit" || line == "/exit" {
                    break;
                }
                if let Some(rest) = line.strip_prefix('/') {
                    if let Err(e) = handle_slash(session, rest).await {
                        eprintln!("error: {e}");
                    }
                } else if bare_line_is_id_candidate(line) && session.db.get_email_by_id(line).ok().flatten().is_some() {
                    // A bare line that is a single token matching a real email id
                    // acts like "click to open": paste/type an id from the
                    // `/emails` or `/search` listing and it runs `/show <id>`.
                    if let Err(e) = handle_slash(session, &format!("show {line}")).await {
                        eprintln!("error: {e}");
                    }
                } else {
                    // Other bare text does not silently start a chat. Point the
                    // user at /chat.
                    eprintln!("commands start with '/'. to chat, use: /chat {line}   (try /help)");
                }
            }
            Ok(Signal::CtrlC) | Ok(Signal::CtrlD) => break,
            Ok(_) => continue,
            Err(e) => {
                eprintln!("[emailops-cli] readline error: {e}");
                break;
            }
        }
    }

    println!("bye.");
    Ok(())
}

/// Whether a bare (non-slash) REPL line could be an email id: a single,
/// non-empty token with no whitespace. The caller still verifies the id exists
/// before running `show`, so a single non-id word falls through to the chat hint
/// rather than erroring.
fn bare_line_is_id_candidate(line: &str) -> bool {
    !line.is_empty() && !line.contains(char::is_whitespace)
}

/// Split a slash-command line into tokens, respecting single/double quotes so a
/// multi-word argument can be passed as one token (e.g.
/// `search "IB Trading Assistant"` → `["search", "IB Trading Assistant"]`). The
/// quote characters are consumed; whitespace inside quotes is preserved.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut in_single = false;
    let mut in_double = false;
    for c in input.chars() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                has_token = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                has_token = true;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if has_token {
                    tokens.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        tokens.push(cur);
    }
    tokens
}

/// Handle a `/`-prefixed line. Session-management commands are handled inline;
/// everything else is parsed into a [`Command`] and routed through `dispatch`.
async fn handle_slash(session: &mut CliSession, rest: &str) -> Result<()> {
    let tokens = tokenize(rest);
    let (name, args): (&str, Vec<&str>) = match tokens.split_first() {
        Some((n, a)) => (n.as_str(), a.iter().map(String::as_str).collect()),
        None => return Ok(()),
    };

    match name {
        "help" => {
            print_help();
            Ok(())
        }
        "new" => {
            session.conversation_id = None;
            println!("started a new conversation.");
            Ok(())
        }
        "account" => match args.first() {
            Some(hint) => switch_account(session, hint),
            None => {
                println!("current account: {}", session.account.as_deref().unwrap_or("(none)"));
                Ok(())
            }
        },
        "model" => match args.first() {
            Some(m) => switch_model(session, m),
            None => {
                println!("{}", format_model_menu(&session.model));
                Ok(())
            }
        },
        // Explicit multi-turn chat (same as bare text), so `/chat … --trace`
        // works without diverging into the one-shot `Command::Chat` path (which
        // would start a fresh conversation each call).
        "chat" => chat_command(session, &args).await,
        // Map the remaining slash-commands onto the shared Command enum by
        // re-parsing them through clap, so the REPL and one-shot CLI never
        // diverge. We rebuild a full argv (binary name + slash-name + args).
        other => {
            let mut argv: Vec<String> = vec!["emailops-cli".to_string(), other.to_string()];
            argv.extend(args.iter().map(|s| s.to_string()));
            match Cli::try_parse_from_repl(&argv) {
                Ok(Some(command)) => commands::dispatch(session, command).await,
                Ok(None) => {
                    eprintln!("unknown command: /{other} (try /help)");
                    Ok(())
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    Ok(())
                }
            }
        }
    }
}

/// Switch the working account by id or email. The choice is persisted as the
/// CLI default (canonical account id) so the next launch — REPL or one-shot —
/// resolves it without re-selecting.
fn switch_account(session: &mut CliSession, hint: &str) -> Result<()> {
    let h = hint.trim();
    let matched = crate::services::accounts::list_accounts(&session.db)?
        .into_iter()
        .find(|a| a.id.eq_ignore_ascii_case(h) || a.email.eq_ignore_ascii_case(h));
    match matched {
        Some(a) => {
            session
                .db
                .set_preference(super::config::CLI_DEFAULT_ACCOUNT_KEY, &a.id)?;
            println!("switched to {} ({}) — saved as default", a.email, a.id);
            session.account = Some(a.id);
            // A different mailbox means a fresh chat context.
            session.conversation_id = None;
            Ok(())
        }
        None => {
            eprintln!("account '{hint}' not found");
            Ok(())
        }
    }
}

/// Switch the chat model. The choice is persisted to the `ai_model` preference —
/// the same key `AiService::get_config`/`load_provider` read to build the chat
/// provider — so the next `/chat` turn actually runs the chosen model. Updating
/// only `session.model` is not enough: `run_chat_turn` ignores its `model`
/// argument and always loads the provider (and thus the model) from this pref.
fn switch_model(session: &mut CliSession, name: &str) -> Result<()> {
    let model = name.trim().to_string();
    if model.is_empty() {
        eprintln!("usage: /model <name>");
        return Ok(());
    }
    session.db.set_preference("ai_model", &model)?;
    session.model = model.clone();
    // A model the user typed by hand may not be a catalog entry (e.g. an Ollama
    // tag). That's allowed — the provider resolves it — but flag it so a typo'd
    // catalog id doesn't silently fall through to a "model not found" at runtime.
    if crate::ai::model_catalog::find(&model).is_some() {
        println!("model set to {model} — saved");
    } else {
        println!("model set to {model} — saved (not in the download catalog; run /model to list known ones)");
    }
    Ok(())
}

/// Split `/chat` arguments into the question text and a `--trace` flag. Pure so
/// the flag handling is unit-testable without running a chat turn.
fn parse_chat_args(args: &[&str]) -> (String, bool) {
    let mut trace = false;
    let mut words: Vec<&str> = Vec::new();
    for a in args {
        if *a == "--trace" {
            trace = true;
        } else {
            words.push(a);
        }
    }
    (words.join(" "), trace)
}

/// Handle `/chat <question> [--trace]`. Like bare text, this is a multi-turn
/// chat in the session's current conversation; `--trace` prints the route /
/// retrieval / tool-call block after the answer.
async fn chat_command(session: &mut CliSession, args: &[&str]) -> Result<()> {
    let (question, trace) = parse_chat_args(args);
    if question.is_empty() {
        eprintln!("usage: /chat <question> [--trace]");
        return Ok(());
    }
    chat_turn(session, question, trace).await
}

/// Restores the logger verbosity gate to the session default when dropped, so a
/// `/chat` turn's `--trace`-scoped log suppression never leaks into the next
/// REPL command — including when the turn returns early with an error.
struct LogQuietGuard {
    gate: Arc<std::sync::atomic::AtomicBool>,
    restore_to: bool,
}

impl Drop for LogQuietGuard {
    fn drop(&mut self) {
        self.gate.store(self.restore_to, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Run a chat turn in the session's current conversation, creating one on the
/// first turn (or after `/new`). Tokens stream live via the installed sink. The
/// conversation id is carried on [`CliSession`] across turns, so successive
/// turns share context exactly like the desktop app. When `trace` is set the
/// route / retrieval / tool-call block is printed after the answer.
async fn chat_turn(session: &mut CliSession, question: String, trace: bool) -> Result<()> {
    let account_id = session.require_account()?;
    let model = session.model.clone();

    // Gate the diagnostic app-log stream on `--trace` (see `run_chat`); the
    // guard restores the session default on every exit path so later /sync,
    // /search, etc. keep their progress logs even if the turn errors out.
    session.apply_chat_log_quiet(trace);
    let _restore = LogQuietGuard {
        gate: session.log_quiet.clone(),
        restore_to: session.quiet,
    };

    let conversation_id = match &session.conversation_id {
        Some(id) => id.clone(),
        None => {
            let conv = session.db.create_chat_conversation(&account_id, "New chat")?;
            session.conversation_id = Some(conv.id.clone());
            conv.id
        }
    };

    let user_message = session
        .db
        .insert_chat_message(&conversation_id, "user", &question, None)?;
    let assistant_message = session
        .db
        .insert_chat_message(&conversation_id, "assistant", "", Some(&model))?;

    let mut history = session.db.get_recent_chat_turns(&conversation_id, 20)?;
    history.retain(|m| m.id != assistant_message.id && m.id != user_message.id);

    let registry = Arc::new(crate::services::chat::tools::default_registry());
    let categories: Vec<String> = crate::services::chat::DEFAULT_RAG_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();

    crate::services::chat::run_chat_turn(
        session.db.clone(),
        registry,
        conversation_id.clone(),
        user_message.id.clone(),
        assistant_message.id.clone(),
        account_id,
        question,
        model,
        history,
        categories,
    )
    .await?;

    // Read the finished assistant message back so we can re-render the answer as
    // aligned, styled markdown (replacing the dim live preview the sink cleared),
    // surface any drafts it created, and — with `--trace` — the route/retrieval
    // block.
    let assistant = session
        .db
        .get_chat_messages(&conversation_id)?
        .into_iter()
        .find(|m| m.id == assistant_message.id);
    let answer = assistant.as_ref().map(|m| m.content.clone()).unwrap_or_default();
    super::output::render_final_answer(&answer, session.style);

    if let Some(m) = assistant.as_ref() {
        for draft in commands::collect_referenced_drafts(&session.db, &m.referenced_draft_ids)? {
            super::output::render_draft(&draft, session.style.color());
        }
    }

    if trace {
        let chat_trace = assistant.as_ref().and_then(|m| m.trace.clone());
        let sources = assistant.as_ref().map(|m| m.sources.clone()).unwrap_or_default();
        super::output::render_chat_trace(chat_trace.as_ref(), &sources, session.style.color());
    }

    Ok(())
}

/// Render the `/model` (no-arg) menu: every chat model in the catalog, one per
/// line, with the active one marked by `*`. Pure (returns a `String`) so the
/// layout is unit-testable without capturing stdout. `selected` is the session's
/// current model id; if it isn't a catalog entry (e.g. an Ollama model the user
/// typed by hand), it's still surfaced on its own line so the user can see what
/// is actually active.
fn format_model_menu(selected: &str) -> String {
    use crate::ai::model_catalog;

    let models: Vec<&model_catalog::CatalogModel> = model_catalog::chat_models().collect();
    let id_width = models.iter().map(|m| m.id.len()).max().unwrap_or(0);

    let mut out = String::from("available chat models (* = current):\n");
    let mut selected_in_catalog = false;
    for m in &models {
        let marker = if m.id == selected {
            selected_in_catalog = true;
            '*'
        } else {
            ' '
        };
        let tag = if m.recommended { " (recommended)" } else { "" };
        out.push_str(&format!(
            "  {marker} {id:<id_width$}  {name}{tag}\n",
            id = m.id,
            name = m.display_name,
        ));
    }
    if !selected_in_catalog {
        out.push_str(&format!(
            "  * {selected:<id_width$}  (current; not in the download catalog)\n"
        ));
    }
    // Drop the trailing newline so the caller's `println!` adds exactly one.
    out.pop();
    out
}

fn print_help() {
    println!(
        "Commands (every action is a slash-command):\n\
         \x20 /chat <question> [--trace]   ask a question (multi-turn; --trace shows route/retrieval)\n\
         \x20 /accounts [add <gmail|outlook|imap …>]  list accounts, or add one\n\
         \x20 /emails [--limit N] [--offset N] [--mailbox M] [--category C]   list recent emails\n\
         \x20 /search <query> [--limit N] [--offset N] [--trace]   full-text search\n\
         \x20 /show <id>        show one email (or just paste an id on its own to open it)\n\
         \x20 /sync [account]   download new mail\n\
         \x20 /classify [--all] classify emails\n\
         \x20 /embed [--batch N] generate embeddings\n\
         \x20 /stats            dashboard stats per account (emails, categories, embeddings…)\n\
         \x20 /calendar [--days N] [--next] [--sync]   upcoming calendar events (per-account)\n\
         \x20 /translate <id> [--to LANG] [--detect-only]   detect / translate an email with AI\n\
         \x20 /config <get|set|unset|list> [key] [value]  manage CLI preferences\n\
         \x20 /account [<id|email>]  show or switch the working account (switch is saved as default)\n\
         \x20 /model [<name>]   list available models (no arg) or set the AI model\n\
         \x20 /new              start a fresh conversation\n\
         \x20 /help             show this help\n\
         \x20 /quit             exit"
    );
}

impl Cli {
    /// Parse a REPL slash-command argv into a [`Command`]. Returns `Ok(None)`
    /// when no subcommand matched, and `Err(message)` with clap's rendered
    /// help/error text on a parse failure (so the REPL can print it without
    /// exiting the process the way `Cli::parse` would).
    fn try_parse_from_repl(argv: &[String]) -> std::result::Result<Option<Command>, String> {
        use clap::Parser;
        match Cli::try_parse_from(argv) {
            Ok(cli) => Ok(cli.command),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_line_id_candidate_accepts_single_token_rejects_phrases() {
        assert!(bare_line_is_id_candidate("demo_12e75795718f4f29"));
        assert!(bare_line_is_id_candidate("18f9c0a1b2c3d4e5"));
        assert!(!bare_line_is_id_candidate("")); // empty
        assert!(!bare_line_is_id_candidate("what invoices arrived")); // a phrase → chat hint
        assert!(!bare_line_is_id_candidate("show me")); // two tokens
    }

    #[test]
    fn tokenize_splits_on_whitespace() {
        assert_eq!(
            tokenize("search invoice --limit 5"),
            vec!["search", "invoice", "--limit", "5"]
        );
    }

    #[test]
    fn tokenize_keeps_double_quoted_phrase_as_one_token() {
        assert_eq!(
            tokenize("search \"IB Trading Assistant\""),
            vec!["search", "IB Trading Assistant"]
        );
    }

    #[test]
    fn tokenize_keeps_single_quoted_phrase_as_one_token() {
        assert_eq!(
            tokenize("search 'from:ana invoice'"),
            vec!["search", "from:ana invoice"]
        );
    }

    #[test]
    fn tokenize_handles_flag_after_quoted_phrase() {
        assert_eq!(
            tokenize("search \"IB Trading\" --limit 5"),
            vec!["search", "IB Trading", "--limit", "5"]
        );
    }

    #[test]
    fn tokenize_preserves_empty_quoted_token() {
        assert_eq!(
            tokenize("config set default-account \"\""),
            vec!["config", "set", "default-account", ""]
        );
    }

    #[test]
    fn parse_chat_args_joins_question_without_trace() {
        let (q, trace) = parse_chat_args(&["what", "did", "ana", "say?"]);
        assert_eq!(q, "what did ana say?");
        assert!(!trace);
    }

    #[test]
    fn parse_chat_args_extracts_trace_flag_anywhere() {
        let (q, trace) = parse_chat_args(&["summarize", "--trace", "my", "inbox"]);
        assert_eq!(q, "summarize my inbox");
        assert!(trace);
    }

    #[test]
    fn parse_chat_args_empty_when_only_flag() {
        let (q, trace) = parse_chat_args(&["--trace"]);
        assert!(q.is_empty());
        assert!(trace);
    }

    #[test]
    fn model_menu_lists_every_chat_model_and_marks_current() {
        let menu = format_model_menu("qwen3.5-9b-q4_k_m");
        // Every chat model in the catalog is listed.
        for m in crate::ai::model_catalog::chat_models() {
            assert!(menu.contains(m.id), "menu must list {}", m.id);
        }
        // The current model is marked with '*', a non-current one is not.
        assert!(
            menu.contains("* qwen3.5-9b-q4_k_m"),
            "current model must be marked: {menu}"
        );
        assert!(
            !menu.contains("* qwen3.5-4b-q8_0"),
            "non-current model must not be marked: {menu}"
        );
        // The recommended default is tagged.
        assert!(
            menu.contains("(recommended)"),
            "recommended model must be tagged: {menu}"
        );
    }

    #[test]
    fn model_menu_surfaces_current_model_absent_from_catalog() {
        // A model the user typed by hand (e.g. an Ollama tag) isn't in the
        // download catalog, but the menu still shows it as the active one.
        let menu = format_model_menu("llama3:8b");
        assert!(
            menu.contains("* llama3:8b"),
            "off-catalog current model must show: {menu}"
        );
        assert!(
            menu.contains("not in the download catalog"),
            "off-catalog note must show: {menu}"
        );
        // No catalog entry is marked current in this case (only the synthetic line is).
        assert!(
            crate::ai::model_catalog::chat_models().all(|m| !menu.contains(&format!("* {}", m.id))),
            "no catalog model should be marked when current is off-catalog: {menu}"
        );
    }

    fn seed_account(db: &Arc<crate::db::Database>, id: &str, email: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, 'gmail', ?2, ?2, 0, 0, 1)",
                rusqlite::params![id, email],
            )
            .expect("seed account");
    }

    fn test_session(db: Arc<crate::db::Database>) -> CliSession {
        CliSession {
            db,
            account: None,
            model: "test-model".to_string(),
            mode: OutputMode::Pretty,
            style: crate::cli::RenderStyle::Rich,
            quiet: true,
            log_quiet: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            data_dir: std::path::PathBuf::from("/tmp/emailops-cli-test"),
            conversation_id: None,
        }
    }

    #[test]
    fn switch_model_persists_to_ai_model_pref() {
        let db = Arc::new(crate::db::Database::new_for_testing().expect("test db"));
        let mut session = test_session(db.clone());

        switch_model(&mut session, "gemma-4-12b-it-qat-ud-q4_k_xl").expect("switch model");

        // In-memory model updated …
        assert_eq!(session.model, "gemma-4-12b-it-qat-ud-q4_k_xl");
        // … and persisted to the `ai_model` pref that load_provider/get_config
        // actually read. Regression: /model used to update only session.model,
        // which run_chat_turn ignores — so the chat kept running the old model.
        assert_eq!(
            db.get_preference("ai_model").expect("pref"),
            Some("gemma-4-12b-it-qat-ud-q4_k_xl".to_string())
        );
        // get_config — the exact path load_provider uses to pick the model —
        // now reflects the choice, so the next chat turn runs it.
        assert_eq!(
            crate::services::ai::AiService::get_config(&db).expect("config").model,
            "gemma-4-12b-it-qat-ud-q4_k_xl"
        );
    }

    #[test]
    fn switch_account_persists_choice_as_default() {
        let db = Arc::new(crate::db::Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "one@example.com");
        seed_account(&db, "a2", "two@example.com");
        let mut session = test_session(db.clone());

        switch_account(&mut session, "two@example.com").expect("switch");

        // In-memory account updated …
        assert_eq!(session.account.as_deref(), Some("a2"));
        // … and persisted (as the canonical id) so the next launch resolves it
        // without re-selecting.
        assert_eq!(
            db.get_preference(super::super::config::CLI_DEFAULT_ACCOUNT_KEY)
                .expect("pref"),
            Some("a2".to_string())
        );
    }

    #[test]
    fn switch_account_unknown_hint_leaves_default_unset() {
        let db = Arc::new(crate::db::Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "one@example.com");
        let mut session = test_session(db.clone());

        switch_account(&mut session, "ghost@example.com").expect("switch is non-fatal");

        assert!(session.account.is_none());
        assert_eq!(
            db.get_preference(super::super::config::CLI_DEFAULT_ACCOUNT_KEY)
                .expect("pref"),
            None
        );
    }
}
