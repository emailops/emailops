//! Record live provider API responses as JSON cassettes for sync tests.
//!
//! Uses the user's existing OAuth tokens (keychain in release, SQLite
//! `dev_tokens` table in debug) and hits Gmail / Microsoft Graph live,
//! capturing each `(method, url, status, headers, body)` triple into a
//! [`emailops_lib::sync::mock::Cassette`].
//!
//! Hard guarded for development use only:
//! - The recording module (`sync::mock`) is `#[cfg(any(test, debug_assertions))]`,
//!   so a release build won't link the cassette types.
//! - This example checks `cfg!(debug_assertions)` at runtime and refuses to
//!   record otherwise.
//! - Recording requires an explicit `--record-scenario` flag — no default,
//!   no auto-run.
//!
//! # Example
//!
//! ```bash
//! # Discover the account id
//! cargo run --example record_provider_cassette -- --list-accounts
//!
//! # Record a happy-path inbox list + per-message fetch sequence
//! cargo run --example record_provider_cassette -- \
//!     --record-scenario outlook_happy_path \
//!     --account you@example.com \
//!     --limit 5
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use emailops_lib::models::OAuthTokens;
use emailops_lib::services;
use emailops_lib::sync::mock::{sanitize_cassette, Cassette, Interaction, RecordedRequest, RecordedResponse};
use emailops_lib::Database;
use reqwest::Method;

#[derive(Parser, Debug)]
#[command(name = "record_provider_cassette")]
#[command(about = "Capture live provider HTTP responses as cassettes for sync tests")]
struct Cli {
    /// Explicit opt-in. Recording does nothing without this flag.
    /// Pass the scenario name as the value (will become the cassette filename
    /// stem). Example: `--record-scenario outlook_happy_path`.
    #[arg(long)]
    record_scenario: Option<String>,

    /// Email or account id to record against.
    #[arg(long)]
    account: Option<String>,

    /// How many list items to fetch and how many of them to follow up on
    /// with a per-message GET.
    #[arg(long, default_value_t = 5)]
    limit: u32,

    /// Output directory. Defaults to `src-tauri/tests/fixtures/cassettes/<provider>/`.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Skip sanitisation. Recorded file goes under `cassettes/raw/<provider>/`
    /// and is `.gitignore`d so real email content never gets committed.
    #[arg(long, default_value_t = false)]
    raw: bool,

    /// Override the data directory containing `emailops.db`. Defaults to the
    /// macOS app data path.
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// List accounts and exit. Use this to find the right `--account` value.
    #[arg(long, default_value_t = false)]
    list_accounts: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Dev-build-only safety gate. The `sync::mock` types this example uses
    // are themselves cfg-gated, so a release build wouldn't link — but bail
    // explicitly so the failure mode is obvious instead of a linker error.
    if !cfg!(debug_assertions) {
        eprintln!("ERROR: cassette recording is only available in debug builds. Run with `cargo run --example …` (not `--release`).");
        std::process::exit(1);
    }

    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);
    let db = Arc::new(Database::new(data_dir.clone())?);
    services::accounts::warm_token_cache(&db);

    if cli.list_accounts {
        return list_accounts(&db);
    }

    let scenario = cli
        .record_scenario
        .ok_or("missing --record-scenario <name>; required so recording never happens accidentally")?;
    let account_selector = cli
        .account
        .ok_or("missing --account <email-or-id>; pass --list-accounts to see options")?;
    let account = resolve_account(&db, &account_selector)?;

    let tokens = services::accounts::get_tokens(&account.id)?;

    eprintln!(
        "[record] scenario={} provider={} account={} limit={}",
        scenario, account.provider, account.email, cli.limit
    );

    let interactions = match account.provider.as_str() {
        "outlook" => record_outlook(&tokens, cli.limit).await?,
        "gmail" => record_gmail(&tokens, cli.limit).await?,
        other => return Err(format!("recording not supported for provider {}", other).into()),
    };

    let cassette = Cassette {
        scenario: scenario.clone(),
        provider: account.provider.clone(),
        sanitized: false,
        recorded_at: chrono::Utc::now().timestamp(),
        interactions,
    };
    let cassette = if cli.raw { cassette } else { sanitize_cassette(cassette) };

    let out_dir = cli.out.unwrap_or_else(|| default_out_dir(&account.provider, cli.raw));
    let path = cassette.write_to(&out_dir)?;
    eprintln!(
        "[record] wrote {} interaction(s) to {} (sanitized={})",
        cassette.interactions.len(),
        path.display(),
        cassette.sanitized
    );

    Ok(())
}

fn list_accounts(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let accounts = db.list_accounts()?;
    if accounts.is_empty() {
        eprintln!("No accounts found in DB at this data dir.");
        return Ok(());
    }
    eprintln!("Accounts:");
    for a in accounts {
        eprintln!("  {}  {}  ({})", a.id, a.email, a.provider);
    }
    Ok(())
}

fn resolve_account(db: &Database, selector: &str) -> Result<emailops_lib::models::Account, Box<dyn std::error::Error>> {
    let accounts = db.list_accounts()?;
    accounts
        .into_iter()
        .find(|a| a.email == selector || a.id == selector)
        .ok_or_else(|| format!("account '{}' not found (try --list-accounts)", selector).into())
}

fn default_data_dir() -> PathBuf {
    if let Ok(env) = std::env::var("EMAILOPS_DATA_DIR") {
        return PathBuf::from(env);
    }
    if let Some(home) = dirs::home_dir() {
        return home.join("Library/Application Support/com.emailops.app");
    }
    PathBuf::from(".")
}

fn default_out_dir(provider: &str, raw: bool) -> PathBuf {
    let root = PathBuf::from("src-tauri/tests/fixtures/cassettes");
    if raw {
        root.join("raw").join(provider)
    } else {
        root.join(provider)
    }
}

// ── Provider record sequences ────────────────────────────────────────────────

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

async fn record_outlook(tokens: &OAuthTokens, limit: u32) -> Result<Vec<Interaction>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mut interactions = Vec::new();

    // 1. List N most-recent inbox messages.
    let list_url = format!(
        "{}/me/mailFolders/inbox/messages?$top={}&$select=id,conversationId&$orderby=receivedDateTime desc",
        GRAPH_API_BASE, limit
    );
    let list_interaction = record_get(&client, &tokens.access_token, &list_url).await?;
    let message_ids: Vec<String> = list_interaction
        .response
        .body_json
        .as_ref()
        .and_then(|v| v.get("value"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    interactions.push(list_interaction);

    // 2. Fetch full body for each message.
    for id in message_ids {
        let url = format!(
            "{}/me/messages/{}?$select=id,conversationId,internetMessageId,subject,bodyPreview,body,from,toRecipients,ccRecipients,receivedDateTime,isRead,hasAttachments,inferenceClassification",
            GRAPH_API_BASE,
            urlencoding::encode(&id)
        );
        interactions.push(record_get(&client, &tokens.access_token, &url).await?);
    }

    Ok(interactions)
}

async fn record_gmail(tokens: &OAuthTokens, limit: u32) -> Result<Vec<Interaction>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mut interactions = Vec::new();

    let list_url = format!("{}/users/me/messages?maxResults={}", GMAIL_API_BASE, limit);
    let list_interaction = record_get(&client, &tokens.access_token, &list_url).await?;
    let ids: Vec<String> = list_interaction
        .response
        .body_json
        .as_ref()
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    interactions.push(list_interaction);

    for id in ids {
        let url = format!("{}/users/me/messages/{}?format=full", GMAIL_API_BASE, id);
        interactions.push(record_get(&client, &tokens.access_token, &url).await?);
    }

    Ok(interactions)
}

// ── HTTP record helper ───────────────────────────────────────────────────────

async fn record_get(
    client: &reqwest::Client,
    access_token: &str,
    url: &str,
) -> Result<Interaction, Box<dyn std::error::Error>> {
    let (url_path, query_params) = parse_url(url)?;
    let response = client
        .request(Method::GET, url)
        .bearer_auth(access_token)
        .send()
        .await?;
    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect();
    let bytes = response.bytes().await?;
    let body_json: Option<serde_json::Value> = if bytes.is_empty() {
        None
    } else {
        serde_json::from_slice(&bytes).ok()
    };
    Ok(Interaction {
        request: RecordedRequest {
            method: "GET".into(),
            url_path,
            query_params,
        },
        response: RecordedResponse {
            status,
            headers,
            body_json,
        },
    })
}

type ParsedUrl = (String, Vec<(String, String)>);

fn parse_url(url: &str) -> Result<ParsedUrl, Box<dyn std::error::Error>> {
    let parsed = url::Url::parse(url)?;
    let path = parsed.path().to_string();
    let qp: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    Ok((path, qp))
}
