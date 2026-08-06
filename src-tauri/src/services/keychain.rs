//! Keychain trait seam.
//!
//! Wraps the OS keychain (via the `keyring` crate) behind a trait so tests can
//! swap in an in-memory backend instead of hitting the real macOS keychain
//! (which would prompt the user and pollute their actual credential store).
//!
//! The trait is exposed through a swappable global (`current()` / `install()`)
//! rather than threaded as `&dyn Keychain` through every call site, because
//! credential access happens deep in 13+ caller chains (sync/, services/emails/,
//! services/attachments/, commands/) that have no other reason to know about
//! the keychain. The global mirrors the existing `TOKEN_DB`/`TOKEN_CACHE`
//! `LazyLock` singletons in `services::accounts`.
//!
//! ## Production
//!
//! `OsKeychain` is installed by default the first time `current()` is called.
//! Wraps `keyring_core::Entry::new(...).{get_password, set_password,
//! delete_credential}`. The native OS credential store is selected at startup
//! by `init_native_store()` (called from `pub fn run()`).
//!
//! ## Tests
//!
//! Tests that exercise keychain code paths must call
//! [`install_for_testing()`] before invoking any code that reads/writes
//! credentials. This swaps in an `InMemoryKeychain`. Note: tests sharing the
//! global must not run in parallel against different keychain states — use a
//! test-level mutex or run them serially if you need isolation.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use crate::models::error::{AppError, Result};

/// Read/write/delete passwords from a credential store. All operations are
/// synchronous because the underlying `keyring` crate is sync.
///
/// `get_password` returns `Ok(None)` when there is no entry — distinct from
/// `Err(...)` which means the backend itself failed. Callers that need to
/// distinguish "user must re-authenticate" from "keychain unavailable" should
/// match on this Option vs Result split.
pub trait Keychain: Send + Sync {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>>;
    fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()>;
    fn delete_password(&self, service: &str, account: &str) -> Result<()>;
}

/// Production keychain backend — talks to the OS keychain through the
/// `keyring` crate.
pub struct OsKeychain;

impl Keychain for OsKeychain {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>> {
        let entry = entry_for(service, account)?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            // Missing entry is a normal state (account not yet signed in, or
            // credentials cleared) — surface it as None instead of an error.
            Err(keyring_core::error::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::KeyringError(e.to_string())),
        }
    }

    fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()> {
        let entry = entry_for(service, account)?;
        entry
            .set_password(password)
            .map_err(|e| AppError::KeyringError(e.to_string()))
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<()> {
        let entry = entry_for(service, account)?;
        // Note: keyring 4 renamed `delete_password` -> `delete_credential`.
        match entry.delete_credential() {
            // Deleting a missing entry is idempotent.
            Ok(()) | Err(keyring_core::error::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::KeyringError(e.to_string())),
        }
    }
}

/// Which credential store [`init_native_store`] must install for a platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreChoice {
    /// Apple's protected (data-protection) keychain, selected explicitly.
    AppleProtected,
    /// Whatever `keyring::use_native_store` picks for this platform.
    Native,
}

/// Pure platform → store decision, so the one platform that must NOT take the
/// default path is pinned by a test rather than by a `cfg` nobody re-reads.
///
/// **iOS must be explicit.** `keyring::use_native_store` matches on android,
/// macos, windows, linux, freebsd and openbsd, and falls through to the
/// `sample` store for everything else — including iOS. keyring-core documents
/// that store as "explicitly *not* for use in production apps", in memory, and
/// not persisted between runs by default. The symptom is precise and was
/// reported from a device: every OAuth token is written, read back fine for the
/// life of the process, and gone at next launch — so the account asks for
/// authentication *every time*. It only bites release builds, because
/// `services::accounts::use_dev_tokens` routes debug builds to the `dev_tokens`
/// table instead of the keychain.
pub fn store_choice(target_os: &str) -> StoreChoice {
    match target_os {
        "ios" => StoreChoice::AppleProtected,
        _ => StoreChoice::Native,
    }
}

/// Entry modifiers this platform needs, as `keyring_core` modifier pairs.
///
/// On iOS the protected store defaults to `AccessibleWhenUnlocked`, which is
/// wrong for this app: a background refresh (`services::background_refresh`)
/// runs while the phone is in a pocket, and would find the token unreadable and
/// report a sync failure. `AfterFirstUnlockThisDeviceOnly` is readable from the
/// moment the user unlocks once after boot, and — unlike plain
/// `AfterFirstUnlock` — never travels to another device in a backup, which is
/// the right default for a mail credential.
///
/// Empty everywhere else: the macOS/Windows/Linux stores reject unknown
/// modifiers, so passing iOS's would break the platforms that work today.
pub fn entry_modifiers(target_os: &str) -> Vec<(&'static str, &'static str)> {
    match target_os {
        "ios" => vec![("access-policy", "AfterFirstUnlockThisDeviceOnly")],
        _ => Vec::new(),
    }
}

/// Build an entry for the current platform, applying [`entry_modifiers`].
fn entry_for(service: &str, account: &str) -> Result<keyring_core::Entry> {
    let modifiers = entry_modifiers(std::env::consts::OS);
    if modifiers.is_empty() {
        keyring_core::Entry::new(service, account).map_err(|e| AppError::KeyringError(e.to_string()))
    } else {
        let map: HashMap<&str, &str> = modifiers.into_iter().collect();
        keyring_core::Entry::new_with_modifiers(service, account, &map)
            .map_err(|e| AppError::KeyringError(e.to_string()))
    }
}

/// Initialise the OS-native credential store. Must be called once at process
/// start before any `keyring_core::Entry` operation. keyring 4 requires
/// explicit backend selection; previously this was implicit.
pub fn init_native_store() -> Result<()> {
    match store_choice(std::env::consts::OS) {
        StoreChoice::AppleProtected => {
            // `use_apple_protected_store`, NOT `use_apple_keychain_store`. The
            // latter is the legacy macOS Keychain Services store and returns
            // `NotSupportedByStore` on iOS — which `run()` turns into a fatal
            // startup error, so the app launched, printed nothing, and exited
            // with status 1. The protected (data-protection) store is the only
            // keychain iOS has.
            //
            // Empty config: no `access-group` means the app's own default group,
            // and `cloud-sync` stays off so tokens never leave the device.
            let config: HashMap<&str, &str> = HashMap::new();
            keyring::use_apple_protected_store(&config).map_err(|e| AppError::KeyringError(e.to_string()))?;
        }
        StoreChoice::Native => {
            // `prefer_secret_service = true` on Linux selects the Secret Service
            // backend (which persists across reboots in a desktop session)
            // instead of the kernel keyutils store. Ignored on macOS/Windows.
            keyring::use_native_store(true).map_err(|e| AppError::KeyringError(e.to_string()))?;
        }
    }
    Ok(())
}

/// In-memory keychain for tests. Mimics OS keychain semantics (idempotent
/// delete, missing entries return `Ok(None)`).
#[derive(Default)]
pub struct InMemoryKeychain {
    entries: RwLock<HashMap<(String, String), String>>,
}

impl InMemoryKeychain {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Keychain for InMemoryKeychain {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>> {
        let map = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        Ok(map.get(&(service.to_string(), account.to_string())).cloned())
    }

    fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()> {
        let mut map = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        map.insert((service.to_string(), account.to_string()), password.to_string());
        Ok(())
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<()> {
        let mut map = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        map.remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

// ── Global registry ──────────────────────────────────────────────────────────

static KEYCHAIN: std::sync::LazyLock<RwLock<Arc<dyn Keychain>>> =
    std::sync::LazyLock::new(|| RwLock::new(Arc::new(OsKeychain)));

/// Get a handle to the active keychain backend. Production code reads through
/// this to access OS credentials; tests install an in-memory backend via
/// [`install_for_testing`] before calling.
pub fn current() -> Arc<dyn Keychain> {
    // A per-user context, when one is installed, always wins: it is how a server
    // keeps one user's keychain from resolving to another user's. Desktop, CLI and
    // tests install no context and fall through to the process-global backend.
    if let Some(cx) = crate::runtime::ctx::try_current() {
        return cx.keychain.clone();
    }
    KEYCHAIN.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Swap the active backend. Used at process start to install a non-default
/// implementation (e.g. a remote/secrets-manager backend) and by tests.
pub fn install(backend: Arc<dyn Keychain>) {
    let mut k = KEYCHAIN.write().unwrap_or_else(PoisonError::into_inner);
    *k = backend;
}

/// Install an `InMemoryKeychain` and return a handle to it so the test can
/// inspect state directly if it needs to.
///
/// Returns the same Arc that is now stored in the global — modifications via
/// either reference are visible to both. Callers must already hold
/// `events::seam_test_lock()`, which serializes every process-global seam.
#[cfg(test)]
pub fn install_for_testing() -> Arc<InMemoryKeychain> {
    let kc = Arc::new(InMemoryKeychain::new());
    install(kc.clone() as Arc<dyn Keychain>);
    kc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_never_takes_the_default_store_path() {
        // Regression: `keyring::use_native_store` has no iOS arm and falls
        // through to the in-memory `sample` store, so tokens vanished when the
        // process exited and the account asked for authentication on every
        // launch. Reported from a device, invisible in debug builds (which use
        // `dev_tokens` instead of the keychain).
        assert_eq!(store_choice("ios"), StoreChoice::AppleProtected);
    }

    #[test]
    fn every_other_platform_keeps_the_native_store() {
        for os in ["macos", "windows", "linux", "freebsd", "openbsd", "android"] {
            assert_eq!(store_choice(os), StoreChoice::Native, "{os}");
        }
    }

    #[test]
    fn an_unknown_platform_is_left_to_keyring() {
        // Deliberate: a new target should fail loudly inside keyring rather
        // than be silently handed Apple's store.
        assert_eq!(store_choice("plan9"), StoreChoice::Native);
    }

    #[test]
    fn ios_credentials_survive_a_locked_screen() {
        // The default protected-store policy is "accessible when unlocked",
        // which a background refresh — the whole point of which is running
        // while the phone is in a pocket — could never read.
        assert_eq!(
            entry_modifiers("ios"),
            vec![("access-policy", "AfterFirstUnlockThisDeviceOnly")]
        );
    }

    #[test]
    fn no_other_platform_is_handed_apple_modifiers() {
        // The macOS/Windows/Linux stores reject modifiers they don't know, so
        // leaking iOS's would break every platform that works today.
        for os in ["macos", "windows", "linux"] {
            assert!(entry_modifiers(os).is_empty(), "{os}");
        }
    }

    #[test]
    fn in_memory_round_trip() {
        let kc = InMemoryKeychain::new();
        assert_eq!(kc.get_password("svc", "acct").unwrap(), None);
        kc.set_password("svc", "acct", "hunter2").unwrap();
        assert_eq!(kc.get_password("svc", "acct").unwrap(), Some("hunter2".into()));
        kc.delete_password("svc", "acct").unwrap();
        assert_eq!(kc.get_password("svc", "acct").unwrap(), None);
        // Idempotent delete
        kc.delete_password("svc", "acct").unwrap();
    }

    #[test]
    fn in_memory_isolates_by_service_and_account() {
        let kc = InMemoryKeychain::new();
        kc.set_password("svc-a", "acct1", "x").unwrap();
        kc.set_password("svc-b", "acct1", "y").unwrap();
        kc.set_password("svc-a", "acct2", "z").unwrap();
        assert_eq!(kc.get_password("svc-a", "acct1").unwrap(), Some("x".into()));
        assert_eq!(kc.get_password("svc-b", "acct1").unwrap(), Some("y".into()));
        assert_eq!(kc.get_password("svc-a", "acct2").unwrap(), Some("z".into()));
        assert_eq!(kc.get_password("svc-c", "acct1").unwrap(), None);
    }
}
