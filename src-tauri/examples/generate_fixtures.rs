//! CLI tool to generate test fixtures from the EmailOps database.
//!
//! # Usage
//!
//! ```bash
//! # List all accounts
//! cargo run --bin generate_fixtures -- --list-accounts
//!
//! # Export fixtures for a specific account
//! cargo run --bin generate_fixtures -- --account-id <id> --output tests/fixtures/
//!
//! # Export with a limit on number of emails
//! cargo run --bin generate_fixtures -- --account-id <id> --limit 50 --output tests/fixtures/
//! ```

use clap::{Parser, Subcommand};
// Embeddings are now stored in sqlite-vec; fixture export skips them
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct EmailEmbedding {
    email_id: String,
    embedding_model: String,
    content_hash: String,
}
use emailops_lib::models::Account;
use emailops_lib::Database;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "generate_fixtures")]
#[command(about = "Generate test fixtures from EmailOps database")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Account ID to export
    #[arg(long)]
    account_id: Option<String>,

    /// Output directory for fixtures
    #[arg(long, short)]
    output: Option<PathBuf>,

    /// Maximum number of emails to export
    #[arg(long, default_value = "1000")]
    limit: i32,

    /// List all accounts
    #[arg(long)]
    list_accounts: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// List all accounts in the database
    ListAccounts,
    /// Export fixtures for an account
    Export {
        /// Account ID to export
        #[arg(long)]
        account_id: String,
        /// Output directory
        #[arg(long, short)]
        output: PathBuf,
        /// Maximum emails to export
        #[arg(long, default_value = "1000")]
        limit: i32,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportMetadata {
    account: Account,
    export_timestamp: i64,
    email_count: usize,
    embedding_count: usize,
}

fn get_data_dir() -> PathBuf {
    // Try to use the same directory as Tauri app
    if let Some(data_dir) = dirs::data_dir() {
        let app_data = data_dir.join("com.emailops.app");
        if app_data.exists() {
            return app_data;
        }
    }

    // Fallback to home directory
    if let Some(home) = dirs::home_dir() {
        let app_data = home.join("Library/Application Support/com.emailops.app");
        if app_data.exists() {
            return app_data;
        }
    }

    // Last resort: current directory
    PathBuf::from(".")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let data_dir = get_data_dir();

    println!("Using database from: {}", data_dir.display());

    let db = Database::new(data_dir)?;

    // Handle --list-accounts flag
    if cli.list_accounts {
        return list_accounts(&db);
    }

    // Handle subcommands
    match cli.command {
        Some(Commands::ListAccounts) => {
            list_accounts(&db)?;
        }
        Some(Commands::Export {
            account_id,
            output,
            limit,
        }) => {
            export_fixtures(&db, &account_id, &output, limit)?;
        }
        None => {
            // Handle positional args style
            if let (Some(account_id), Some(output)) = (cli.account_id, cli.output) {
                export_fixtures(&db, &account_id, &output, cli.limit)?;
            } else {
                eprintln!("Error: Either use --list-accounts or provide --account-id and --output");
                eprintln!("\nUsage:");
                eprintln!("  generate_fixtures --list-accounts");
                eprintln!("  generate_fixtures --account-id <ID> --output <DIR>");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn list_accounts(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let accounts = db.list_accounts()?;

    if accounts.is_empty() {
        println!("No accounts found in database.");
        return Ok(());
    }

    println!("\nAvailable accounts:");
    println!("{:-<60}", "");
    for account in accounts {
        let email_count = db.count_emails(emailops_lib::db::AccountScope::Account(&account.id), None)?;
        println!(
            "ID: {}\n  Email: {}\n  Provider: {}\n  Emails: {}\n",
            account.id, account.email, account.provider, email_count
        );
    }

    Ok(())
}

fn export_fixtures(
    db: &Database,
    account_id: &str,
    output_dir: &PathBuf,
    limit: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    // Verify account exists
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| format!("Account not found: {}", account_id))?;

    println!("Exporting fixtures for account: {}", account.email);

    // Create output directory
    fs::create_dir_all(output_dir)?;

    // Fetch emails
    println!("Fetching emails (limit: {})...", limit);
    let emails = db.get_emails(
        emailops_lib::db::AccountScope::Account(account_id),
        limit,
        0,
        None,
        None,
        None,
    )?;
    println!("  Found {} emails", emails.len());

    // Embeddings now use sqlite-vec (vec0) — skip export
    let embeddings: Vec<EmailEmbedding> = Vec::new();
    println!("  Embeddings stored in sqlite-vec (not exported)");

    // Create metadata
    let metadata = ExportMetadata {
        account: account.clone(),
        export_timestamp: chrono::Utc::now().timestamp(),
        email_count: emails.len(),
        embedding_count: embeddings.len(),
    };

    // Write files
    let emails_path = output_dir.join("emails.json");
    let embeddings_path = output_dir.join("embeddings.json");
    let metadata_path = output_dir.join("metadata.json");

    println!("\nWriting files...");

    fs::write(&emails_path, serde_json::to_string_pretty(&emails)?)?;
    println!("  Wrote {}", emails_path.display());

    fs::write(&embeddings_path, serde_json::to_string_pretty(&embeddings)?)?;
    println!("  Wrote {}", embeddings_path.display());

    fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)?;
    println!("  Wrote {}", metadata_path.display());

    println!("\nExport complete!");

    Ok(())
}
