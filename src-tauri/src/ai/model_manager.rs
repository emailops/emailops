// Model manager — download, verify, and inventory local GGUF files.
//
// Models are stored in:
//   <app_data_dir>/models/chat/<id>.gguf
//   <app_data_dir>/models/embed/<id>.gguf
//
// Downloads are resumable (HTTP Range header) and SHA-256 verified.
// In-progress downloads use a `.partial` suffix; the file is renamed
// atomically only after the hash checks out.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::ai::model_catalog::ModelKind;
use crate::models::error::{AppError, Result};

/// Information about a GGUF that has been downloaded locally.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub id: String,
    pub display_name: String,
    pub kind: ModelKind,
    pub path: String,
    pub size_bytes: u64,
    /// True when this model is a symlink to a file elsewhere on disk (via
    /// `link_local_model`) rather than a file downloaded/copied into the
    /// managed models directory.
    pub is_linked: bool,
}

/// Progress event emitted during a download.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// "downloading" | "verifying" | "complete" | "error"
    pub status: String,
    pub error: Option<String>,
}

/// Root directory for all model files inside `app_data_dir`.
pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("models")
}

pub fn chat_models_dir(app_data_dir: &Path) -> PathBuf {
    models_dir(app_data_dir).join("chat")
}

pub fn embed_models_dir(app_data_dir: &Path) -> PathBuf {
    models_dir(app_data_dir).join("embed")
}

pub fn model_path(app_data_dir: &Path, kind: ModelKind, id: &str) -> PathBuf {
    match kind {
        ModelKind::Chat => chat_models_dir(app_data_dir).join(format!("{}.gguf", id)),
        ModelKind::Embedding => embed_models_dir(app_data_dir).join(format!("{}.gguf", id)),
    }
}

/// List all locally available models by scanning `app_data_dir/models/`.
pub fn list_local_models(app_data_dir: &Path) -> Vec<LocalModel> {
    let mut models = Vec::new();
    for (kind, dir) in [
        (ModelKind::Chat, chat_models_dir(app_data_dir)),
        (ModelKind::Embedding, embed_models_dir(app_data_dir)),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("gguf") {
                continue;
            }
            // `std::fs::metadata` follows symlinks, so a linked model reports
            // its real target size. A dangling symlink (target moved/deleted)
            // errors here and is skipped entirely rather than reported with a
            // bogus size of 0.
            let Ok(size_bytes) = std::fs::metadata(&path).map(|m| m.len()) else {
                continue;
            };
            let is_linked = std::fs::symlink_metadata(&path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            // Look up display name from catalog; fall back to the file stem.
            let display_name = crate::ai::model_catalog::find(&id)
                .map(|m| m.display_name.to_string())
                .unwrap_or_else(|| id.clone());
            models.push(LocalModel {
                id,
                display_name,
                kind,
                path: path.to_string_lossy().into_owned(),
                size_bytes,
                is_linked,
            });
        }
    }
    models
}

/// Copy a model file shipped with the app bundle into the user's models
/// directory, if it isn't there yet. Returns `true` if a copy happened,
/// `false` if the destination already existed (and was left untouched —
/// the user may have downloaded their own copy with the same id).
///
/// Used at startup to seed models flagged `bundled: true` in the catalog
/// (currently the Nomic embedding model). The source lives under Tauri's
/// `resource_dir()`; the destination is the same path the download flow
/// writes to, so the rest of the runtime treats it as an ordinary local
/// model.
pub fn seed_bundled_model(src: &Path, app_data_dir: &Path, kind: ModelKind, id: &str) -> Result<bool> {
    if !src.exists() {
        return Err(AppError::NotFound(format!(
            "Bundled model source for '{}' not found at {}",
            id,
            src.display()
        )));
    }
    let dest = model_path(app_data_dir, kind, id);
    if dest.exists() {
        return Ok(false);
    }
    // `dest` is `<app_data_dir>/models/<kind>/<file>` — parent is always present
    // in the type, but the directory may not exist on first launch.
    #[allow(clippy::expect_used)]
    let dir = dest.parent().expect("model path always has a parent");
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::IoError(format!("Cannot create models directory for bundled '{}': {}", id, e)))?;
    std::fs::copy(src, &dest).map_err(|e| {
        AppError::IoError(format!(
            "Failed to seed bundled model '{}' from {} to {}: {}",
            id,
            src.display(),
            dest.display(),
            e
        ))
    })?;
    Ok(true)
}

/// Delete a local model file. Returns an error if the file doesn't exist or
/// cannot be removed.
pub fn delete_local_model(app_data_dir: &Path, kind: ModelKind, id: &str) -> Result<()> {
    let path = model_path(app_data_dir, kind, id);
    if !path.exists() {
        return Err(AppError::NotFound(format!("Model file not found: {}", path.display())));
    }
    std::fs::remove_file(&path).map_err(|e| AppError::IoError(format!("Failed to delete model '{}': {}", id, e)))
}

// ── Download ─────────────────────────────────────────────────────────────────

/// Token to cancel an in-flight download.
///
/// Construct with [`CancelToken::new`] which returns the token (sender side)
/// plus a [`CancelHandle`] receiver that the download loop polls between
/// chunks. Calling `.cancel()` flips the watch value to `true`; the next
/// chunk read in `download_model` observes it via `CancelHandle::is_cancelled`
/// and returns [`AppError::Cancelled`] cleanly. The `.partial` file is
/// preserved on cancel so a future download resumes where the user stopped.
#[derive(Clone)]
pub struct CancelToken(tokio::sync::watch::Sender<bool>);

#[derive(Clone)]
pub struct CancelHandle(tokio::sync::watch::Receiver<bool>);

impl CancelToken {
    /// Create a paired token + handle. The sender lives in the commands
    /// module's `ACTIVE_DOWNLOADS` map; the receiver travels into
    /// `download_model` so the streaming loop can poll it.
    pub fn new() -> (Self, CancelHandle) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        (CancelToken(tx), CancelHandle(rx))
    }

    pub fn cancel(&self) {
        let _ = self.0.send(true);
    }
}

impl CancelHandle {
    /// Non-blocking check used inside the download loop.
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
}

/// Download a catalog model to `<app_data_dir>/models/{chat,embed}/<id>.gguf`.
///
/// - Resumes interrupted downloads via the HTTP `Range` header.
/// - Verifies the SHA-256 hash of the final file (when the catalog entry has one).
/// - Emits `ModelDownloadProgress` events via `on_progress`.
/// - Returns the final file path on success.
/// - Returns `AppError::Cancelled` if `cancel` is provided and the user
///   cancels mid-download. The `.partial` file is preserved so a subsequent
///   call resumes from where it stopped (HTTP Range header).
pub async fn download_model<F>(
    app_data_dir: &Path,
    model_id: &str,
    cancel: Option<CancelHandle>,
    on_progress: F,
) -> Result<PathBuf>
where
    F: Fn(ModelDownloadProgress) + Send + 'static,
{
    use futures::StreamExt;
    use std::io::Write as _;

    let entry = crate::ai::model_catalog::find(model_id)
        .ok_or_else(|| AppError::NotFound(format!("Model '{}' not in catalog", model_id)))?;

    let dest = model_path(app_data_dir, entry.kind, model_id);

    // Already fully downloaded.
    if dest.exists() {
        return Ok(dest);
    }

    // Ensure the target directory exists.
    // `dest` is `<app_data_dir>/models/<kind>/<file>`; the parent is always
    // present (we never call this with a root path).
    #[allow(clippy::expect_used)]
    let dir = dest.parent().expect("model path always has a parent");
    std::fs::create_dir_all(dir).map_err(|e| AppError::IoError(format!("Cannot create model directory: {}", e)))?;

    // `.partial` file for resumable writes.
    let partial = dest.with_extension("gguf.partial");
    let already_downloaded = if partial.exists() {
        partial.metadata().map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    // ── HTTP request with optional Range header ───────────────────────────────
    let client = reqwest::Client::builder()
        // connect_timeout only covers TCP connection establishment.
        // Do NOT set .timeout() here — that caps the entire transfer duration,
        // which kills large GGUF downloads (4-10 GB) after just 30 seconds.
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::AiError(format!("Failed to build HTTP client: {}", e)))?;

    let mut req = client.get(entry.url);
    if already_downloaded > 0 {
        req = req.header("Range", format!("bytes={}-", already_downloaded));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| AppError::AiError(format!("Download request failed: {}", e)))?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(AppError::AiError(format!(
            "Download failed with HTTP {}: {}",
            status.as_u16(),
            entry.display_name
        )));
    }

    // Content-Length gives the size of the *remaining* chunk; add already-downloaded.
    let total_bytes = resp
        .content_length()
        .map(|len| len + already_downloaded)
        .unwrap_or(entry.size_bytes);

    // ── Stream to file ────────────────────────────────────────────────────────
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .map_err(|e| AppError::IoError(format!("Cannot open partial file for writing: {}", e)))?;

    let mut stream = resp.bytes_stream();
    let mut downloaded = already_downloaded;

    while let Some(chunk) = stream.next().await {
        // Check for user cancellation between chunks. Bail out cleanly so
        // the `.partial` file is left intact for resume on next download.
        if let Some(ref h) = cancel {
            if h.is_cancelled() {
                let _ = file.flush();
                return Err(AppError::Cancelled);
            }
        }

        let chunk = chunk.map_err(|e| AppError::AiError(format!("Download stream error: {}", e)))?;

        file.write_all(&chunk)
            .map_err(|e| AppError::IoError(format!("Failed to write chunk: {}", e)))?;

        downloaded += chunk.len() as u64;

        on_progress(ModelDownloadProgress {
            model_id: model_id.to_string(),
            downloaded_bytes: downloaded,
            total_bytes,
            status: "downloading".to_string(),
            error: None,
        });
    }

    // Flush and close before hashing.
    file.flush()
        .map_err(|e| AppError::IoError(format!("Failed to flush model file: {}", e)))?;
    drop(file);

    // ── Completeness check (independent of SHA-256) ───────────────────────────
    // The HTTP stream can end prematurely without surfacing an error chunk —
    // proxies, mid-flight TCP resets, or the server dropping connection mid-LFS-
    // redirect all produce a truncated `.partial` file that would silently get
    // renamed as if complete. For catalog entries without a pinned SHA this is
    // the only line of defence: refuse to finalise a partial file that doesn't
    // match the expected byte count.
    validate_download_size(downloaded, total_bytes, model_id).inspect_err(|_| {
        // Drop the partial file so the next attempt starts clean rather than
        // resuming from a truncated state that just produced an error.
        let _ = std::fs::remove_file(&partial);
    })?;

    // ── SHA-256 verification (if hash is known) ───────────────────────────────
    if !entry.sha256.is_empty() {
        on_progress(ModelDownloadProgress {
            model_id: model_id.to_string(),
            downloaded_bytes: downloaded,
            total_bytes,
            status: "verifying".to_string(),
            error: None,
        });

        // Hash the entire file (handles resumed downloads correctly). A
        // cancellation here (or any IO error) leaves the `.partial` file in
        // place — the download itself already completed, so a future attempt
        // resumes straight into re-verifying rather than re-downloading.
        let hash = hash_file_sha256(&partial, cancel.as_ref())?;

        if hash != entry.sha256 {
            // Remove the corrupted file so the user can retry.
            let _ = std::fs::remove_file(&partial);
            return Err(AppError::AiError(format!(
                "SHA-256 mismatch for '{}': expected {}, got {}",
                model_id, entry.sha256, hash
            )));
        }
    }

    // ── Atomic rename partial → final ─────────────────────────────────────────
    std::fs::rename(&partial, &dest).map_err(|e| AppError::IoError(format!("Failed to finalise model file: {}", e)))?;

    Ok(dest)
}

// ── Link (use an already-downloaded file in place) ────────────────────────────

/// Reverse lookup: does `hash` belong to a *different* catalog entry than
/// the one the caller was trying to match? Used to turn a bare hash
/// mismatch into a pointed error ("this file is actually model X").
pub(crate) fn find_catalog_id_by_hash(hash: &str) -> Option<&'static crate::ai::model_catalog::CatalogModel> {
    crate::ai::model_catalog::CATALOG.iter().find(|m| m.sha256 == hash)
}

/// Adopt a user-supplied GGUF, already sitting somewhere on disk, as the
/// local copy of `entry` — without copying it. Verifies `source`'s SHA-256
/// against `entry.sha256`, then symlinks
/// `<app_data_dir>/models/{chat,embed}/<entry.id>.gguf` to `source`. The
/// user's original file is only ever read (to hash it), never moved,
/// modified, or deleted, so it survives even if the app's data directory is
/// later removed.
///
/// No-ops (`Ok(dest)`) if the destination already exists — same idempotency
/// as `download_model`. Rejects entries with no pinned hash: there would be
/// nothing to verify a local file against.
pub async fn link_local_model<F>(
    app_data_dir: &Path,
    entry: &crate::ai::model_catalog::CatalogModel,
    source: &Path,
    cancel: Option<CancelHandle>,
    on_progress: F,
) -> Result<PathBuf>
where
    F: Fn(ModelDownloadProgress) + Send + 'static,
{
    if entry.sha256.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "Model '{}' has no pinned checksum to verify a local file against",
            entry.id
        )));
    }

    let dest = model_path(app_data_dir, entry.kind, entry.id);
    if dest.exists() || std::fs::symlink_metadata(&dest).is_ok() {
        return Ok(dest);
    }

    if !source.is_file() {
        return Err(AppError::NotFound(format!("File not found: {}", source.display())));
    }
    let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if extension != "gguf" {
        return Err(AppError::InvalidInput(format!(
            "Expected a .gguf file, got: {}",
            source.display()
        )));
    }

    on_progress(ModelDownloadProgress {
        model_id: entry.id.to_string(),
        downloaded_bytes: 0,
        total_bytes: 0,
        status: "verifying".to_string(),
        error: None,
    });

    let hash = hash_file_sha256(source, cancel.as_ref())?;
    if hash != entry.sha256 {
        let detail = match find_catalog_id_by_hash(&hash) {
            Some(other) => format!(
                "This file matches '{}', not '{}'. Pick the correct file for this model.",
                other.display_name, entry.display_name
            ),
            None => format!(
                "SHA-256 mismatch for '{}': expected {}, got {}",
                entry.id, entry.sha256, hash
            ),
        };
        return Err(AppError::AiError(detail));
    }

    // `dest` is `<app_data_dir>/models/<kind>/<file>` — parent is always
    // present in the type, but the directory may not exist on first launch.
    #[allow(clippy::expect_used)]
    let dir = dest.parent().expect("model path always has a parent");
    std::fs::create_dir_all(dir).map_err(|e| AppError::IoError(format!("Cannot create models directory: {}", e)))?;

    std::os::unix::fs::symlink(source, &dest)
        .map_err(|e| AppError::IoError(format!("Failed to link model file: {}", e)))?;

    Ok(dest)
}

/// Verify a finished download matches the expected total byte count.
///
/// Pure helper so the truncation-detection logic is unit-testable without
/// having to spin up an HTTP server. Returns [`AppError::AiError`] with a
/// human-readable message when the sizes disagree.
fn validate_download_size(downloaded: u64, expected: u64, model_id: &str) -> Result<()> {
    if downloaded != expected {
        return Err(AppError::AiError(format!(
            "Download incomplete for '{}': got {} bytes, expected {}. The connection was likely interrupted — try again.",
            model_id, downloaded, expected
        )));
    }
    Ok(())
}

/// Compute the hex-encoded SHA-256 of a file. When `cancel` is provided and
/// flipped mid-hash, returns `AppError::Cancelled` without reading the rest
/// of the file — matters for `link_local_model`, where hashing (not a
/// network transfer) is the entire operation and can take real time on a
/// multi-GB file.
fn hash_file_sha256(path: &Path, cancel: Option<&CancelHandle>) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;

    let mut file =
        std::fs::File::open(path).map_err(|e| AppError::IoError(format!("Failed to open file to hash: {}", e)))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024]; // 1 MiB read buffer

    loop {
        if let Some(h) = cancel {
            if h.is_cancelled() {
                return Err(AppError::Cancelled);
            }
        }
        let n = file
            .read(&mut buf)
            .map_err(|e| AppError::IoError(format!("Failed to read file while hashing: {}", e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_download_size_rejects_truncated() {
        // Reproduces the production bug: HuggingFace responded with 3 013 027 808
        // bytes total but the stream ended after 2 871 743 520 (~141 MB short).
        // Without this check the partial file got renamed and llama.cpp failed
        // later with "null result" — far from the actual root cause.
        let err = validate_download_size(2_871_743_520, 3_013_027_808, "qwen3.5-4b-q4_k_m").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("incomplete"), "expected truncation error, got: {msg}");
        assert!(
            msg.contains("qwen3.5-4b-q4_k_m"),
            "error should mention model id: {msg}"
        );
    }

    #[test]
    fn validate_download_size_accepts_exact_match() {
        validate_download_size(3_013_027_808, 3_013_027_808, "qwen3.5-4b-q4_k_m").unwrap();
    }

    #[test]
    fn validate_download_size_rejects_overrun() {
        // A response that delivers more bytes than Content-Length advertised is
        // also wrong — silently accepting it would mask server / proxy bugs.
        let err = validate_download_size(3_013_027_900, 3_013_027_808, "qwen3.5-4b-q4_k_m").unwrap_err();
        assert!(err.to_string().contains("incomplete"));
    }

    #[test]
    fn validate_download_size_accepts_zero_byte_model() {
        // Degenerate but valid: catalog has size_bytes = 0 (placeholder entry).
        validate_download_size(0, 0, "placeholder").unwrap();
    }

    // ── hash_file_sha256 cancellation ────────────────────────────────────────

    #[test]
    fn hash_file_sha256_returns_cancelled_when_flagged_before_start() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.gguf");
        // Several MiB so a real (non-cancelled) hash would need multiple chunks.
        std::fs::write(&path, vec![7u8; 3 * 1024 * 1024]).unwrap();

        let (token, handle) = CancelToken::new();
        token.cancel();

        let err = hash_file_sha256(&path, Some(&handle)).unwrap_err();
        assert!(matches!(err, AppError::Cancelled), "expected Cancelled, got: {err:?}");
    }

    #[test]
    fn hash_file_sha256_without_cancel_handle_hashes_normally() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("small.gguf");
        std::fs::write(&path, b"hello world").unwrap();

        let hash = hash_file_sha256(&path, None).unwrap();
        assert_eq!(hash.len(), 64, "sha256 hex digest must be 64 chars");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── find_catalog_id_by_hash ──────────────────────────────────────────────

    #[test]
    fn find_catalog_id_by_hash_resolves_known_pinned_hash() {
        let entry = find_catalog_id_by_hash("13c16f426047e2de38cd075bdade4a7bcbc8c774384876f677740cda65f8a983")
            .expect("must resolve the real qwen3.5-4b-q4_k_m pinned hash");
        assert_eq!(entry.id, "qwen3.5-4b-q4_k_m");
    }

    #[test]
    fn find_catalog_id_by_hash_returns_none_for_unknown_hash() {
        assert!(find_catalog_id_by_hash("0000000000000000000000000000000000000000000000000000000000000").is_none());
    }

    // ── link_local_model ──────────────────────────────────────────────────────

    fn write_fixture_gguf(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn fixture_entry(id: &'static str, sha256: &'static str) -> crate::ai::model_catalog::CatalogModel {
        crate::ai::model_catalog::CatalogModel {
            id,
            display_name: "Fixture Model",
            kind: ModelKind::Chat,
            size_bytes: 0,
            context_window: 0,
            sha256,
            url: "https://example.invalid/fixture.gguf",
            license: "test",
            min_ram_gb: 0,
            recommended: false,
            supports_tools: true,
            bundled: false,
        }
    }

    #[tokio::test]
    async fn link_local_model_symlinks_dest_to_source_on_hash_match() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let data_dir = tmp.path().join("data");

        let content = b"fake gguf content for testing";
        let source = write_fixture_gguf(&src_dir, "my-model.gguf", content);
        let hash = hash_file_sha256(&source, None).unwrap();
        let entry = fixture_entry("fixture-chat", Box::leak(hash.into_boxed_str()));

        let dest = link_local_model(&data_dir, &entry, &source, None, |_| {})
            .await
            .unwrap();

        assert_eq!(dest, model_path(&data_dir, ModelKind::Chat, "fixture-chat"));
        let meta = std::fs::symlink_metadata(&dest).unwrap();
        assert!(meta.file_type().is_symlink(), "destination must be a symlink");
        assert_eq!(std::fs::read_link(&dest).unwrap(), source);
        // Source is untouched.
        assert_eq!(std::fs::read(&source).unwrap(), content);
    }

    #[tokio::test]
    async fn link_local_model_rejects_hash_mismatch_and_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let data_dir = tmp.path().join("data");

        let source = write_fixture_gguf(&src_dir, "my-model.gguf", b"actual content");
        let entry = fixture_entry(
            "fixture-chat",
            "0000000000000000000000000000000000000000000000000000000000000",
        );

        let err = link_local_model(&data_dir, &entry, &source, None, |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AiError(_)), "expected AiError, got: {err:?}");
        let dest = model_path(&data_dir, ModelKind::Chat, "fixture-chat");
        assert!(!dest.exists() && std::fs::symlink_metadata(&dest).is_err());
    }

    #[tokio::test]
    async fn link_local_model_rejects_missing_source_file() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let source = tmp.path().join("does-not-exist.gguf");
        let entry = fixture_entry("fixture-chat", Box::leak("a".repeat(64).into_boxed_str()));

        let err = link_local_model(&data_dir, &entry, &source, None, |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "expected NotFound, got: {err:?}");
    }

    #[tokio::test]
    async fn link_local_model_rejects_non_gguf_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let data_dir = tmp.path().join("data");

        let content = b"actual content";
        let source = write_fixture_gguf(&src_dir, "my-model.bin", content);
        let hash = hash_file_sha256(&source, None).unwrap();
        let entry = fixture_entry("fixture-chat", Box::leak(hash.into_boxed_str()));

        let err = link_local_model(&data_dir, &entry, &source, None, |_| {})
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::InvalidInput(_)),
            "expected InvalidInput, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn link_local_model_rejects_entry_with_empty_sha256_before_touching_source() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        // Bogus, nonexistent source — proves the empty-hash rejection happens
        // before any attempt to read or validate `source`.
        let source = tmp.path().join("nonexistent.gguf");
        let entry = fixture_entry("fixture-chat", "");

        let err = link_local_model(&data_dir, &entry, &source, None, |_| {})
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::InvalidInput(_)),
            "expected InvalidInput, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn link_local_model_is_noop_when_destination_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let dest = model_path(&data_dir, ModelKind::Chat, "fixture-chat");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"already here").unwrap();

        // A source that would fail validation (wrong extension) if it were
        // ever actually processed — proves the existing-destination
        // short-circuit happens before source validation.
        let source = tmp.path().join("would-fail.bin");
        let entry = fixture_entry("fixture-chat", Box::leak("a".repeat(64).into_boxed_str()));

        let result = link_local_model(&data_dir, &entry, &source, None, |_| {})
            .await
            .unwrap();
        assert_eq!(result, dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
    }

    // ── list_local_models symlink-awareness ──────────────────────────────────

    #[test]
    fn list_local_models_reports_symlink_target_size_and_is_linked() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let chat_dir = chat_models_dir(&data_dir);
        std::fs::create_dir_all(&chat_dir).unwrap();

        let external_dir = tmp.path().join("external");
        std::fs::create_dir_all(&external_dir).unwrap();
        let target = external_dir.join("real.gguf");
        std::fs::write(&target, vec![1u8; 12345]).unwrap();

        let link_path = chat_dir.join("linked-model.gguf");
        std::os::unix::fs::symlink(&target, &link_path).unwrap();

        let models = list_local_models(&data_dir);
        let linked = models
            .iter()
            .find(|m| m.id == "linked-model")
            .expect("linked model present");
        assert_eq!(
            linked.size_bytes, 12345,
            "must report the target's size, not the symlink's"
        );
        assert!(linked.is_linked, "symlinked entry must be flagged is_linked");
    }

    #[test]
    fn list_local_models_flags_regular_downloaded_file_as_not_linked() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let chat_dir = chat_models_dir(&data_dir);
        std::fs::create_dir_all(&chat_dir).unwrap();
        std::fs::write(chat_dir.join("downloaded-model.gguf"), vec![2u8; 999]).unwrap();

        let models = list_local_models(&data_dir);
        let downloaded = models
            .iter()
            .find(|m| m.id == "downloaded-model")
            .expect("downloaded model present");
        assert_eq!(downloaded.size_bytes, 999);
        assert!(!downloaded.is_linked, "regular file must not be flagged is_linked");
    }

    #[test]
    fn list_local_models_excludes_dangling_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let chat_dir = chat_models_dir(&data_dir);
        std::fs::create_dir_all(&chat_dir).unwrap();

        let external_dir = tmp.path().join("external");
        std::fs::create_dir_all(&external_dir).unwrap();
        let target = external_dir.join("real.gguf");
        std::fs::write(&target, b"content").unwrap();
        let link_path = chat_dir.join("dangling-model.gguf");
        std::os::unix::fs::symlink(&target, &link_path).unwrap();

        // Now remove the target — the symlink becomes dangling.
        std::fs::remove_file(&target).unwrap();

        let models = list_local_models(&data_dir);
        assert!(
            !models.iter().any(|m| m.id == "dangling-model"),
            "a dangling symlink must be excluded, not reported with size 0"
        );
    }

    // ── delete_local_model on a linked entry ─────────────────────────────────

    #[test]
    fn delete_local_model_removes_symlink_but_preserves_external_target() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let chat_dir = chat_models_dir(&data_dir);
        std::fs::create_dir_all(&chat_dir).unwrap();

        let external_dir = tmp.path().join("external");
        std::fs::create_dir_all(&external_dir).unwrap();
        let target = external_dir.join("real.gguf");
        std::fs::write(&target, b"user's original model bytes").unwrap();
        let link_path = chat_dir.join("fixture-chat.gguf");
        std::os::unix::fs::symlink(&target, &link_path).unwrap();

        delete_local_model(&data_dir, ModelKind::Chat, "fixture-chat").unwrap();

        assert!(!link_path.exists() && std::fs::symlink_metadata(&link_path).is_err());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"user's original model bytes",
            "the external target file must survive deleting the link"
        );
    }

    // ── seed_bundled_model ───────────────────────────────────────────────────

    #[test]
    fn seed_bundled_model_copies_when_destination_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("resources");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("test-embed-q4.gguf");
        std::fs::write(&src, b"fake gguf bytes").unwrap();

        let copied = seed_bundled_model(&src, &data_dir, ModelKind::Embedding, "test-embed-q4").unwrap();
        assert!(copied, "should report copied=true on first seed");

        let dest = model_path(&data_dir, ModelKind::Embedding, "test-embed-q4");
        assert!(dest.exists(), "destination must exist after seed");
        assert_eq!(std::fs::read(&dest).unwrap(), b"fake gguf bytes");
    }

    #[test]
    fn seed_bundled_model_is_noop_when_destination_already_present() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("resources");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("test-embed-q4.gguf");
        std::fs::write(&src, b"new bundled bytes").unwrap();

        // Pre-existing file at the destination — a user-downloaded copy that
        // we must not stomp on, even if it differs from the bundled one.
        let dest_dir = embed_models_dir(&data_dir);
        std::fs::create_dir_all(&dest_dir).unwrap();
        let dest = dest_dir.join("test-embed-q4.gguf");
        std::fs::write(&dest, b"user-installed bytes").unwrap();

        let copied = seed_bundled_model(&src, &data_dir, ModelKind::Embedding, "test-embed-q4").unwrap();
        assert!(!copied, "should report copied=false when destination exists");
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"user-installed bytes",
            "must leave the existing file untouched"
        );
    }

    #[test]
    fn seed_bundled_model_errors_when_source_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("does-not-exist.gguf");
        let data_dir = tmp.path().join("data");

        let err = seed_bundled_model(&src, &data_dir, ModelKind::Embedding, "ghost")
            .expect_err("missing source should error");
        let msg = err.to_string();
        assert!(msg.contains("ghost"), "error must mention model id: {msg}");
    }

    #[test]
    fn seed_bundled_model_creates_parent_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("nomic.gguf");
        std::fs::write(&src, b"x").unwrap();
        let data_dir = tmp.path().join("brand-new-data-dir");
        // Note: data_dir does NOT exist yet. seed must create models/embed/ under it.

        seed_bundled_model(&src, &data_dir, ModelKind::Embedding, "nomic").unwrap();
        assert!(embed_models_dir(&data_dir).exists(), "models/embed/ must be created");
    }
}
