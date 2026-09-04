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
        let entry = keyring_core::Entry::new(service, account).map_err(|e| AppError::KeyringError(e.to_string()))?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            // Missing entry is a normal state (account not yet signed in, or
            // credentials cleared) — surface it as None instead of an error.
            Err(keyring_core::error::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::KeyringError(e.to_string())),
        }
    }

    fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()> {
        let entry = keyring_core::Entry::new(service, account).map_err(|e| AppError::KeyringError(e.to_string()))?;
        entry
            .set_password(password)
            .map_err(|e| AppError::KeyringError(e.to_string()))
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<()> {
        let entry = keyring_core::Entry::new(service, account).map_err(|e| AppError::KeyringError(e.to_string()))?;
        // Note: keyring 4 renamed `delete_password` -> `delete_credential`.
        match entry.delete_credential() {
            // Deleting a missing entry is idempotent.
            Ok(()) | Err(keyring_core::error::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::KeyringError(e.to_string())),
        }
    }
}

/// Initialise the OS-native credential store. Must be called once at process
/// start before any `keyring_core::Entry::new(...)` operation. keyring 4
/// requires explicit backend selection; previously this was implicit.
pub fn init_native_store() -> Result<()> {
    // `not_keyutils = true` on Linux selects the Secret Service backend (which
    // persists across reboots in a desktop session) instead of the kernel
    // keyutils store. Ignored on macOS/Windows.
    keyring::use_native_store(true).map_err(|e| AppError::KeyringError(e.to_string()))?;
    Ok(())
}

// ── Oversized-value chunking ─────────────────────────────────────────────────

/// Windows caps a credential blob at `CRED_MAX_CREDENTIAL_BLOB_SIZE`
/// (5 × 512 = 2560 bytes) and `keyring` measures the value *after* encoding it
/// as UTF-16 — so the real ceiling is 1280 ASCII characters, not 2560.
pub const WINDOWS_CREDENTIAL_BLOB_LIMIT: usize = 5 * 512;

/// Per-chunk budget used in production, in UTF-16 bytes. Comfortably under
/// [`WINDOWS_CREDENTIAL_BLOB_LIMIT`] so the manifest and any per-backend
/// framing have room to spare.
pub const DEFAULT_CHUNK_BUDGET: usize = 2048;

/// Smallest budget a [`ChunkedKeychain`] will honour. The manifest is ~45 ASCII
/// characters, so a budget below this could not store one.
pub const MIN_CHUNK_BUDGET: usize = 256;

/// Ceiling on how many pieces one value may be split into — 512 × 2048 bytes is
/// ~1 MB of secret, far past anything a credential store should be asked to
/// hold, so exceeding it means a bug rather than a big mailbox.
const MAX_CHUNKS: usize = 512;

/// Size of `value` in the units Windows actually measures.
fn utf16_bytes(value: &str) -> usize {
    value.chars().map(|c| c.len_utf16() * 2).sum()
}

/// Keychain account name of one chunk. `#` cannot appear in the account names
/// we generate (UUIDs, `"vault"`, fixed key ids), so a chunk can never collide
/// with a real entry.
fn chunk_account(account: &str, index: usize) -> String {
    format!("{account}#c{index}")
}

/// Placeholder written at the value's own key once it has been split. Its
/// single dunder field is what tells a read that the entry is a manifest and
/// not the secret itself; `deny_unknown_fields` keeps any other JSON secret
/// (the vault blob's `{"version":…,"entries":…}`, for one) from matching.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkManifest {
    #[serde(rename = "__emailops_chunked_v1__")]
    chunks: usize,
    /// Byte length of the reassembled value. Read back as a checksum: a write
    /// interrupted partway through leaves chunks that do not add up, and that
    /// must surface as an error rather than as a silently truncated secret.
    len: usize,
}

fn parse_chunk_manifest(raw: &str) -> Option<ChunkManifest> {
    serde_json::from_str::<ChunkManifest>(raw).ok()
}

/// Split `value` into pieces of at most `budget` UTF-16 bytes each, never
/// cutting a character (and therefore never a surrogate pair) in half.
///
/// Always returns at least one piece, so `pieces.concat() == value` holds for
/// every input including the empty string.
fn split_by_utf16_budget(value: &str, budget: usize) -> Vec<&str> {
    // The overwhelming majority of secrets fit, and this is on the read/write
    // path of every credential — measure once before walking characters.
    if utf16_bytes(value) <= budget {
        return vec![value];
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (index, ch) in value.char_indices() {
        let cost = ch.len_utf16() * 2;
        if used + cost > budget && index > start {
            pieces.push(&value[start..index]);
            start = index;
            used = 0;
        }
        used += cost;
    }
    if pieces.is_empty() || start < value.len() {
        pieces.push(&value[start..]);
    }
    pieces
}

/// Wraps a keychain whose backend caps how large one value may be, splitting
/// anything over the cap across extra entries and reassembling it on read.
///
/// Only Windows needs this. macOS authorizes keychain access **per item**, so
/// the secrets vault deliberately keeps every secret in one item to cost the
/// user exactly one prompt at startup (see `services::secrets_vault`); splitting
/// it there would reintroduce the prompt storm the vault exists to remove.
/// Windows' Credential Manager has no such prompt but does have a hard 2560-byte
/// blob limit, which one Microsoft OAuth token can exceed on its own — so on
/// Windows, and only there, the vault item is stored in pieces.
pub struct ChunkedKeychain {
    inner: Arc<dyn Keychain>,
    budget: usize,
}

impl ChunkedKeychain {
    /// `budget` is the per-entry ceiling in UTF-16 bytes, clamped up to
    /// [`MIN_CHUNK_BUDGET`] so the manifest always fits.
    pub fn new(inner: Arc<dyn Keychain>, budget: usize) -> Self {
        Self {
            inner,
            budget: budget.max(MIN_CHUNK_BUDGET),
        }
    }

    /// Delete chunk entries from `start` upward until one is missing.
    ///
    /// Best-effort by design: the value itself is already written, so a failure
    /// here leaves litter rather than losing a secret. It is still logged —
    /// a leftover chunk is how a later, longer write could read back corrupt.
    fn clear_chunks_from(&self, service: &str, account: &str, start: usize) {
        for index in start..MAX_CHUNKS {
            let key = chunk_account(account, index);
            match self.inner.get_password(service, &key) {
                Ok(Some(_)) => {
                    if let Err(e) = self.inner.delete_password(service, &key) {
                        crate::services::logger::log(
                            "error",
                            "account",
                            format!("keychain: failed to remove stale chunk {service}/{key}: {e}"),
                        );
                        return;
                    }
                }
                Ok(None) => return,
                Err(e) => {
                    crate::services::logger::log(
                        "error",
                        "account",
                        format!("keychain: failed to scan for stale chunks of {service}/{account}: {e}"),
                    );
                    return;
                }
            }
        }
    }
}

impl Keychain for ChunkedKeychain {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<String>> {
        let Some(raw) = self.inner.get_password(service, account)? else {
            return Ok(None);
        };
        let Some(manifest) = parse_chunk_manifest(&raw) else {
            return Ok(Some(raw));
        };
        let mut joined = String::with_capacity(manifest.len);
        for index in 0..manifest.chunks {
            let key = chunk_account(account, index);
            let piece = self.inner.get_password(service, &key)?.ok_or_else(|| {
                AppError::KeyringError(format!(
                    "stored credential {service}/{account} is missing part {} of {} — sign in again to store it afresh",
                    index + 1,
                    manifest.chunks
                ))
            })?;
            joined.push_str(&piece);
        }
        if joined.len() != manifest.len {
            return Err(AppError::KeyringError(format!(
                "stored credential {service}/{account} is incomplete ({} of {} bytes) — sign in again to store it afresh",
                joined.len(),
                manifest.len
            )));
        }
        Ok(Some(joined))
    }

    fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()> {
        let pieces = split_by_utf16_budget(password, self.budget);
        if pieces.len() == 1 {
            self.inner.set_password(service, account, password)?;
            // A value that used to be chunked and now fits must not leave its
            // old pieces behind for a later write to pick up.
            self.clear_chunks_from(service, account, 0);
            return Ok(());
        }
        if pieces.len() > MAX_CHUNKS {
            return Err(AppError::KeyringError(format!(
                "credential {service}/{account} is too large to store ({} bytes)",
                password.len()
            )));
        }
        for (index, piece) in pieces.iter().enumerate() {
            self.inner
                .set_password(service, &chunk_account(account, index), piece)?;
        }
        // Manifest last: it is what makes the pieces readable, so a write that
        // dies before this point leaves the entry reading as whatever it held
        // before. If that older value was itself chunked its pieces have now
        // been overwritten, which the manifest's length check turns into a
        // clear "sign in again" on the next read rather than a corrupt secret.
        let manifest = serde_json::to_string(&ChunkManifest {
            chunks: pieces.len(),
            len: password.len(),
        })?;
        self.inner.set_password(service, account, &manifest)?;
        self.clear_chunks_from(service, account, pieces.len());
        Ok(())
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<()> {
        self.inner.delete_password(service, account)?;
        self.clear_chunks_from(service, account, 0);
        Ok(())
    }
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

/// The backend production runs on, before any [`install`] override.
///
/// Windows gets the chunking wrapper because its credential blobs are capped at
/// 2560 UTF-16 bytes; macOS and Linux store the value as one item, which is what
/// keeps the secrets vault to a single authorization prompt.
fn default_backend() -> Arc<dyn Keychain> {
    let os: Arc<dyn Keychain> = Arc::new(OsKeychain);
    // `cfg!` rather than `#[cfg]`: both arms are type-checked in every build, so a
    // change that compiles here cannot still break the Windows job later in CI.
    // The dead arm is folded away at compile time.
    if cfg!(target_os = "windows") {
        Arc::new(ChunkedKeychain::new(os, DEFAULT_CHUNK_BUDGET))
    } else {
        os
    }
}

static KEYCHAIN: std::sync::LazyLock<RwLock<Arc<dyn Keychain>>> =
    std::sync::LazyLock::new(|| RwLock::new(default_backend()));

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
mod chunking_tests {
    use super::*;

    /// Stands in for the Windows credential store: `CredWrite` refuses a blob
    /// larger than `CRED_MAX_CREDENTIAL_BLOB_SIZE`, and the `keyring` crate
    /// surfaces that as the error issue #54 reported.
    struct WindowsLikeKeychain {
        inner: InMemoryKeychain,
    }

    impl Keychain for WindowsLikeKeychain {
        fn get_password(&self, service: &str, account: &str) -> Result<Option<String>> {
            self.inner.get_password(service, account)
        }
        fn set_password(&self, service: &str, account: &str, password: &str) -> Result<()> {
            if utf16_bytes(password) > WINDOWS_CREDENTIAL_BLOB_LIMIT {
                return Err(AppError::KeyringError(format!(
                    "Value of 'password encoded as UTF-16' is longer than the platform limit of {WINDOWS_CREDENTIAL_BLOB_LIMIT} chars"
                )));
            }
            self.inner.set_password(service, account, password)
        }
        fn delete_password(&self, service: &str, account: &str) -> Result<()> {
            self.inner.delete_password(service, account)
        }
    }

    fn chunked(budget: usize) -> (ChunkedKeychain, Arc<InMemoryKeychain>) {
        let inner = Arc::new(InMemoryKeychain::new());
        (ChunkedKeychain::new(inner.clone(), budget), inner)
    }

    #[test]
    fn an_outlook_sized_token_blob_survives_the_windows_blob_limit() {
        // Regression for #54. A Microsoft refresh token alone runs to a few KB,
        // and the secrets vault holds every account's secrets in ONE item, so
        // the write blew past Windows' 2560-byte cap and adding the account
        // failed outright.
        let store = ChunkedKeychain::new(
            Arc::new(WindowsLikeKeychain {
                inner: InMemoryKeychain::new(),
            }),
            DEFAULT_CHUNK_BUDGET,
        );
        let vault_blob = format!(
            r#"{{"version":1,"entries":{{"emailops":{{"acc":"{}"}}}}}}"#,
            "T".repeat(9_000)
        );

        // The failure the reporter hit, reproduced against the raw backend.
        let raw = WindowsLikeKeychain {
            inner: InMemoryKeychain::new(),
        };
        let err = raw.set_password("emailops", "vault", &vault_blob).unwrap_err();
        assert!(
            err.to_string().contains("longer than the platform limit"),
            "expected the platform-limit rejection, got {err}"
        );

        store
            .set_password("emailops", "vault", &vault_blob)
            .expect("a blob past the platform limit must still be storable");

        assert_eq!(store.get_password("emailops", "vault").unwrap(), Some(vault_blob));
    }

    #[test]
    fn a_value_within_budget_is_stored_verbatim_under_its_own_key() {
        // Chunking must not change how ordinary values are stored — an existing
        // install has to keep reading them, and macOS keeps its one-item vault.
        let (store, inner) = chunked(DEFAULT_CHUNK_BUDGET);
        store.set_password("svc", "acct", "hunter2").unwrap();

        assert_eq!(inner.get_password("svc", "acct").unwrap(), Some("hunter2".into()));
        assert_eq!(store.get_password("svc", "acct").unwrap(), Some("hunter2".into()));
    }

    #[test]
    fn every_chunk_of_a_split_value_fits_the_budget() {
        let budget = 256;
        let (store, inner) = chunked(budget);
        let value = "x".repeat(4_000);
        store.set_password("svc", "acct", &value).unwrap();

        let manifest = inner.get_password("svc", "acct").unwrap().expect("manifest");
        let chunks = parse_chunk_manifest(&manifest).expect("value must be chunked").chunks;
        assert!(chunks > 1, "a 4000-char value must not fit one 256-byte chunk");
        for i in 0..chunks {
            let piece = inner
                .get_password("svc", &chunk_account("acct", i))
                .unwrap()
                .expect("chunk");
            assert!(
                utf16_bytes(&piece) <= budget,
                "chunk {i} is {} UTF-16 bytes, over the {budget} budget",
                utf16_bytes(&piece)
            );
        }
    }

    #[test]
    fn multibyte_characters_are_never_split_across_chunks() {
        // A surrogate pair costs 4 UTF-16 bytes. Cutting one in half would
        // corrupt the secret on reassembly.
        let (store, _) = chunked(MIN_CHUNK_BUDGET);
        let value = "🔐é".repeat(400);
        store.set_password("svc", "acct", &value).unwrap();
        assert_eq!(store.get_password("svc", "acct").unwrap(), Some(value));
    }

    #[test]
    fn shrinking_a_chunked_value_clears_the_chunks_it_no_longer_uses() {
        // A stale chunk left behind would be picked up by a later, longer write
        // and silently corrupt the secret.
        let (store, inner) = chunked(MIN_CHUNK_BUDGET);
        store.set_password("svc", "acct", &"x".repeat(4_000)).unwrap();
        store.set_password("svc", "acct", "small").unwrap();

        assert_eq!(store.get_password("svc", "acct").unwrap(), Some("small".into()));
        assert_eq!(
            inner.get_password("svc", &chunk_account("acct", 0)).unwrap(),
            None,
            "no chunk may outlive the value it belonged to"
        );
    }

    #[test]
    fn deleting_a_chunked_value_removes_every_chunk() {
        let (store, inner) = chunked(MIN_CHUNK_BUDGET);
        store.set_password("svc", "acct", &"x".repeat(4_000)).unwrap();
        store.delete_password("svc", "acct").unwrap();

        assert_eq!(store.get_password("svc", "acct").unwrap(), None);
        for i in 0..8 {
            assert_eq!(inner.get_password("svc", &chunk_account("acct", i)).unwrap(), None);
        }
    }

    #[test]
    fn a_torn_chunk_set_is_reported_instead_of_returned_as_a_truncated_secret() {
        // Losing a chunk (an interrupted write, a user pruning Credential
        // Manager) must read as "sign in again", never as a shorter token that
        // fails somewhere far away.
        let (store, inner) = chunked(MIN_CHUNK_BUDGET);
        store.set_password("svc", "acct", &"x".repeat(4_000)).unwrap();
        inner.delete_password("svc", &chunk_account("acct", 1)).unwrap();

        let err = store.get_password("svc", "acct").unwrap_err();
        assert!(matches!(err, AppError::KeyringError(_)), "got {err:?}");
    }

    #[test]
    fn splitting_is_a_no_op_for_values_within_budget() {
        assert_eq!(split_by_utf16_budget("", 256), vec![""]);
        assert_eq!(split_by_utf16_budget("abc", 256), vec!["abc"]);
    }

    #[test]
    fn split_pieces_rejoin_to_the_original() {
        let value = "ünïcødé-🔐-payload".repeat(120);
        let pieces = split_by_utf16_budget(&value, MIN_CHUNK_BUDGET);
        assert!(pieces.len() > 1);
        assert_eq!(pieces.concat(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
