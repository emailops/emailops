//! One-shot CLI that derives and persists the `tag_type='company'`
//! `email_tags` row for every email in a given account.
//!
//! ```bash
//! # List accounts to find the id
//! cargo run --bin company_backfill -- --list-accounts
//!
//! # Backfill by account email (what the user normally has at hand)
//! cargo run --bin company_backfill -- --account-email alex@northwindlabs.io
//!
//! # Or by account id
//! cargo run --bin company_backfill -- --account-id <uuid>
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use emailops_lib::services::email_company;
use emailops_lib::Database;

#[derive(Parser)]
#[command(name = "company_backfill")]
#[command(about = "Backfill company tags for an account's emails")]
struct Cli {
    /// Account email (e.g. alex@northwindlabs.io). Preferred — human-friendly.
    #[arg(long)]
    account_email: Option<String>,

    /// Account id (UUID). Overrides --account-email when both given.
    #[arg(long)]
    account_id: Option<String>,

    /// List all accounts in the database and exit.
    #[arg(long)]
    list_accounts: bool,
}

fn data_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let p = home.join("Library/Application Support/com.emailops.app");
        if p.exists() {
            return p;
        }
    }
    if let Some(d) = dirs::data_dir() {
        let p = d.join("com.emailops.app");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(".")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let dir = data_dir();
    println!("Opening DB at {}", dir.display());
    let db = Arc::new(Database::new(dir)?);

    if cli.list_accounts {
        for acc in db.list_accounts()? {
            let count = db.count_emails(&acc.id).unwrap_or(0);
            println!("  {}  {:<40}  {}  threads={}", acc.id, acc.email, acc.provider, count);
        }
        return Ok(());
    }

    let account_id = match (cli.account_id.clone(), cli.account_email.clone()) {
        (Some(id), _) => id,
        (None, Some(email)) => db
            .list_accounts()?
            .into_iter()
            .find(|a| a.email.eq_ignore_ascii_case(&email))
            .map(|a| a.id)
            .ok_or_else(|| format!("No account found with email {email}"))?,
        (None, None) => {
            eprintln!("Provide --account-email or --account-id (or --list-accounts).");
            std::process::exit(2);
        }
    };

    println!("Backfilling company tags for account {account_id}");
    let n = email_company::backfill_account(&db, None, &account_id)?;
    println!("Done. Tagged {n} email(s).");
    Ok(())
}
