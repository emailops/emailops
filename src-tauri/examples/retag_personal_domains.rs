//! One-shot CLI that migrates `email_tags(tag_type='company')` rows whose
//! `tag_value` is a bare-domain shortname of a known personal-email provider
//! (e.g. `gmail`, `outlook`, `yahoo`) to the new per-address vocabulary
//! produced by [`emailops_lib::services::email_company::derive_company_tag`].
//!
//! The migration is three steps, all run inside this binary:
//!
//! 1. [`email_company::retag_personal_domains`] — drop the stale shortname
//!    rows from `email_tags` (and `tag_priority`).
//! 2. [`email_company::backfill_account`] — re-derive company tags for the
//!    now-untagged emails (yielding `alice@gmail.com` instead of `gmail`).
//! 3. [`tag_priority::rebuild_account_tag_type`] — recompute every
//!    `tag_priority` row for `tag_type='company'` since the vocabulary
//!    changed.
//!
//! ```bash
//! # List accounts to find the id
//! cargo run --bin retag_personal_domains -- --list-accounts
//!
//! # Run for a single account
//! cargo run --bin retag_personal_domains -- --account-email alex@example.com
//!
//! # Or for every account
//! cargo run --bin retag_personal_domains -- --all
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use emailops_lib::services::{email_company, tag_priority};
use emailops_lib::Database;

#[derive(Parser)]
#[command(name = "retag_personal_domains")]
#[command(about = "Migrate company tags from bare-domain to per-address for personal-mail providers")]
struct Cli {
    /// Account email (e.g. alex@example.com). Preferred — human-friendly.
    #[arg(long)]
    account_email: Option<String>,

    /// Account id (UUID). Overrides --account-email when both given.
    #[arg(long)]
    account_id: Option<String>,

    /// Run for every account in the DB.
    #[arg(long)]
    all: bool,

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

fn run_for(db: &Arc<Database>, account_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== account {account_id} ===");

    print!("  1/3 retag_personal_domains ... ");
    let deleted = email_company::retag_personal_domains(db, account_id)?;
    println!("dropped {deleted} stale email_tags row(s)");

    print!("  2/3 email_company::backfill_account ... ");
    let tagged = email_company::backfill_account(db, None, account_id)?;
    println!("tagged {tagged} email(s)");

    print!("  3/3 tag_priority::rebuild_account_tag_type(company) ... ");
    let rebuilt = tag_priority::rebuild_account_tag_type(db, account_id, "company")?;
    println!("rebuilt {rebuilt} tag_priority row(s)");

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let dir = data_dir();
    println!("Opening DB at {}", dir.display());
    let db = Arc::new(Database::new(dir)?);

    if cli.list_accounts {
        for acc in db.list_accounts()? {
            let count = db
                .count_emails(emailops_lib::db::AccountScope::Account(&acc.id), None)
                .unwrap_or(0);
            println!("  {}  {:<40}  {}  threads={}", acc.id, acc.email, acc.provider, count);
        }
        return Ok(());
    }

    if cli.all {
        for acc in db.list_accounts()? {
            run_for(&db, &acc.id)?;
        }
        println!("\nDone.");
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
            eprintln!("Provide --account-email, --account-id, or --all (or --list-accounts).");
            std::process::exit(2);
        }
    };

    run_for(&db, &account_id)?;
    println!("\nDone.");
    Ok(())
}
