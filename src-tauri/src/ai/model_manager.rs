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
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
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

        // Hash the entire file (handles resumed downloads correctly).
        let hash =
            hash_file_sha256(&partial).map_err(|e| AppError::IoError(format!("Failed to hash model file: {}", e)))?;

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

/// Compute the hex-encoded SHA-256 of a file.
fn hash_file_sha256(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024]; // 1 MiB read buffer

    loop {
        let n = file.read(&mut buf)?;
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
