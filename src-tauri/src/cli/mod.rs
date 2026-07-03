//! `emailops-cli` — power-user / agent-driven command line for EmailOps.
//!
//! The CLI is a thin front-end over the same `services::*` entry points the
//! Tauri commands call. It needs no `AppHandle`: UI events route through the
//! [`crate::services::events`] seam (a stdout/stderr sink in one-shot mode, a
//! channel sink feeding a live renderer in the REPL) and logs route through the
//! [`crate::services::logger`] seam.
//!
//! Two modes share one binary:
//!   - **one-shot** — a subcommand was given (`emailops-cli search invoice`).
//!   - **REPL** — no subcommand (bare `emailops-cli`) drops into an interactive
//!     chat shell with slash-commands (see [`repl`]).
//!
//! Every command supports `--json` so an agent (or a test harness) can drive
//! real features headlessly and assert on structured output.

mod accounts;
mod commands;
mod config;
mod doctor;
mod eval;
mod output;
mod render;
mod repl;
mod session;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

pub use accounts::AccountAction;
pub use config::ConfigAction;
pub use session::CliSession;

/// Output format for command results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable tables / sections (default).
    Pretty,
    /// One JSON document per command — for agents and tests.
    Json,
}

/// How human-facing output should be styled. Resolved **once** at bootstrap so
/// the styling decision never leaks token-cost into the agent (`--json`) or
/// piped paths: ANSI color, aligned-table re-rendering, live preview and cursor
/// redraw all live strictly in [`RenderStyle::Rich`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStyle {
    /// `--json`: one structured envelope, no styling at all. The agent contract.
    Json,
    /// Pretty **and** an interactive TTY (with `NO_COLOR` unset): full color,
    /// aligned tables, dim live preview + redraw to a clean render.
    Rich,
    /// Pretty but piped / redirected, or `NO_COLOR` set: plain text, no ANSI, no
    /// cursor control. Keeps captured output free of escape-code bloat (the
    /// `chat … > file` / forgot-`--json` footgun).
    Plain,
}

impl RenderStyle {
    /// Whether ANSI color may be emitted (Rich only).
    pub fn color(self) -> bool {
        matches!(self, RenderStyle::Rich)
    }

    /// Whether interactive cursor control (live preview, redraw, spinner) is
    /// allowed (Rich only — requires a real TTY).
    pub fn interactive(self) -> bool {
        matches!(self, RenderStyle::Rich)
    }
}

/// Resolve the human-output style from the `--json` flag, whether stdout is an
/// interactive terminal, and whether `NO_COLOR` is set. Pure so the policy is
/// unit-testable without a real TTY: `--json` always wins (agents pay nothing);
/// otherwise Rich requires a TTY with color allowed, and everything else (piped
/// output, `NO_COLOR`) degrades to Plain.
pub(crate) fn resolve_render_style(json: bool, stdout_is_tty: bool, no_color: bool) -> RenderStyle {
    if json {
        RenderStyle::Json
    } else if stdout_is_tty && !no_color {
        RenderStyle::Rich
    } else {
        RenderStyle::Plain
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "emailops-cli",
    about = "Power-user / agent command line for EmailOps.",
    long_about = "Drive EmailOps from the terminal: list/search/show mail, sync, chat, \
                  classify, and embed. Run with no subcommand to enter an interactive REPL.\n\n\
                  Output: pass --json for the stable, unstyled envelope (the agent contract — \
                  no color or table re-rendering, so no token bloat). Without --json, human output \
                  is styled (aligned tables, color) only on an interactive terminal; piped/redirected \
                  output and NO_COLOR fall back to plain text with zero escape codes.",
    version
)]
pub struct Cli {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress the app-log stream on stderr (errors still print).
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Override the EmailOps data directory (else `$EMAILOPS_DATA_DIR` or the
    /// platform default).
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// Account id or email to operate on (defaults to the single enabled
    /// account).
    #[arg(long, global = true, value_name = "ID|EMAIL")]
    pub account: Option<String>,

    /// Model override for AI commands (else the `ai_model` preference).
    #[arg(long, global = true, value_name = "MODEL")]
    pub model: Option<String>,

    /// Override a prompt template for THIS run only (not persisted, never
    /// touches the DB). Repeatable. Format: `<prompt-id>=<file>`. Ids come from
    /// the prompt registry — e.g. `chat.system`, `chat.query_rewrite`,
    /// `chat.rerank`, `classify.email`, `memory.tasks`, `memory.facts`. The
    /// file is read verbatim and rendered with the usual `{{variables}}`, so a
    /// custom `chat.system` can still use `{{user_identity}}`, `{{today}}`,
    /// `{{tools_section}}`, etc. Lets you A/B prompts without editing code.
    #[arg(long = "prompt", global = true, value_name = "ID=FILE")]
    pub prompt_override: Vec<String>,

    /// Shorthand for `--prompt chat.system=<file>`.
    #[arg(long, global = true, value_name = "FILE")]
    pub system_prompt: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// One-shot subcommands. The REPL maps its slash-commands onto these same
/// variants so behavior never diverges between the two front-ends.
#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// List or add accounts. Bare `accounts` lists; `accounts add <provider>`
    /// connects a new Gmail / Outlook (OAuth) or IMAP account.
    Accounts {
        #[command(subcommand)]
        action: Option<AccountAction>,
    },

    /// List recent emails for an account.
    Emails {
        /// Max emails to return.
        #[arg(long, default_value_t = 25)]
        limit: i32,
        /// Skip this many emails before returning (for paging: page 2 = `--offset 25`).
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Mailbox filter: inbox | sent | spam | trash.
        #[arg(long)]
        mailbox: Option<String>,
        /// Gmail category filter: primary | social | promotions | updates | forums.
        #[arg(long)]
        category: Option<String>,
    },

    /// Show a single email (headers + body).
    Show {
        /// Email id.
        id: String,
    },

    /// Full-text search across an account's mail.
    Search {
        /// Search query.
        query: String,
        /// Max hits to return.
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Skip this many hits before returning (for paging: page 2 = `--offset 25`).
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Include search diagnostics (method, AI availability, parsed filters,
        /// hit counts). `--json` puts them under `data.trace`; pretty mode
        /// prints a trace block after the results.
        #[arg(long)]
        trace: bool,
    },

    /// Ask one or more questions against your mail and stream the answers.
    /// Multiple questions run sequentially in ONE conversation and process, so
    /// the model stays loaded between turns (what `make cli-bench` relies on).
    Chat {
        /// The question(s) to ask.
        #[arg(required = true, num_args = 1..)]
        questions: Vec<String>,
        /// Include the chat trace (route, retrieval, tool calls, model timings)
        /// in the result — `--json` puts it under `data.trace`; pretty mode
        /// prints a dim trace block after the answer.
        #[arg(long)]
        trace: bool,
        /// Continue an existing conversation instead of starting a new one. Pass
        /// the `conversationId` returned by a previous `chat --json` to carry
        /// context across one-shot invocations (multi-turn).
        #[arg(long, value_name = "ID")]
        conversation: Option<String>,
        /// Run each question in its OWN new conversation (instead of one shared
        /// conversation), all within this single process so the model + KV cache
        /// stay loaded. Use this to measure cross-conversation prompt-cache reuse
        /// (the first LLM round of chat N reusing chat N-1's resident prefix).
        /// Ignored together with `--conversation`.
        #[arg(long)]
        fresh: bool,
    },

    /// Download new mail for an account.
    Sync {
        /// Account id or email (else the global `--account` / default).
        account: Option<String>,
    },

    /// Classify new (or all) emails into categories.
    Classify {
        /// Re-classify every email, not just new ones.
        #[arg(long)]
        all: bool,
        /// Classify exactly the given email id (skips the queue scan). Useful
        /// for reproducing classifier failures on a known-bad message.
        #[arg(long, value_name = "ID", conflicts_with = "all")]
        id: Option<String>,
    },

    /// Generate search embeddings for pending emails.
    Embed {
        /// Rows to embed per batch.
        #[arg(long, default_value_t = 50)]
        batch: i32,
    },

    /// Report environment readiness (DB, accounts, AI config). Read-only and
    /// fast — loads no AI model.
    Doctor,

    /// Dashboard stats per account: local/sent/server email totals, per-category
    /// counts, and classified / embeddings / memory / tasks coverage.
    Stats,

    /// Compose an email. Saves it as a draft by default (and pushes it to the
    /// provider's Drafts folder when supported); pass `--send` to deliver now.
    Compose {
        /// Recipient address (repeatable): `--to a@x.com --to b@y.com`.
        #[arg(long, value_name = "EMAIL")]
        to: Vec<String>,
        /// Cc address (repeatable).
        #[arg(long, value_name = "EMAIL")]
        cc: Vec<String>,
        /// Subject line.
        #[arg(long, default_value = "")]
        subject: String,
        /// Body text. Mutually exclusive with `--body-file`.
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the body from a file instead of `--body`.
        #[arg(long, value_name = "FILE", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Attach a file by path (repeatable). Stored as a reference; bytes are
        /// read at send / provider-push time.
        #[arg(long = "attach", value_name = "FILE")]
        attach: Vec<PathBuf>,
        /// Send immediately instead of saving as a draft.
        #[arg(long)]
        send: bool,
        /// Update an existing draft (by id) instead of creating a new one.
        #[arg(long, value_name = "ID")]
        draft: Option<String>,
    },

    /// List saved drafts for an account.
    Drafts,

    /// Show a single draft (recipients, subject, body, attachments).
    Draft {
        /// Draft id.
        id: String,
    },

    /// Get/set CLI-local preferences (e.g. the default account).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Run chat eval cases through the shared harness and report pass/fail.
    /// Requires the `eval` cargo feature.
    Eval {
        /// Run only the case with this id.
        #[arg(long)]
        case: Option<String>,
        /// Run only cases in this tier (e.g. `smoke`).
        #[arg(long)]
        tier: Option<String>,
        /// Override the eval-cases directory.
        #[arg(long, value_name = "DIR")]
        cases_dir: Option<PathBuf>,
    },
}

/// Process entry point. Builds a current-thread tokio runtime (mirrors the eval
/// harnesses), parses args, bootstraps the session, and dispatches.
pub fn run() -> ExitCode {
    // Load .env from the same locations the eval harnesses use so local model
    // / provider config is picked up when running from the repo.
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let cli = Cli::parse();
    let mode = if cli.json { OutputMode::Json } else { OutputMode::Pretty };

    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[emailops-cli] failed to start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The CLI is the boundary that turns typed `AppError`s into a structured
    // failure envelope (JSON mode → stdout, pretty → stderr) plus a process exit
    // code grouped by remediation. Success returns 0.
    match rt.block_on(run_async(cli, mode)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            output::emit_error(&e, mode);
            ExitCode::from(output::exit_code(&e))
        }
    }
}

async fn run_async(cli: Cli, mode: OutputMode) -> crate::models::error::Result<()> {
    // Bootstrap: keychain → data dir → DB → install logger + event sink.
    let mut session = CliSession::bootstrap(&cli, mode)?;

    // Run-scoped prompt overrides (--prompt id=file / --system-prompt file).
    // Installed after bootstrap so the logger is up and validation errors flow
    // through the standard envelope. No-op when neither flag is passed.
    let overrides = build_prompt_overrides(&cli.prompt_override, cli.system_prompt.as_deref())?;
    if !overrides.is_empty() {
        let mut ids: Vec<&str> = overrides.keys().map(String::as_str).collect();
        ids.sort_unstable();
        crate::services::logger::log(
            "info",
            "prompts",
            format!("run-scoped prompt override active: {}", ids.join(", ")),
        );
        crate::services::prompts::install_overrides(overrides);
    }

    match cli.command.clone() {
        Some(command) => commands::dispatch(&mut session, command).await,
        None => repl::run(&mut session).await,
    }
}

/// Build the run-scoped prompt-override map from the CLI flags. Validates each
/// id against the prompt registry (a typo errors instead of silently doing
/// nothing) and reads each template file verbatim. `--system-prompt` is sugar
/// for `--prompt chat.system=<file>`.
fn build_prompt_overrides(
    specs: &[String],
    system_prompt: Option<&std::path::Path>,
) -> crate::models::error::Result<std::collections::HashMap<String, String>> {
    use crate::models::error::AppError;
    let mut out = std::collections::HashMap::new();

    let mut add = |id: &str, path: &std::path::Path| -> crate::models::error::Result<()> {
        if crate::services::prompts::registry::lookup(id).is_none() {
            return Err(AppError::InvalidInput(format!(
                "unknown prompt id '{id}' — run `emailops-cli` prompt ids include chat.system, chat.query_rewrite, chat.rerank, classify.email, memory.tasks, memory.facts"
            )));
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| AppError::InvalidInput(format!("cannot read prompt file {}: {e}", path.display())))?;
        if text.trim().is_empty() {
            return Err(AppError::InvalidInput(format!(
                "prompt file {} is empty",
                path.display()
            )));
        }
        out.insert(id.to_string(), text);
        Ok(())
    };

    if let Some(path) = system_prompt {
        add("chat.system", path)?;
    }
    for spec in specs {
        let (id, path) = spec
            .split_once('=')
            .ok_or_else(|| AppError::InvalidInput(format!("--prompt expects ID=FILE, got '{spec}'")))?;
        add(id.trim(), std::path::Path::new(path.trim()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn render_style_json_wins_even_on_a_tty() {
        // Agents pass --json; they must never get styling regardless of TTY.
        assert_eq!(resolve_render_style(true, true, false), RenderStyle::Json);
        assert_eq!(resolve_render_style(true, false, true), RenderStyle::Json);
        assert!(!RenderStyle::Json.color());
        assert!(!RenderStyle::Json.interactive());
    }

    #[test]
    fn render_style_rich_only_on_tty_with_color() {
        let s = resolve_render_style(false, true, false);
        assert_eq!(s, RenderStyle::Rich);
        assert!(s.color());
        assert!(s.interactive());
    }

    #[test]
    fn render_style_plain_when_piped_or_no_color() {
        // Piped (not a TTY) → Plain even though color isn't disabled.
        assert_eq!(resolve_render_style(false, false, false), RenderStyle::Plain);
        // NO_COLOR set on a TTY → Plain (no ANSI, no redraw).
        let s = resolve_render_style(false, true, true);
        assert_eq!(s, RenderStyle::Plain);
        assert!(!s.color());
        assert!(!s.interactive());
    }

    #[test]
    fn cli_definition_is_valid() {
        // Catches duplicate flags / bad arg specs at test time.
        Cli::command().debug_assert();
    }

    #[test]
    fn no_subcommand_parses_to_none() {
        let cli = Cli::parse_from(["emailops-cli"]);
        assert!(cli.command.is_none(), "bare invocation must enter the REPL");
    }

    #[test]
    fn global_flags_parse_before_and_after_subcommand() {
        let cli = Cli::parse_from(["emailops-cli", "--json", "search", "invoice", "--limit", "5"]);
        assert!(cli.json);
        match cli.command {
            Some(Command::Search {
                query, limit, trace, ..
            }) => {
                assert_eq!(query, "invoice");
                assert_eq!(limit, 5);
                assert!(!trace);
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn account_is_global_and_works_after_subcommand() {
        let cli = Cli::parse_from(["emailops-cli", "emails", "--account", "me@example.com", "--limit", "10"]);
        assert_eq!(cli.account.as_deref(), Some("me@example.com"));
        match cli.command {
            Some(Command::Emails {
                limit,
                mailbox,
                category,
                ..
            }) => {
                assert_eq!(limit, 10);
                assert!(mailbox.is_none());
                assert!(category.is_none());
            }
            other => panic!("expected Emails, got {other:?}"),
        }
    }

    #[test]
    fn classify_all_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "classify", "--all"]);
        assert!(matches!(cli.command, Some(Command::Classify { all: true, id: None })));
    }

    #[test]
    fn classify_defaults_to_new_only() {
        let cli = Cli::parse_from(["emailops-cli", "classify"]);
        assert!(matches!(cli.command, Some(Command::Classify { all: false, id: None })));
    }

    #[test]
    fn classify_id_targets_single_email() {
        // `--id <id>` reproduces classifier failures on a specific message
        // without scanning the whole unclassified queue.
        let cli = Cli::parse_from(["emailops-cli", "classify", "--id", "abc123"]);
        match cli.command {
            Some(Command::Classify { all, id }) => {
                assert!(!all);
                assert_eq!(id.as_deref(), Some("abc123"));
            }
            other => panic!("expected Classify, got {other:?}"),
        }
    }

    #[test]
    fn classify_id_and_all_are_mutually_exclusive() {
        // Either re-run the whole queue (`--all`) or a single message (`--id`),
        // never both: avoids ambiguity about which path runs.
        let err = Cli::try_parse_from(["emailops-cli", "classify", "--all", "--id", "abc"]);
        assert!(err.is_err(), "clap should reject --all together with --id");
    }

    #[test]
    fn search_limit_defaults_to_25() {
        let cli = Cli::parse_from(["emailops-cli", "search", "invoice"]);
        match cli.command {
            Some(Command::Search {
                query, limit, trace, ..
            }) => {
                assert_eq!(query, "invoice");
                assert_eq!(limit, 25);
                assert!(!trace);
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn search_offset_defaults_to_zero_and_parses() {
        let cli = Cli::parse_from(["emailops-cli", "search", "invoice"]);
        match cli.command {
            Some(Command::Search { offset, .. }) => assert_eq!(offset, 0),
            other => panic!("expected Search, got {other:?}"),
        }
        let cli = Cli::parse_from(["emailops-cli", "search", "invoice", "--offset", "25"]);
        match cli.command {
            Some(Command::Search { offset, .. }) => assert_eq!(offset, 25),
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn emails_offset_defaults_to_zero_and_parses() {
        let cli = Cli::parse_from(["emailops-cli", "emails"]);
        match cli.command {
            Some(Command::Emails { offset, .. }) => assert_eq!(offset, 0),
            other => panic!("expected Emails, got {other:?}"),
        }
        let cli = Cli::parse_from(["emailops-cli", "emails", "--offset", "50"]);
        match cli.command {
            Some(Command::Emails { offset, .. }) => assert_eq!(offset, 50),
            other => panic!("expected Emails, got {other:?}"),
        }
    }

    #[test]
    fn stats_command_parses() {
        let cli = Cli::parse_from(["emailops-cli", "stats"]);
        assert!(matches!(cli.command, Some(Command::Stats)));
    }

    #[test]
    fn emails_limit_defaults_to_25() {
        let cli = Cli::parse_from(["emailops-cli", "emails"]);
        match cli.command {
            Some(Command::Emails { limit, .. }) => assert_eq!(limit, 25),
            other => panic!("expected Emails, got {other:?}"),
        }
    }

    #[test]
    fn search_trace_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "search", "invoice", "--trace"]);
        match cli.command {
            Some(Command::Search { query, trace, .. }) => {
                assert_eq!(query, "invoice");
                assert!(trace);
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn emails_mailbox_filter_parses() {
        let cli = Cli::parse_from(["emailops-cli", "emails", "--mailbox", "sent"]);
        match cli.command {
            Some(Command::Emails {
                limit,
                mailbox,
                category,
                ..
            }) => {
                assert_eq!(limit, 25);
                assert_eq!(mailbox.as_deref(), Some("sent"));
                assert!(category.is_none());
            }
            other => panic!("expected Emails, got {other:?}"),
        }
    }

    #[test]
    fn emails_category_filter_parses() {
        let cli = Cli::parse_from(["emailops-cli", "emails", "--category", "promotions"]);
        match cli.command {
            Some(Command::Emails {
                limit,
                mailbox,
                category,
                ..
            }) => {
                assert_eq!(limit, 25);
                assert!(mailbox.is_none());
                assert_eq!(category.as_deref(), Some("promotions"));
            }
            other => panic!("expected Emails, got {other:?}"),
        }
    }

    #[test]
    fn show_takes_positional_id() {
        let cli = Cli::parse_from(["emailops-cli", "show", "email-123"]);
        assert!(matches!(cli.command, Some(Command::Show { id }) if id == "email-123"));
    }

    #[test]
    fn embed_batch_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "embed", "--batch", "200"]);
        assert!(matches!(cli.command, Some(Command::Embed { batch: 200 })));
    }

    #[test]
    fn chat_trace_flag_is_off_by_default() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "what's new?"]);
        assert!(matches!(cli.command, Some(Command::Chat { trace: false, .. })));
    }

    #[test]
    fn chat_trace_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "what's new?", "--trace"]);
        match cli.command {
            Some(Command::Chat {
                questions,
                trace,
                conversation,
                fresh,
            }) => {
                assert_eq!(questions, vec!["what's new?".to_string()]);
                assert!(trace);
                assert!(conversation.is_none());
                assert!(!fresh);
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn chat_conversation_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "and then?", "--conversation", "conv-123"]);
        match cli.command {
            Some(Command::Chat {
                questions,
                conversation,
                ..
            }) => {
                assert_eq!(questions, vec!["and then?".to_string()]);
                assert_eq!(conversation.as_deref(), Some("conv-123"));
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn chat_fresh_flag_is_off_by_default() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "what's new?"]);
        assert!(matches!(cli.command, Some(Command::Chat { fresh: false, .. })));
    }

    #[test]
    fn chat_fresh_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "a?", "b?", "--fresh"]);
        match cli.command {
            Some(Command::Chat { questions, fresh, .. }) => {
                assert_eq!(questions, vec!["a?", "b?"]);
                assert!(fresh);
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn chat_accepts_multiple_questions() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "first?", "second?", "third?"]);
        match cli.command {
            Some(Command::Chat { questions, .. }) => {
                assert_eq!(questions, vec!["first?", "second?", "third?"]);
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn chat_requires_at_least_one_question() {
        assert!(Cli::try_parse_from(["emailops-cli", "chat"]).is_err());
    }

    #[test]
    fn doctor_parses() {
        let cli = Cli::parse_from(["emailops-cli", "doctor"]);
        assert!(matches!(cli.command, Some(Command::Doctor)));
    }

    #[test]
    fn config_set_default_account_parses() {
        let cli = Cli::parse_from(["emailops-cli", "config", "set", "default-account", "me@example.com"]);
        match cli.command {
            Some(Command::Config {
                action: ConfigAction::Set { key, value },
            }) => {
                assert!(matches!(key, config::ConfigKey::DefaultAccount));
                assert_eq!(value, "me@example.com");
            }
            other => panic!("expected Config Set, got {other:?}"),
        }
    }

    #[test]
    fn config_list_parses() {
        let cli = Cli::parse_from(["emailops-cli", "config", "list"]);
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                action: ConfigAction::List
            })
        ));
    }

    #[test]
    fn eval_case_and_tier_flags_parse() {
        let cli = Cli::parse_from(["emailops-cli", "eval", "--case", "kickoff_es", "--tier", "smoke"]);
        match cli.command {
            Some(Command::Eval { case, tier, cases_dir }) => {
                assert_eq!(case.as_deref(), Some("kickoff_es"));
                assert_eq!(tier.as_deref(), Some("smoke"));
                assert!(cases_dir.is_none());
            }
            other => panic!("expected Eval, got {other:?}"),
        }
    }

    #[test]
    fn bare_accounts_parses_to_list_default() {
        let cli = Cli::parse_from(["emailops-cli", "accounts"]);
        assert!(matches!(cli.command, Some(Command::Accounts { action: None })));
    }

    #[test]
    fn accounts_list_parses() {
        let cli = Cli::parse_from(["emailops-cli", "accounts", "list"]);
        assert!(matches!(
            cli.command,
            Some(Command::Accounts {
                action: Some(AccountAction::List)
            })
        ));
    }

    #[test]
    fn accounts_add_gmail_parses_with_sync_from() {
        let cli = Cli::parse_from(["emailops-cli", "accounts", "add", "gmail", "--sync-from", "2024-01-01"]);
        match cli.command {
            Some(Command::Accounts {
                action:
                    Some(AccountAction::Add {
                        provider: accounts::AddProvider::Gmail { sync_from },
                    }),
            }) => assert_eq!(sync_from.as_deref(), Some("2024-01-01")),
            other => panic!("expected accounts add gmail, got {other:?}"),
        }
    }

    #[test]
    fn accounts_add_outlook_parses() {
        let cli = Cli::parse_from(["emailops-cli", "accounts", "add", "outlook"]);
        assert!(matches!(
            cli.command,
            Some(Command::Accounts {
                action: Some(AccountAction::Add {
                    provider: accounts::AddProvider::Outlook { sync_from: None }
                })
            })
        ));
    }

    #[test]
    fn accounts_add_imap_parses_fields_and_defaults() {
        let cli = Cli::parse_from([
            "emailops-cli",
            "accounts",
            "add",
            "imap",
            "--host",
            "imap.fastmail.com",
            "--username",
            "me@fastmail.com",
            "--password",
            "secret",
        ]);
        match cli.command {
            Some(Command::Accounts {
                action:
                    Some(AccountAction::Add {
                        provider:
                            accounts::AddProvider::Imap {
                                host,
                                port,
                                username,
                                password,
                                smtp_host,
                                smtp_port,
                                name,
                                sync_from,
                            },
                    }),
            }) => {
                assert_eq!(host, "imap.fastmail.com");
                assert_eq!(port, 993);
                assert_eq!(username, "me@fastmail.com");
                assert_eq!(password.as_deref(), Some("secret"));
                assert!(smtp_host.is_none());
                assert_eq!(smtp_port, 587);
                assert!(name.is_none());
                assert!(sync_from.is_none());
            }
            other => panic!("expected accounts add imap, got {other:?}"),
        }
    }

    #[test]
    fn accounts_add_imap_requires_host_and_username() {
        // Missing both required flags → clap parse error (not a silent default).
        assert!(Cli::try_parse_from(["emailops-cli", "accounts", "add", "imap"]).is_err());
    }

    #[test]
    fn sync_takes_optional_positional_account() {
        let with = Cli::parse_from(["emailops-cli", "sync", "me@example.com"]);
        assert!(matches!(with.command, Some(Command::Sync { account: Some(a) }) if a == "me@example.com"));
        let without = Cli::parse_from(["emailops-cli", "sync"]);
        assert!(matches!(without.command, Some(Command::Sync { account: None })));
    }

    #[test]
    fn prompt_override_flags_parse_as_global() {
        // Both flags are global, so they attach before OR after the subcommand.
        let cli = Cli::parse_from([
            "emailops-cli",
            "--system-prompt",
            "/tmp/sys.txt",
            "chat",
            "hi",
            "--prompt",
            "chat.rerank=/tmp/rr.txt",
        ]);
        assert_eq!(cli.system_prompt.as_deref(), Some(std::path::Path::new("/tmp/sys.txt")));
        assert_eq!(cli.prompt_override, vec!["chat.rerank=/tmp/rr.txt".to_string()]);
    }

    #[test]
    fn build_prompt_overrides_rejects_unknown_id_and_bad_spec() {
        // Unknown prompt id → loud error, not a silent no-op.
        assert!(build_prompt_overrides(&["not.a.prompt=/tmp/x".into()], None).is_err());
        // Missing '=' → error.
        assert!(build_prompt_overrides(&["chat.system".into()], None).is_err());
    }

    #[test]
    fn build_prompt_overrides_reads_files_and_maps_system_shorthand() {
        let dir = std::env::temp_dir();
        let sys = dir.join("emailops_test_sys_prompt.txt");
        let rr = dir.join("emailops_test_rerank_prompt.txt");
        std::fs::write(&sys, "CUSTOM SYSTEM {{today}}").expect("write sys");
        std::fs::write(&rr, "CUSTOM RERANK").expect("write rr");

        let map = build_prompt_overrides(&[format!("chat.rerank={}", rr.display())], Some(sys.as_path()))
            .expect("overrides build");

        assert_eq!(
            map.get("chat.system").map(String::as_str),
            Some("CUSTOM SYSTEM {{today}}")
        );
        assert_eq!(map.get("chat.rerank").map(String::as_str), Some("CUSTOM RERANK"));

        let _ = std::fs::remove_file(&sys);
        let _ = std::fs::remove_file(&rr);
    }

    #[test]
    fn build_prompt_overrides_rejects_empty_file() {
        let path = std::env::temp_dir().join("emailops_test_empty_prompt.txt");
        std::fs::write(&path, "   \n").expect("write empty");
        assert!(build_prompt_overrides(&[], Some(path.as_path())).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
