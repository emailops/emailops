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

#[derive(Parser, Debug)]
#[command(
    name = "emailops-cli",
    about = "Power-user / agent command line for EmailOps.",
    long_about = "Drive EmailOps from the terminal: list/search/show mail, sync, chat, \
                  classify, and embed. Run with no subcommand to enter an interactive REPL.",
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
        #[arg(long, default_value_t = 50)]
        limit: i32,
        /// Skip this many emails before returning (for paging: page 2 = `--offset 50`).
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

    /// Ask one question against your mail and stream the answer.
    Chat {
        /// The question to ask.
        question: String,
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

    match cli.command.clone() {
        Some(command) => commands::dispatch(&mut session, command).await,
        None => repl::run(&mut session).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

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
        assert!(matches!(cli.command, Some(Command::Classify { all: true })));
    }

    #[test]
    fn classify_defaults_to_new_only() {
        let cli = Cli::parse_from(["emailops-cli", "classify"]);
        assert!(matches!(cli.command, Some(Command::Classify { all: false })));
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
                assert_eq!(limit, 50);
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
                assert_eq!(limit, 50);
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
                question,
                trace,
                conversation,
            }) => {
                assert_eq!(question, "what's new?");
                assert!(trace);
                assert!(conversation.is_none());
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn chat_conversation_flag_parses() {
        let cli = Cli::parse_from(["emailops-cli", "chat", "and then?", "--conversation", "conv-123"]);
        match cli.command {
            Some(Command::Chat {
                question, conversation, ..
            }) => {
                assert_eq!(question, "and then?");
                assert_eq!(conversation.as_deref(), Some("conv-123"));
            }
            other => panic!("expected Chat, got {other:?}"),
        }
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
}
