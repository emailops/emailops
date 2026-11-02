//! CLI tool for quickly testing the list a search runs through
//!
//! Usage:
//!   cargo run --bin search_test "your search query here"
//!   cargo run --bin search_test --rebuild   # Rebuild FTS + embeddings first

use clap::Parser;
use emailops_lib::db::emails::list::ListView;
use emailops_lib::db::Database;
use emailops_lib::services::{embeddings, list};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "search_test")]
#[command(about = "Test a search from the command line")]
struct Args {
    /// Search query
    #[arg(default_value = "")]
    query: String,

    /// Rebuild FTS index and embeddings before searching
    #[arg(long)]
    rebuild: bool,

    /// Show detailed scores
    #[arg(long, short)]
    verbose: bool,
}

fn get_data_dir() -> PathBuf {
    // Try standard Tauri app data location
    if let Some(data_dir) = dirs::data_dir() {
        let app_data = data_dir.join("com.emailops.dev");
        if app_data.exists() {
            return app_data;
        }
    }
    // macOS specific path
    if let Some(home) = dirs::home_dir() {
        let app_data = home.join("Library/Application Support/com.emailops.dev");
        if app_data.exists() {
            return app_data;
        }
    }
    // Fallback to current directory
    PathBuf::from(".")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let data_dir = get_data_dir();

    println!("Database: {}", data_dir.display());

    let db = Arc::new(Database::new(data_dir)?);
    let accounts = db.list_accounts()?;
    let account_id = accounts
        .first()
        .map(|account| account.id.clone())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No accounts found in database"))?;

    // Show stats
    let email_count = db.count_emails(emailops_lib::db::AccountScope::Account(&account_id), None, false)?;
    let pending = db.count_emails_without_embeddings(Some(&account_id))?;
    let embedding_count = email_count - pending;
    println!("Emails: {}, Embeddings: {}", email_count, embedding_count);

    // Rebuild if requested
    if args.rebuild {
        println!("\nRebuilding search index...");

        // Rebuild FTS
        let fts_count = db.rebuild_fts_index()?;
        println!("  FTS index: {} emails", fts_count);

        // Regenerate embeddings
        println!("  Generating embeddings (this may take a while)...");
        let emb_count = embeddings::regenerate_embeddings(&db, None, None, 500, None).await?;
        println!("  Embeddings: {} generated", emb_count);
    }

    // If no query, just show stats and exit
    if args.query.is_empty() {
        println!("\nUsage: cargo run --bin search_test \"your query here\"");
        println!("       cargo run --bin search_test --rebuild  # rebuild index first");
        return Ok(());
    }

    // Perform search
    println!("\nSearching: \"{}\"", args.query);
    println!("{}", "-".repeat(60));

    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = list::list_threads(
            &db,
            list::ListRequest {
                account_id: Some(&account_id),
                query: &args.query,
                view: ListView::Inbox,
                unread_only: false,
                cursor,
                limit: 50,
            },
        )?;
        rows.extend(page.emails);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    println!("Results: {}\n", rows.len());

    for (i, email) in rows.iter().enumerate() {
        println!("{}. {}", i + 1, email.subject);
        println!("   From: {}", email.sender);

        if args.verbose {
            println!("   Snippet: {}...", email.snippet.chars().take(80).collect::<String>());
        }
        println!();
    }

    if rows.is_empty() {
        println!("No results found.");
    }

    Ok(())
}
