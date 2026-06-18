// `attachment_reextract` — diagnose / repair an email whose attachments were
// missed at sync time. Re-fetches ONE message from its provider with the full
// MIME payload and runs the same extraction the sync path uses
// (`EmailProvider::get_message` -> `collect_attachment_infos`), printing what it
// finds. With `--repair` it upserts the recovered attachment metadata into the
// real DB (incremental sync skips already-stored messages, so this is the only
// way to backfill them).
//
// CLOSE THE APP FIRST — this opens the real DB in place (copying a multi-GB
// mailbox is a non-starter). Diagnose only reads + hits the provider; --repair
// also writes the recovered metas back.
//
// Usage:
//   cargo run --manifest-path src-tauri/Cargo.toml --features eval \
//     --example attachment_reextract -- --email-id <id>           # diagnose
//   cargo run --manifest-path src-tauri/Cargo.toml --features eval \
//     --example attachment_reextract -- --email-id <id> --repair  # repair
//
// Flags: --email-id (required), --account <id|email>, --repair, --prod-db <path>.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use emailops_lib::db::Database;
use emailops_lib::evals::db_source::{prepare_eval_db, EvalDbMode};

#[derive(Parser, Debug)]
#[command(name = "attachment_reextract", about = "Re-extract a single email's attachments from its provider.", long_about = None)]
struct Args {
    #[arg(long)]
    email_id: String,
    #[arg(long)]
    account: Option<String>,
    /// Upsert the recovered attachment metas into the REAL DB (close the app first).
    #[arg(long)]
    repair: bool,
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
    // keyring 4 needs the native store selected before any keychain op — the app
    // does this in `run()`; a standalone binary must do it too (else
    // "No default store has been set"). Release build → real OS keychain.
    if let Err(e) = emailops_lib::services::keychain::init_native_store() {
        eprintln!("[attachment_reextract] keychain init failed: {e}");
        std::process::exit(1);
    }
    let prod_db = args
        .prod_db
        .clone()
        .or_else(default_prod_db)
        .unwrap_or_else(|| PathBuf::from("emailops.db"));
    if !prod_db.exists() {
        eprintln!("[attachment_reextract] DB not found at {}", prod_db.display());
        std::process::exit(2);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    if let Err(e) = rt.block_on(run(args, prod_db)) {
        eprintln!("[attachment_reextract] ERROR: {e}");
        std::process::exit(1);
    }
}

async fn run(args: Args, prod_db: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Open the real DB in place — copying is a non-starter on a multi-GB mailbox.
    // CLOSE THE APP FIRST (this opens the prod DB; --repair also writes to it).
    let prepared = prepare_eval_db(&prod_db, EvalDbMode::InPlaceDangerous, "attach-reextract")?;
    let db = Arc::new(Database::new(prepared.db_dir().to_path_buf())?);

    let accounts = db.list_accounts()?;
    let account = match args.account.as_deref() {
        Some(hint) => accounts
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(hint.trim()) || a.email.eq_ignore_ascii_case(hint.trim()))
            .cloned()
            .ok_or_else(|| format!("account '{hint}' not found"))?,
        None => {
            let enabled: Vec<_> = accounts.iter().filter(|a| a.enabled).cloned().collect();
            match enabled.len() {
                1 => enabled[0].clone(),
                0 => return Err("no enabled accounts".into()),
                _ => return Err("multiple enabled accounts — pass --account <id|email>".into()),
            }
        }
    };

    eprintln!(
        "[attachment_reextract] account={} ({}) email={} mode={}",
        account.email,
        account.provider,
        args.email_id,
        if args.repair {
            "REPAIR (writes real DB)"
        } else {
            "diagnose (read-only copy)"
        }
    );

    // Existing rows in the DB for this email (the "before").
    let before = db.get_email_attachment_metas(&args.email_id)?;
    println!("DB currently has {} attachment row(s) for this email.", before.len());

    let provider = emailops_lib::services::emails::build_provider(&account, None).await?;
    let (_email, _category, infos) = provider.get_message(&args.email_id).await?;

    println!("\nProvider get_message returned {} attachment(s):", infos.len());
    for i in &infos {
        println!(
            "  - {}  ({}, {} bytes)  attachment_id={}  inline={}",
            i.filename,
            i.mime_type,
            i.size,
            if i.attachment_id.is_empty() {
                "<none>"
            } else {
                "<present>"
            },
            if i.inline_data.is_some() { "yes" } else { "no" },
        );
    }
    if infos.is_empty() {
        println!("\n⚠ The provider also reports ZERO attachments — the gap is in the");
        println!("  parser (collect_attachment_infos), not a missed fetch. Capture the");
        println!("  raw payload structure next.");
    }

    if args.repair {
        if infos.is_empty() {
            println!("\nNothing to repair (provider returned no attachments).");
        } else {
            let n =
                emailops_lib::services::attachments::reextract_email_attachments(&db, &account, &args.email_id, None)
                    .await?
                    .len();
            let after = db.get_email_attachment_metas(&args.email_id)?;
            println!(
                "\n✓ Repaired: {n} attachment(s) extracted; DB now has {} row(s).",
                after.len()
            );
        }
    } else {
        println!("\n(diagnose only — pass --repair, with the app closed, to backfill the DB)");
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
