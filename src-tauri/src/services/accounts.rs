use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use uuid::Uuid;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, OAuthTokens};
use crate::sync::gmail::GmailClient;
use crate::sync::imap::{ImapClient, ImapCredentials};
use crate::sync::oauth::{self, OAuthConfig};
use crate::sync::outlook::OutlookClient;
use crate::sync::provider::EmailProvider;

/// Build an OAuth-based email provider for the given provider name.
/// Returns an error for unsupported providers (e.g. "imap" — use ImapClient directly).
fn build_oauth_provider(
    provider_name: &str,
    access_token: String,
    refresh_token: Option<String>,
    app: Option<tauri::AppHandle>,
    account_id: Option<String>,
) -> Result<Box<dyn EmailProvider>> {
    match provider_name {
        "gmail" => Ok(Box::new(GmailClient::new(access_token, refresh_token, app, account_id))),
        "outlook" => Ok(Box::new(OutlookClient::new(
            access_token,
            refresh_token,
            app,
            account_id,
        ))),
        other => Err(AppError::InvalidInput(format!("Unsupported OAuth provider: {}", other))),
    }
}

const KEYRING_SERVICE: &str = "emailops";

// In-memory token cache to reduce keyring access
static TOKEN_CACHE: std::sync::LazyLock<RwLock<HashMap<String, OAuthTokens>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));
static TOKEN_DB: std::sync::LazyLock<RwLock<Option<Arc<Database>>>> = std::sync::LazyLock::new(|| RwLock::new(None));

fn use_dev_tokens() -> bool {
    cfg!(debug_assertions)
}

pub async fn add_account(db: &Arc<Database>, provider_name: &str, sync_from_timestamp: Option<i64>) -> Result<Account> {
    let config = OAuthConfig::for_provider(provider_name);

    // Start OAuth flow
    let tokens = oauth::start_oauth_flow(&config).await?;

    // Get user profile via provider
    let provider = build_oauth_provider(
        provider_name,
        tokens.access_token.clone(),
        tokens.refresh_token.clone(),
        None,
        None,
    )?;
    let (email, name): (String, String) = provider.get_profile().await?;

    // Check if account already exists
    if db.account_exists_by_email(&email)? {
        return Err(AppError::InvalidInput(format!(
            "Account {} is already connected",
            email
        )));
    }

    let account = Account {
        id: Uuid::new_v4().to_string(),
        provider: provider_name.to_string(),
        email,
        name,
        created_at: chrono::Utc::now().timestamp(),
        sort_order: 0,
        enabled: true,
        sync_from_timestamp,
    };

    persist_oauth_account(db, &account, &tokens)?;

    Ok(account)
}

/// Persist a new OAuth account and its tokens, account row first.
///
/// Mirrors [`persist_imap_account`]: in dev mode tokens land in `dev_tokens`
/// (no FK today, but the account-first order keeps the two paths consistent and
/// future-proofs against an FK being added). If token storage fails, the
/// orphaned account row is rolled back so a retry isn't blocked by
/// `account_exists_by_email`.
fn persist_oauth_account(db: &Arc<Database>, account: &Account, tokens: &OAuthTokens) -> Result<()> {
    db.insert_account(account)?;

    if let Err(e) = store_tokens(&account.id, tokens) {
        if let Err(cleanup_err) = db.delete_account(&account.id) {
            eprintln!(
                "[accounts] failed to roll back account {} after token storage error: {cleanup_err}",
                account.id
            );
        }
        return Err(e);
    }

    Ok(())
}

pub fn list_accounts(db: &Arc<Database>) -> Result<Vec<Account>> {
    db.list_accounts()
}

pub fn remove_account(db: &Arc<Database>, account_id: &str, app_data_dir: &std::path::Path) -> Result<()> {
    // Determine provider before deletion
    let is_imap = db
        .get_account(account_id)
        .ok()
        .flatten()
        .map(|a| a.provider == "imap")
        .unwrap_or(false);

    // Delete credentials from keyring
    if is_imap {
        let _ = delete_imap_credentials(account_id);
    } else {
        delete_tokens(account_id)?;
    }

    // Delete from database (cascades to attachments table rows)
    db.delete_account(account_id)?;

    // Delete attachment files from disk
    let att_dir = app_data_dir.join("attachments").join(account_id);
    if att_dir.exists() {
        let _ = std::fs::remove_dir_all(&att_dir);
    }

    Ok(())
}

/// Re-authenticate an existing account by triggering OAuth flow and updating tokens.
/// Only valid for OAuth providers (gmail, outlook). IMAP accounts must update
/// credentials via `update_imap_credentials` instead — this function rejects
/// them so a stale OAuth fallback config (gmail) can't pop up the wrong
/// browser flow for an IMAP account.
pub async fn reauthenticate_account(db: &Arc<Database>, account_id: &str) -> Result<()> {
    // Verify account exists
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    if account.provider == "imap" {
        return Err(AppError::InvalidInput(
            "IMAP accounts cannot be re-authenticated via OAuth. \
             Update credentials in Account Settings instead."
                .to_string(),
        ));
    }

    let config = OAuthConfig::for_provider(&account.provider);

    // Start OAuth flow to get new tokens
    let tokens = oauth::start_oauth_flow(&config).await?;

    // Verify the email matches the existing account
    let provider = build_oauth_provider(
        &account.provider,
        tokens.access_token.clone(),
        tokens.refresh_token.clone(),
        None,
        None,
    )?;
    let (email, _name): (String, String) = provider.get_profile().await?;

    if email != account.email {
        return Err(AppError::InvalidInput(format!(
            "Email mismatch: expected {}, got {}. Please sign in with the correct account.",
            account.email, email
        )));
    }

    // Store new tokens
    store_tokens(account_id, &tokens)?;

    // Drop the scheduler's dedup state so the next auto-sync tick reports
    // again from scratch — the user has just taken action and a stale dedup
    // entry would otherwise silence a *new* error that surfaces post-reauth.
    crate::services::sync_scheduler::clear_sync_error_dedup(account_id);

    Ok(())
}

pub fn reorder_accounts(db: &Arc<Database>, account_ids: &[String]) -> Result<()> {
    db.update_account_order(account_ids)
}

pub fn set_account_enabled(db: &Arc<Database>, account_id: &str, enabled: bool) -> Result<()> {
    db.update_account_enabled(account_id, enabled)
}

pub fn update_account_sync_from(
    db: &Arc<Database>,
    account_id: &str,
    sync_from_timestamp: Option<i64>,
) -> Result<Account> {
    db.get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    db.update_account_sync_from(account_id, sync_from_timestamp)?;

    db.get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found after update", account_id)))
}

/// Register the database that backs dev-mode credential storage.
///
/// Every credential read goes through this global in debug builds, so any
/// process that resolves account credentials must bind it before syncing or
/// chatting — including headless ones (`emailops-cli`) that never warm the
/// cache. Binding is deliberately separate from [`warm_token_cache`]: warming
/// reads the keychain for every account, which a one-shot CLI command has no
/// reason to pay for.
pub fn bind_credential_db(db: &Arc<Database>) {
    let mut db_ref = TOKEN_DB.write().unwrap_or_else(PoisonError::into_inner);
    *db_ref = Some(Arc::clone(db));
}

/// Pre-load tokens for all accounts into the in-memory cache.
/// Call once at startup so the keychain is accessed in a single batch,
/// producing at most one macOS authorization prompt.
pub fn warm_token_cache(db: &Arc<Database>) {
    bind_credential_db(db);

    #[cfg(debug_assertions)]
    eprintln!("[dev] Using SQLite for credential storage (no keychain)");

    let accounts = match db.list_accounts() {
        Ok(a) => a,
        Err(_) => return,
    };
    for account in &accounts {
        if account.provider == "imap" {
            continue; // IMAP credentials are fetched on demand, not cached as OAuthTokens
        }
        let _ = get_tokens(&account.id);
    }
}

pub fn get_tokens(account_id: &str) -> Result<OAuthTokens> {
    // Check cache first
    {
        let cache = TOKEN_CACHE.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(tokens) = cache.get(account_id) {
            return Ok(tokens.clone());
        }
    }

    let json = if use_dev_tokens() {
        // Read from SQLite
        let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
        let db = db_ref
            .as_ref()
            .ok_or_else(|| AppError::KeyringError("DB not initialized for dev token storage".to_string()))?;
        // Missing dev-token row → user-recoverable: needs re-auth.
        db.get_dev_tokens(account_id)?.ok_or_else(|| AppError::NeedsReauth {
            account_id: account_id.to_string(),
        })?
    } else {
        // Read via the secrets vault (single keychain item → single macOS
        // prompt). `Ok(None)` means the entry is missing (user-recoverable,
        // needs re-auth); `Err(...)` means the keychain backend itself failed
        // (infrastructure problem the user can't fix).
        match super::secrets_vault::get(KEYRING_SERVICE, account_id)? {
            Some(p) => p,
            None => {
                return Err(AppError::NeedsReauth {
                    account_id: account_id.to_string(),
                });
            }
        }
    };

    let tokens: OAuthTokens = serde_json::from_str(&json).map_err(AppError::JsonError)?;

    // Store in cache
    {
        let mut cache = TOKEN_CACHE.write().unwrap_or_else(PoisonError::into_inner);
        cache.insert(account_id.to_string(), tokens.clone());
    }

    Ok(tokens)
}

pub fn store_tokens(account_id: &str, tokens: &OAuthTokens) -> Result<()> {
    let json = serde_json::to_string(tokens)?;

    if use_dev_tokens() {
        // Store in SQLite
        let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
        let db = db_ref
            .as_ref()
            .ok_or_else(|| AppError::KeyringError("DB not initialized for dev token storage".to_string()))?;
        db.store_dev_tokens(account_id, &json)?;
    } else {
        // Store via the secrets vault.
        super::secrets_vault::set(KEYRING_SERVICE, account_id, &json)?;
    }

    // Update cache
    {
        let mut cache = TOKEN_CACHE.write().unwrap_or_else(PoisonError::into_inner);
        cache.insert(account_id.to_string(), tokens.clone());
    }

    Ok(())
}

fn delete_tokens(account_id: &str) -> Result<()> {
    // Remove from cache
    {
        let mut cache = TOKEN_CACHE.write().unwrap_or_else(PoisonError::into_inner);
        cache.remove(account_id);
    }

    if use_dev_tokens() {
        let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(db) = db_ref.as_ref() {
            db.delete_dev_tokens(account_id)?;
        }
    } else {
        super::secrets_vault::delete(KEYRING_SERVICE, account_id)?;
    }

    Ok(())
}

// ── IMAP credential helpers ───────────────────────────────────────────────────

const IMAP_KEYRING_PREFIX: &str = "emailops-imap";

pub fn get_imap_credentials(account_id: &str) -> Result<ImapCredentials> {
    if use_dev_tokens() {
        let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
        let db = db_ref
            .as_ref()
            .ok_or_else(|| AppError::KeyringError("DB not initialized for dev credential storage".to_string()))?;
        let json = db
            .get_dev_imap_creds(account_id)?
            .ok_or_else(|| AppError::NeedsReauth {
                account_id: account_id.to_string(),
            })?;
        return serde_json::from_str(&json).map_err(AppError::JsonError);
    }
    let json = super::secrets_vault::get(IMAP_KEYRING_PREFIX, account_id)?.ok_or_else(|| AppError::NeedsReauth {
        account_id: account_id.to_string(),
    })?;
    serde_json::from_str(&json).map_err(AppError::JsonError)
}

pub fn store_imap_credentials(account_id: &str, creds: &ImapCredentials) -> Result<()> {
    let json = serde_json::to_string(creds)?;
    if use_dev_tokens() {
        let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
        let db = db_ref
            .as_ref()
            .ok_or_else(|| AppError::KeyringError("DB not initialized for dev credential storage".to_string()))?;
        // Mirror non-secret fields to imap_account_settings so the re-auth
        // dialog can pre-fill them even if dev_imap_creds is later cleared.
        db.upsert_imap_settings(
            account_id,
            &creds.host,
            creds.port,
            &creds.username,
            &creds.smtp_host,
            creds.smtp_port,
        )?;
        return db.store_dev_imap_creds(account_id, &json);
    }
    super::secrets_vault::set(IMAP_KEYRING_PREFIX, account_id, &json)?;
    // Mirror non-secret fields to imap_account_settings (see comment above).
    // Done after the keychain write so a keychain failure short-circuits before
    // we record settings for an account whose password we couldn't save.
    let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
    if let Some(db) = db_ref.as_ref() {
        db.upsert_imap_settings(
            account_id,
            &creds.host,
            creds.port,
            &creds.username,
            &creds.smtp_host,
            creds.smtp_port,
        )?;
    }
    Ok(())
}

/// IMAP server settings for the re-auth/edit dialog. Always returns the
/// non-secret fields (loaded from the DB, falling back to the keychain blob
/// for legacy accounts). `password` is populated only when the keychain entry
/// is intact; otherwise it is empty and `has_password` is false so the UI can
/// prompt the user to retype it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapEditSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub has_password: bool,
}

/// Load IMAP settings for the re-auth dialog. Pulls server settings from the DB
/// (falling back to the keychain blob when the DB row is missing — typical for
/// accounts created before the DB-backed settings table existed) and the
/// password from the keychain. Returns partial data with `has_password: false`
/// when only the password is missing, so the dialog can pre-fill server fields
/// and prompt for the password instead of opening empty.
pub fn load_imap_settings_for_edit(db: &Arc<Database>, account_id: &str) -> Result<ImapEditSettings> {
    // Try keychain first — if it succeeds we have everything *and* can
    // opportunistically backfill the DB for next time.
    match get_imap_credentials(account_id) {
        Ok(creds) => {
            // Backfill DB so future loads don't depend on the keychain entry.
            let _ = db.upsert_imap_settings(
                account_id,
                &creds.host,
                creds.port,
                &creds.username,
                &creds.smtp_host,
                creds.smtp_port,
            );
            Ok(ImapEditSettings {
                host: creds.host,
                port: creds.port,
                username: creds.username,
                password: creds.password,
                smtp_host: creds.smtp_host,
                smtp_port: creds.smtp_port,
                has_password: true,
            })
        }
        Err(AppError::NeedsReauth { .. }) => {
            // Password missing — fall back to DB settings so the user only has
            // to re-enter their password. If the DB also has nothing, the
            // dialog gets blank server fields *with* `has_password = false` so
            // it can show a clear "credentials missing — please enter" state
            // rather than silently displaying empty inputs.
            let settings = db.get_imap_settings(account_id)?;
            let (host, port, username, smtp_host, smtp_port) =
                settings.unwrap_or_else(|| (String::new(), 993, String::new(), String::new(), 465));
            Ok(ImapEditSettings {
                host,
                port,
                username,
                password: String::new(),
                smtp_host,
                smtp_port,
                has_password: false,
            })
        }
        Err(e) => Err(e),
    }
}

/// Backfill `imap_account_settings` for accounts whose keychain entry still
/// exists but predates the DB-backed table. Best-effort: failures for any
/// single account are logged and skipped so a stuck keychain can't block app
/// startup.
pub fn backfill_imap_settings(db: &Arc<Database>) {
    let accounts = match db.list_accounts() {
        Ok(a) => a,
        Err(e) => {
            crate::services::logger::log("error", "account", format!("imap-backfill: list_accounts failed: {e}"));
            return;
        }
    };
    for account in accounts.into_iter().filter(|a| a.provider == "imap") {
        match db.get_imap_settings(&account.id) {
            Ok(Some(_)) => continue, // already migrated
            Ok(None) => {}
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "account",
                    format!("imap-backfill: get_imap_settings({}) failed: {e}", account.id),
                );
                continue;
            }
        }
        match get_imap_credentials(&account.id) {
            Ok(creds) => {
                if let Err(e) = db.upsert_imap_settings(
                    &account.id,
                    &creds.host,
                    creds.port,
                    &creds.username,
                    &creds.smtp_host,
                    creds.smtp_port,
                ) {
                    crate::services::logger::log(
                        "error",
                        "account",
                        format!("imap-backfill: upsert for {} failed: {e}", account.id),
                    );
                }
            }
            Err(AppError::NeedsReauth { .. }) => {
                // Nothing in the keychain either — nothing to migrate. The
                // user will have to re-enter everything when they next open
                // the re-auth dialog.
            }
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "account",
                    format!("imap-backfill: keychain read for {} failed: {e}", account.id),
                );
            }
        }
    }
}

fn delete_imap_credentials(account_id: &str) -> Result<()> {
    if use_dev_tokens() {
        let db_ref = TOKEN_DB.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(db) = db_ref.as_ref() {
            db.delete_dev_imap_creds(account_id)?;
        }
        return Ok(());
    }
    super::secrets_vault::delete(IMAP_KEYRING_PREFIX, account_id)
}

/// Test IMAP + SMTP credentials without saving anything.
/// Returns `Ok(())` if both succeed, or an `Err` describing the first failure.
pub async fn test_imap_connection(credentials: ImapCredentials) -> Result<()> {
    let client = ImapClient::new(
        credentials.clone(),
        credentials.username.clone(),
        String::new(),
        String::new(),
    );
    client.test_connection().await
}

/// Add an IMAP account. Verifies credentials by logging in before saving.
pub async fn add_imap_account(
    db: &Arc<Database>,
    credentials: ImapCredentials,
    display_name: Option<String>,
    sync_from_timestamp: Option<i64>,
) -> Result<Account> {
    let email = credentials.username.clone();
    let display = display_name.unwrap_or_else(|| email.clone());

    // Verify credentials work by actually connecting (no account_id needed for login test)
    let client = ImapClient::new(credentials.clone(), email.clone(), display.clone(), String::new());
    client.login_raw().await?;

    // Check if account already exists
    if db.account_exists_by_email(&email)? {
        return Err(AppError::InvalidInput(format!(
            "Account {} is already connected",
            email
        )));
    }

    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "imap".to_string(),
        email,
        name: display,
        created_at: chrono::Utc::now().timestamp(),
        sort_order: 0,
        enabled: true,
        sync_from_timestamp,
    };

    persist_imap_account(db, &account, &credentials)?;
    Ok(account)
}

/// Persist a new IMAP account and its credentials.
///
/// Order matters: `imap_account_settings` and `dev_imap_creds` carry a
/// `FOREIGN KEY` to `accounts(id)`, so the account row must exist before
/// `store_imap_credentials` runs. If credential storage fails, the orphaned
/// account row is rolled back so a retry isn't blocked by `account_exists_by_email`.
fn persist_imap_account(db: &Arc<Database>, account: &Account, credentials: &ImapCredentials) -> Result<()> {
    db.insert_account(account)?;

    if let Err(e) = store_imap_credentials(&account.id, credentials) {
        // Best-effort rollback of the account row we just inserted. If cleanup
        // also fails, surface the original credential error — it's the more
        // actionable one — but record the cleanup failure so the orphan is visible.
        if let Err(cleanup_err) = db.delete_account(&account.id) {
            eprintln!(
                "[accounts] failed to roll back account {} after credential storage error: {cleanup_err}",
                account.id
            );
        }
        return Err(e);
    }

    Ok(())
}

/// Called by `remove_account` for IMAP accounts to clean up keychain entry.
pub fn remove_imap_credentials_on_delete(account_id: &str) {
    let _ = delete_imap_credentials(account_id);
}

// ── Per-account inbox category set ───────────────────────────────────────────

/// Pure decision: which inbox categories are valid filter chips for an account
/// of the given provider, given that account's saved `gmail_categories`
/// preference (only consulted for Gmail). The Inbox component renders one tab
/// per returned value.
///
/// * Gmail: the user's opted-in categories (Primary / Updates / Promotions /
///   Social / Forums). Defaults to ["primary"] if the user hasn't picked any.
/// * Outlook: the fixed two-tab mapping the sync layer assigns —
///   `inferenceClassification: focused → primary`, `other → updates`. See
///   `sync/outlook.rs` for the source of truth.
/// * IMAP / unknown: no categories. The Inbox UI hides the tab strip.
pub fn available_categories(provider: &str, gmail_categories: &[String]) -> Vec<String> {
    match provider {
        "gmail" => {
            if gmail_categories.is_empty() {
                vec!["primary".to_string()]
            } else {
                gmail_categories.to_vec()
            }
        }
        "outlook" => vec!["primary".to_string(), "updates".to_string()],
        _ => Vec::new(),
    }
}

/// Thin executor that loads the account + its saved settings and delegates
/// the decision to [`available_categories`]. Returns `NotFound` if the
/// account id doesn't exist.
pub fn available_categories_for_account(db: &Database, account_id: &str) -> Result<Vec<String>> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account '{}' not found", account_id)))?;
    let settings_key = format!("account_settings:{}", account_id);
    let gmail_categories = match db.get_preference(&settings_key)? {
        Some(json) => {
            let s: crate::models::AccountSettings = serde_json::from_str(&json).unwrap_or_default();
            s.gmail_categories
        }
        None => Vec::new(),
    };
    Ok(available_categories(&account.provider, &gmail_categories))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `store_imap_credentials` reads the process-global `TOKEN_DB` in dev mode,
    // so credential-store tests must not run concurrently against different DBs.
    static CRED_STORE_LOCK: Mutex<()> = Mutex::new(());

    fn imap_creds() -> ImapCredentials {
        ImapCredentials {
            host: "imap.example.com".into(),
            port: 993,
            username: "hello@example.com".into(),
            password: "app-password".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
        }
    }

    fn imap_account(id: &str, email: &str) -> Account {
        Account {
            id: id.into(),
            provider: "imap".into(),
            email: email.into(),
            name: email.into(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    // Regression: adding an IMAP account stored credentials *before* inserting
    // the account row. `imap_account_settings` has a FOREIGN KEY to accounts(id),
    // so the write failed with "FOREIGN KEY constraint failed" and no account
    // was ever created. The account row must be inserted first.
    #[test]
    fn persist_imap_account_inserts_account_before_credentials() {
        let _guard = CRED_STORE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        warm_token_cache(&db);

        let account = imap_account("acc-1", "hello@example.com");
        let creds = imap_creds();

        persist_imap_account(&db, &account, &creds).expect("persist should succeed");

        assert!(
            db.get_account("acc-1").expect("get_account").is_some(),
            "account row must exist after persisting"
        );
        assert!(
            db.get_imap_settings("acc-1").expect("get_imap_settings").is_some(),
            "imap_account_settings row must exist after persisting"
        );
    }

    // Regression: `emailops-cli` builds its own `Database` and never called
    // `warm_token_cache`, so the process-global credential DB stayed empty and
    // every credential read failed with "DB not initialized for dev credential
    // storage" — `emailops-cli sync` could not authenticate against ANY account
    // in a dev build. Binding must be separable from warming, because the CLI
    // must not touch the keychain for accounts a one-shot command never uses.
    #[test]
    fn bind_credential_db_makes_stored_imap_credentials_readable() {
        let _guard = CRED_STORE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        bind_credential_db(&db);

        let account = imap_account("cli-1", "hello@example.com");
        persist_imap_account(&db, &account, &imap_creds()).expect("persist should succeed");

        let creds = get_imap_credentials("cli-1").expect("credentials must resolve after binding");
        assert_eq!(creds.host, "imap.example.com");
        assert_eq!(creds.username, "hello@example.com");
    }

    // Consistency with the IMAP path: the account row must be inserted before
    // tokens are stored. Tokens land in `dev_tokens` (no FK today), but keeping
    // both account-creation paths account-first guards against future FK drift.
    #[test]
    fn persist_oauth_account_inserts_account_before_tokens() {
        let _guard = CRED_STORE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        warm_token_cache(&db);

        let account = imap_account("oauth-1", "gmail-user@example.com");
        let tokens = OAuthTokens {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: Some(0),
        };

        persist_oauth_account(&db, &account, &tokens).expect("persist should succeed");

        assert!(
            db.get_account("oauth-1").expect("get_account").is_some(),
            "account row must exist after persisting"
        );
        assert!(
            db.get_dev_tokens("oauth-1").expect("get_dev_tokens").is_some(),
            "dev_tokens row must exist after persisting"
        );
    }

    #[test]
    fn gmail_returns_users_opted_in_categories() {
        let got = available_categories("gmail", &["primary".into(), "updates".into()]);
        assert_eq!(got, vec!["primary", "updates"]);
    }

    #[test]
    fn gmail_with_empty_pref_defaults_to_primary() {
        // First-run / migrated DBs may have no saved categories. The UI must
        // still render at least the Primary tab so the inbox isn't blank.
        let got = available_categories("gmail", &[]);
        assert_eq!(got, vec!["primary"]);
    }

    #[test]
    fn outlook_returns_the_focused_other_pair_regardless_of_pref() {
        // Outlook ignores gmail_categories entirely — its tabs come from
        // inferenceClassification (focused / other), mapped 1:1 by the sync
        // layer to primary / updates. Even if a stray pref leaks in from a
        // re-used account row, it must not influence Outlook's tab set.
        let got = available_categories("outlook", &["promotions".into(), "social".into()]);
        assert_eq!(got, vec!["primary", "updates"]);
    }

    #[test]
    fn imap_has_no_category_tabs() {
        // IMAP servers don't expose a category taxonomy. The Inbox hides
        // the tab strip when this returns empty.
        let got = available_categories("imap", &["primary".into()]);
        assert!(got.is_empty(), "expected empty, got {:?}", got);
    }

    #[test]
    fn unknown_provider_has_no_category_tabs() {
        let got = available_categories("brand-new-protocol", &[]);
        assert!(got.is_empty(), "expected empty, got {:?}", got);
    }
}
