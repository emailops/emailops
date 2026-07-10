//! Single-item secrets vault.
//!
//! Consolidates every secret the app keeps in the OS keychain (OAuth tokens,
//! IMAP credentials, remote-AI API keys) into ONE keychain item — service
//! `"emailops"`, account `"vault"` — whose value is a JSON map keyed by the
//! legacy `(service, account)` pair. macOS authorizes keychain access per
//! item, so N accounts stored as N items meant N authorization prompts at
//! startup; one item means exactly one.
//!
//! ## Migration from per-item storage
//!
//! Already-configured installs have one legacy item per account. Migration is
//! lazy and per-key: when a `get` misses the vault, the legacy item is read,
//! copied into the vault, persisted, and only then deleted. A vault-persist
//! failure leaves the legacy item untouched so no secret is ever lost.
//! `set`/`delete` also clear the legacy item so a stale copy can't shadow a
//! newer value if the vault is ever rebuilt.
//!
//! ## Concurrency
//!
//! All operations serialize on one process-wide lock and mutate a clone of
//! the entry map that is committed to the cache only after the keychain write
//! succeeds — the read-modify-write of the shared JSON blob can't lose
//! updates or leave the cache ahead of the persisted state.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use crate::models::error::{AppError, Result};
use crate::services::keychain;

/// Keychain location of the single vault item. Lives under the same service
/// as the legacy OAuth items; account UUIDs can't collide with `"vault"`.
pub const VAULT_SERVICE: &str = "emailops";
pub const VAULT_ACCOUNT: &str = "vault";

const VAULT_VERSION: u32 = 1;

/// service → account → secret. Values are the exact strings callers used to
/// store as standalone keychain items, so migration is a pure move.
type Entries = HashMap<String, HashMap<String, String>>;

#[derive(serde::Deserialize)]
struct VaultData {
    #[allow(dead_code)] // read for forward-compat checks, unused at version 1
    version: u32,
    entries: Entries,
}

#[derive(serde::Serialize)]
struct VaultDataRef<'a> {
    version: u32,
    entries: &'a Entries,
}

/// In-memory copy of the vault. `None` until first use; populated from the
/// keychain item exactly once per process, so after startup no keychain read
/// (and therefore no authorization prompt) happens again.
static VAULT: std::sync::LazyLock<RwLock<Option<Entries>>> = std::sync::LazyLock::new(|| RwLock::new(None));

fn is_vault_location(service: &str, account: &str) -> bool {
    service == VAULT_SERVICE && account == VAULT_ACCOUNT
}

/// Populate the cache from the keychain item if not yet loaded. A missing
/// item is a fresh (or pre-migration) install → empty vault. A present but
/// unparsable item is an error — starting fresh would orphan every secret.
fn ensure_loaded(cache: &mut Option<Entries>) -> Result<()> {
    if cache.is_some() {
        return Ok(());
    }
    let entries = match keychain::current().get_password(VAULT_SERVICE, VAULT_ACCOUNT)? {
        Some(json) => {
            serde_json::from_str::<VaultData>(&json)
                .map_err(|e| AppError::KeyringError(format!("secrets vault is corrupt: {e}")))?
                .entries
        }
        None => Entries::new(),
    };
    *cache = Some(entries);
    Ok(())
}

fn loaded(cache: &mut Option<Entries>) -> Result<&mut Entries> {
    ensure_loaded(cache)?;
    cache
        .as_mut()
        .ok_or_else(|| AppError::KeyringError("secrets vault cache not initialised".to_string()))
}

fn persist(entries: &Entries) -> Result<()> {
    let json = serde_json::to_string(&VaultDataRef {
        version: VAULT_VERSION,
        entries,
    })?;
    keychain::current().set_password(VAULT_SERVICE, VAULT_ACCOUNT, &json)
}

/// Remove a legacy standalone item after its secret is safely in the vault.
/// Best-effort: the secret is already persisted, a leftover copy is only a
/// hygiene issue — log and move on rather than failing the caller.
fn delete_legacy_best_effort(service: &str, account: &str) {
    if let Err(e) = keychain::current().delete_password(service, account) {
        crate::services::logger::log(
            "error",
            "account",
            format!("secrets-vault: failed to remove legacy keychain item {service}/{account}: {e}"),
        );
    }
}

/// Read a secret. Falls back to (and migrates) the legacy per-account
/// keychain item when the vault has no entry, so already-configured clients
/// keep working across the upgrade without re-authenticating.
pub fn get(service: &str, account: &str) -> Result<Option<String>> {
    let mut cache = VAULT.write().unwrap_or_else(PoisonError::into_inner);
    let entries = loaded(&mut cache)?;
    if let Some(v) = entries.get(service).and_then(|m| m.get(account)) {
        return Ok(Some(v.clone()));
    }
    if is_vault_location(service, account) {
        // Never treat the vault item itself as a legacy entry — that would
        // recursively swallow the blob into itself.
        return Ok(None);
    }
    let Some(legacy) = keychain::current().get_password(service, account)? else {
        return Ok(None);
    };
    let mut updated = entries.clone();
    updated
        .entry(service.to_string())
        .or_default()
        .insert(account.to_string(), legacy.clone());
    // Persist before deleting the legacy item: if this write fails the only
    // copy of the secret must survive for the next attempt.
    persist(&updated)?;
    *entries = updated;
    delete_legacy_best_effort(service, account);
    Ok(Some(legacy))
}

/// Store a secret in the vault (single keychain write), clearing any stale
/// legacy standalone item so it can't shadow the vault later.
pub fn set(service: &str, account: &str, value: &str) -> Result<()> {
    let mut cache = VAULT.write().unwrap_or_else(PoisonError::into_inner);
    let entries = loaded(&mut cache)?;
    let mut updated = entries.clone();
    updated
        .entry(service.to_string())
        .or_default()
        .insert(account.to_string(), value.to_string());
    persist(&updated)?;
    *entries = updated;
    if !is_vault_location(service, account) {
        delete_legacy_best_effort(service, account);
    }
    Ok(())
}

/// Delete a secret from the vault and any legacy standalone item. Idempotent,
/// mirroring OS keychain delete semantics.
pub fn delete(service: &str, account: &str) -> Result<()> {
    let mut cache = VAULT.write().unwrap_or_else(PoisonError::into_inner);
    let entries = loaded(&mut cache)?;
    let mut updated = entries.clone();
    let removed = updated
        .get_mut(service)
        .map(|m| m.remove(account).is_some())
        .unwrap_or(false);
    if updated.get(service).is_some_and(HashMap::is_empty) {
        updated.remove(service);
    }
    if removed {
        persist(&updated)?;
        *entries = updated;
    }
    if !is_vault_location(service, account) {
        keychain::current().delete_password(service, account)?;
    }
    Ok(())
}

/// Drop the in-memory cache so the next operation re-reads the keychain item.
/// Simulates a process restart in tests.
#[cfg(test)]
pub fn reset_for_testing() {
    let mut cache = VAULT.write().unwrap_or_else(PoisonError::into_inner);
    *cache = None;
}

// ── Tests (written first — TDD) ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::keychain::{self, InMemoryKeychain, Keychain};
    use std::sync::{Arc, Mutex, MutexGuard};

    /// The vault cache and the keychain backend are process globals — tests
    /// must not interleave. Each test takes this lock, resets the cache, and
    /// installs a fresh in-memory keychain.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn setup() -> (MutexGuard<'static, ()>, Arc<InMemoryKeychain>) {
        let guard = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        reset_for_testing();
        let kc = keychain::install_for_testing();
        (guard, kc)
    }

    #[test]
    fn get_returns_none_when_vault_and_legacy_are_both_empty() {
        let (_guard, _kc) = setup();
        assert_eq!(get("emailops", "acct-1").unwrap(), None);
    }

    #[test]
    fn set_then_get_round_trips_through_a_single_vault_item() {
        let (_guard, kc) = setup();
        set("emailops", "acct-1", r#"{"access_token":"tok"}"#).unwrap();

        assert_eq!(
            get("emailops", "acct-1").unwrap(),
            Some(r#"{"access_token":"tok"}"#.to_string())
        );
        // The secret lands inside the vault item, never as its own entry.
        assert!(kc.get_password("emailops", "vault").unwrap().is_some());
        assert_eq!(kc.get_password("emailops", "acct-1").unwrap(), None);
    }

    #[test]
    fn entries_are_isolated_by_service() {
        let (_guard, _kc) = setup();
        set("emailops", "acct-1", "oauth-json").unwrap();
        set("emailops-imap", "acct-1", "imap-json").unwrap();

        assert_eq!(get("emailops", "acct-1").unwrap(), Some("oauth-json".into()));
        assert_eq!(get("emailops-imap", "acct-1").unwrap(), Some("imap-json".into()));
    }

    #[test]
    fn get_migrates_a_legacy_per_account_item_into_the_vault() {
        let (_guard, kc) = setup();
        // Simulate an already-configured client: per-account items, no vault.
        kc.set_password("emailops", "acct-1", "legacy-oauth").unwrap();
        kc.set_password("emailops-imap", "acct-2", "legacy-imap").unwrap();

        assert_eq!(get("emailops", "acct-1").unwrap(), Some("legacy-oauth".into()));
        assert_eq!(get("emailops-imap", "acct-2").unwrap(), Some("legacy-imap".into()));

        // Legacy items are gone; the vault item holds both secrets now.
        assert_eq!(kc.get_password("emailops", "acct-1").unwrap(), None);
        assert_eq!(kc.get_password("emailops-imap", "acct-2").unwrap(), None);
        assert!(kc.get_password("emailops", "vault").unwrap().is_some());

        // A fresh process (cache cleared) reads them back from the vault item.
        reset_for_testing();
        assert_eq!(get("emailops", "acct-1").unwrap(), Some("legacy-oauth".into()));
        assert_eq!(get("emailops-imap", "acct-2").unwrap(), Some("legacy-imap".into()));
    }

    #[test]
    fn migration_keeps_the_legacy_item_when_the_vault_write_fails() {
        // A keychain that accepts everything except writes to the vault item —
        // simulates a locked/failing keychain mid-migration.
        struct VaultWriteFails(InMemoryKeychain);
        impl Keychain for VaultWriteFails {
            fn get_password(&self, service: &str, account: &str) -> Result<Option<String>> {
                self.0.get_password(service, account)
            }
            fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()> {
                if service == VAULT_SERVICE && account == VAULT_ACCOUNT {
                    return Err(crate::models::error::AppError::KeyringError(
                        "vault write rejected".into(),
                    ));
                }
                self.0.set_password(service, account, password)
            }
            fn delete_password(&self, service: &str, account: &str) -> Result<()> {
                self.0.delete_password(service, account)
            }
        }

        let guard = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        reset_for_testing();
        let kc = Arc::new(VaultWriteFails(InMemoryKeychain::new()));
        keychain::install(kc.clone());
        kc.0.set_password("emailops", "acct-1", "legacy-oauth").unwrap();

        assert!(get("emailops", "acct-1").is_err(), "persist failure must surface");
        // The one copy of the secret must survive for the next attempt.
        assert_eq!(
            kc.0.get_password("emailops", "acct-1").unwrap(),
            Some("legacy-oauth".into())
        );

        // Once the keychain recovers, the same get succeeds and migrates.
        keychain::install(Arc::new({
            let healthy = InMemoryKeychain::new();
            healthy.set_password("emailops", "acct-1", "legacy-oauth").unwrap();
            healthy
        }));
        assert_eq!(get("emailops", "acct-1").unwrap(), Some("legacy-oauth".into()));
        drop(guard);
    }

    #[test]
    fn set_clears_a_stale_legacy_item() {
        let (_guard, kc) = setup();
        kc.set_password("emailops-imap", "acct-1", "old-imap").unwrap();

        set("emailops-imap", "acct-1", "new-imap").unwrap();

        assert_eq!(get("emailops-imap", "acct-1").unwrap(), Some("new-imap".into()));
        // The stale per-account copy can't shadow the vault later.
        assert_eq!(kc.get_password("emailops-imap", "acct-1").unwrap(), None);
    }

    #[test]
    fn delete_removes_vault_entry_and_legacy_item_idempotently() {
        let (_guard, kc) = setup();
        kc.set_password("emailops", "acct-1", "legacy").unwrap();
        set("emailops", "acct-2", "vaulted").unwrap();

        delete("emailops", "acct-1").unwrap();
        delete("emailops", "acct-2").unwrap();

        assert_eq!(get("emailops", "acct-1").unwrap(), None);
        assert_eq!(get("emailops", "acct-2").unwrap(), None);
        assert_eq!(kc.get_password("emailops", "acct-1").unwrap(), None);

        // Deleting again is a no-op, mirroring OS keychain semantics.
        delete("emailops", "acct-1").unwrap();
        delete("emailops", "acct-2").unwrap();
    }

    #[test]
    fn vault_contents_survive_a_simulated_restart() {
        let (_guard, _kc) = setup();
        set("emailops", "acct-1", "tok").unwrap();

        reset_for_testing(); // drop the in-memory cache, keep the keychain

        assert_eq!(get("emailops", "acct-1").unwrap(), Some("tok".into()));
    }

    #[test]
    fn corrupted_vault_json_surfaces_an_error_instead_of_wiping_secrets() {
        let (_guard, kc) = setup();
        kc.set_password("emailops", "vault", "this is not json").unwrap();
        kc.set_password("emailops", "acct-1", "legacy").unwrap();

        assert!(get("emailops", "acct-1").is_err());
        // Nothing was destroyed: both items are still in the keychain.
        assert_eq!(
            kc.get_password("emailops", "vault").unwrap(),
            Some("this is not json".into())
        );
        assert_eq!(kc.get_password("emailops", "acct-1").unwrap(), Some("legacy".into()));
    }

    #[test]
    fn the_vaults_own_location_is_not_treated_as_a_legacy_entry() {
        let (_guard, kc) = setup();
        set("emailops", "acct-1", "tok").unwrap();

        // Asking for the vault's own (service, account) must not recurse into
        // migration and swallow the blob into itself.
        assert_eq!(get(VAULT_SERVICE, VAULT_ACCOUNT).unwrap(), None);
        assert!(kc.get_password("emailops", "vault").unwrap().is_some());
        assert_eq!(get("emailops", "acct-1").unwrap(), Some("tok".into()));
    }
}
