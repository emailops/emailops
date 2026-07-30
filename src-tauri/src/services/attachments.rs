use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::Regex;
use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, AppLogEvent, Attachment, AttachmentRule};
use crate::services::emails::build_provider;
use crate::sync::provider::{AttachmentInfo, EmailProvider};

/// Emit an `app-log` event when an `AppHandle` is available.
///
/// `apply_rule_retroactively` and `process_attachments_for_email` accept
/// `Option<&AppHandle>` so they remain unit-testable without a Tauri runtime
/// — in those tests `app` is `None` and the events are silently dropped.
fn emit_log(app: Option<&AppHandle>, level: &str, source: &str, message: impl Into<String>) {
    if let Some(app) = app {
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: level.to_string(),
                source: source.to_string(),
                message: message.into(),
            },
        );
    }
}

/// Resolve a stored attachment `file_path` against `app_data_dir` and verify
/// that the result lives inside the data dir. Canonicalising both sides
/// dereferences any `..` segments and follows symlinks, so a row with a
/// crafted `file_path` like `../../Library/Keychains/login.keychain-db`
/// cannot be turned into a successful read/open. The canonical-form check
/// also rejects paths that point outside via a symlink that was created
/// inside the data dir.
///
/// Returns the canonical absolute path on success; returns an error on
/// missing files or escape attempts. Callers that want "missing file is OK"
/// (e.g. bulk copy that skips gone files) should detect `AppError::NotFound`
/// rather than papering over with `exists()` first — TOCTOU-safe.
pub fn safe_attachment_path(app_data_dir: &Path, file_path: &str) -> Result<PathBuf> {
    if file_path.is_empty() {
        return Err(AppError::InvalidInput("Empty attachment file path".into()));
    }

    let raw = app_data_dir.join(file_path);
    let canonical = raw.canonicalize().map_err(|e| {
        // ErrorKind::NotFound here means the row pointed at a file that's
        // gone — surface as NotFound so callers can distinguish from
        // "rejected for being outside the sandbox".
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::NotFound(format!("Attachment file not found: {file_path}"))
        } else {
            AppError::IoError(format!("Failed to resolve attachment path '{file_path}': {e}"))
        }
    })?;

    let canonical_root = app_data_dir.canonicalize().map_err(|e| {
        AppError::IoError(format!(
            "Failed to canonicalize app data dir '{}': {e}",
            app_data_dir.display()
        ))
    })?;

    if !canonical.starts_with(&canonical_root) {
        return Err(AppError::InvalidInput(format!(
            "Attachment path '{file_path}' resolves outside the application data directory"
        )));
    }

    Ok(canonical)
}

/// Reduce a client-supplied filename to a safe bare file name: path
/// separators and `.`/`..` segments are dropped so a crafted name like
/// `../../evil.sh` cannot escape the destination directory.
fn sanitize_download_filename(filename: &str) -> String {
    let name = filename
        .replace('\\', "/")
        .split('/')
        .rfind(|part| !part.is_empty() && *part != "." && *part != "..")
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        "attachment".to_string()
    } else {
        name
    }
}

/// Pick a non-colliding destination path in `dir` for `filename`, appending
/// ` (1)`, ` (2)`, … before the extension while a file with that name exists.
pub fn unique_download_path(dir: &Path, filename: &str) -> PathBuf {
    let dest = dir.join(filename);
    if !dest.exists() {
        return dest;
    }
    let stem = Path::new(filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = Path::new(filename)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 1u32;
    loop {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Write attachment bytes into `dir` (the user's Downloads folder) under a
/// sanitized, collision-free name. Returns the path actually written.
pub fn save_bytes_to_downloads(dir: &Path, filename: &str, bytes: &[u8]) -> Result<PathBuf> {
    let safe_name = sanitize_download_filename(filename);
    let dest = unique_download_path(dir, &safe_name);
    std::fs::write(&dest, bytes)
        .map_err(|e| AppError::IoError(format!("Failed to save {} to Downloads: {e}", dest.display())))?;
    Ok(dest)
}

/// Validate a frontend-supplied path before revealing it in the OS file
/// manager: it must exist and canonicalize to somewhere inside the user's
/// Downloads folder (the folder itself is allowed — bulk downloads reveal
/// the whole directory). Anything else is rejected so the reveal command
/// can't be pointed at arbitrary filesystem locations.
pub fn validate_reveal_path(downloads_dir: &Path, path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::NotFound(format!("File not found: {}", path.display()))
        } else {
            AppError::IoError(format!("Failed to resolve path '{}': {e}", path.display()))
        }
    })?;
    let root = downloads_dir.canonicalize().map_err(|e| {
        AppError::IoError(format!(
            "Failed to canonicalize Downloads dir '{}': {e}",
            downloads_dir.display()
        ))
    })?;
    if !canonical.starts_with(&root) {
        return Err(AppError::InvalidInput(format!(
            "Path '{}' is outside the Downloads folder",
            path.display()
        )));
    }
    Ok(canonical)
}

/// Reveal an already-validated path in the OS file manager. On macOS a file
/// is selected in Finder (`open -R`); a directory is opened directly.
pub fn reveal_in_file_manager(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if path.is_dir() {
            cmd.arg(path);
        } else {
            cmd.arg("-R").arg(path);
        }
        cmd.spawn()
            .map_err(|e| AppError::IoError(format!("Failed to reveal in Finder: {e}")))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf())
        };
        open::that(target).map_err(|e| AppError::IoError(format!("Failed to open folder: {e}")))?;
        Ok(())
    }
}

/// Decode an inline attachment payload from any of the base64 flavors providers
/// hand us. Gmail's `data` field is URL-safe base64 (RFC 4648 §5, alphabet
/// `-_`); Microsoft Graph's `contentBytes` is standard base64 (RFC 4648 §4,
/// alphabet `+/`). Both arrive in the same `AttachmentInfo::inline_data` slot,
/// so the decoder must be lenient about either alphabet.
///
/// Implementation: try the standard alphabet first (it's the universal default
/// and matches Outlook, which is where this came up — see the
/// `decode_inline_base64_accepts_outlook_standard_alphabet` regression test).
/// If that fails, fall through to the URL-safe alphabet for Gmail. Both
/// passes are padding-indifferent and tolerate whitespace.
pub(crate) fn decode_inline_base64(data: &str) -> crate::models::error::Result<Vec<u8>> {
    use base64::{
        alphabet,
        engine::{self, GeneralPurpose, GeneralPurposeConfig},
        Engine,
    };

    const STANDARD: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
    );
    const URL_SAFE: GeneralPurpose = GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
    );

    let cleaned: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    match STANDARD.decode(&cleaned) {
        Ok(bytes) => Ok(bytes),
        Err(std_err) => URL_SAFE.decode(&cleaned).map_err(|url_err| {
            // Both decoders failed — surface both errors so debugging surfaces
            // a useful clue even when the payload is genuinely corrupt.
            AppError::SyncError(format!(
                "Base64 decode error (standard: {}; url-safe: {})",
                std_err, url_err
            ))
        }),
    }
}

// --- Rule matching ---

/// Convert a user-friendly glob pattern to a regex string.
/// `*` becomes `.*`, `?` becomes `.`, everything else is escaped.
fn glob_to_regex(pattern: &str) -> String {
    let mut regex = String::with_capacity(pattern.len() * 2 + 4);
    regex.push_str("(?i)^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '^' | '$' | '|' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    regex.push('$');
    regex
}

fn matches_glob(pattern: &str, value: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        let regex_str = glob_to_regex(pattern);
        Regex::new(&regex_str).map(|re| re.is_match(value)).unwrap_or(false)
    } else {
        value.eq_ignore_ascii_case(pattern)
    }
}

/// Check if a filename matches a rule's filename pattern.
pub fn matches_filename(rule: &AttachmentRule, filename: &str) -> bool {
    match &rule.filename_pattern {
        Some(pattern) if !pattern.is_empty() => matches_glob(pattern, filename),
        _ => true,
    }
}

/// Check if an email matches a rule's sender/subject criteria.
/// The sender pattern supports comma-separated values (OR logic between entries).
pub fn matches_rule(rule: &AttachmentRule, sender_email: &str, subject: &str) -> bool {
    let sender_match = match &rule.sender_email_pattern {
        Some(pattern) if !pattern.is_empty() => pattern
            .split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .any(|p| matches_glob(p, sender_email)),
        _ => true,
    };
    if !sender_match {
        return false;
    }

    match &rule.subject_pattern {
        Some(pattern) if !pattern.is_empty() => matches_glob(pattern, subject),
        _ => true,
    }
}

// --- Attachment processing during sync ---

/// Re-fetch a single message from its provider and re-extract its attachment
/// metadata into `email_attachment_meta`. Repairs emails whose attachments were
/// missed at original sync time — incremental sync skips already-stored messages,
/// so a normal re-sync never revisits them, leaving the gap permanent.
///
/// Returns the attachment infos the provider reports for the message (so callers
/// can show what was found / recovered). Idempotent: the batch upsert is
/// `ON CONFLICT(email_id, filename) DO NOTHING`, so re-running never duplicates.
/// `app` may be `None` (CLI / example / test contexts).
pub async fn reextract_email_attachments(
    db: &Arc<Database>,
    account: &Account,
    email_id: &str,
    app: Option<AppHandle>,
) -> Result<Vec<AttachmentInfo>> {
    let provider = build_provider(account, app).await?;
    reextract_with_provider(db, provider.as_ref(), &account.id, email_id).await
}

/// Provider-injected core (trait seam) so the upsert behaviour is unit-testable
/// with a fake provider. `get_message` fetches the full MIME payload
/// (format=full) and runs the same `collect_attachment_infos` the sync path uses
/// — so this both repairs and diagnoses (an empty result means the parser missed
/// the part, not that the original fetch was incomplete).
pub(crate) async fn reextract_with_provider(
    db: &Arc<Database>,
    provider: &dyn EmailProvider,
    account_id: &str,
    email_id: &str,
) -> Result<Vec<AttachmentInfo>> {
    let (_email, _category, infos) = provider.get_message(email_id).await?;
    if !infos.is_empty() {
        // The 7-tuple shape is the existing contract of
        // `insert_email_attachment_metas_batch`; mirror it here rather than
        // invent a parallel struct just for this one call.
        #[allow(clippy::type_complexity)]
        let metas: Vec<(String, String, String, String, String, i64, Option<String>)> = infos
            .iter()
            .map(|i| {
                (
                    email_id.to_string(),
                    account_id.to_string(),
                    i.attachment_id.clone(),
                    i.filename.clone(),
                    i.mime_type.clone(),
                    i.size,
                    i.inline_data.clone(),
                )
            })
            .collect();
        db.insert_email_attachment_metas_batch(&metas)?;
    }
    Ok(infos)
}

pub async fn process_attachments_for_email(
    db: &Arc<Database>,
    provider: &dyn EmailProvider,
    email: &crate::models::Email,
    attachment_infos: &[AttachmentInfo],
    rules: &[AttachmentRule],
    app_data_dir: &Path,
    app: Option<&AppHandle>,
) -> Result<u32> {
    let mut count = 0u32;

    for rule in rules {
        if !matches_rule(rule, &email.sender_email, &email.subject) {
            continue;
        }

        for info in attachment_infos {
            // Skip attachments that don't match the filename pattern
            if !matches_filename(rule, &info.filename) {
                continue;
            }

            // Dedup by (email_id, filename, rule_id)
            if db.attachment_exists(&email.id, &info.filename, &rule.id)? {
                continue;
            }

            // Get binary data: either from inline data or by fetching via API
            let bytes = if let Some(ref inline_b64) = info.inline_data {
                match decode_inline_base64(inline_b64) {
                    Ok(b) => b,
                    Err(e) => {
                        emit_log(
                            app,
                            "error",
                            "attachments",
                            format!("Failed to decode inline attachment '{}': {}", info.filename, e),
                        );
                        continue;
                    }
                }
            } else {
                match provider.fetch_attachment_bytes(&email.id, &info.attachment_id).await {
                    Ok(b) => b,
                    Err(e) => {
                        emit_log(
                            app,
                            "error",
                            "attachments",
                            format!("Failed to download attachment '{}': {}", info.filename, e),
                        );
                        continue;
                    }
                }
            };

            // Determine file extension and path
            let ext = info.filename.rsplit('.').next().unwrap_or("bin");
            let file_id = uuid::Uuid::new_v4().to_string();
            let relative_path = format!("attachments/{}/{}.{}", email.account_id, file_id, ext);
            let absolute_path = app_data_dir.join(&relative_path);

            // Create directory and write file
            if let Some(parent) = absolute_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| AppError::IoError(format!("Failed to create attachment directory: {}", e)))?;
            }
            tokio::fs::write(&absolute_path, &bytes)
                .await
                .map_err(|e| AppError::IoError(format!("Failed to write attachment file: {}", e)))?;

            let attachment = Attachment {
                id: uuid::Uuid::new_v4().to_string(),
                account_id: email.account_id.clone(),
                email_id: email.id.clone(),
                rule_id: rule.id.clone(),
                gmail_attachment_id: info.attachment_id.clone(),
                filename: info.filename.clone(),
                mime_type: info.mime_type.clone(),
                file_size: bytes.len() as i64,
                file_path: relative_path,
                tags: rule.tags.clone(),
                sender_email: email.sender_email.clone(),
                subject: email.subject.clone(),
                email_timestamp: email.timestamp,
                created_at: chrono::Utc::now().timestamp(),
            };

            db.insert_attachment(&attachment)?;
            count += 1;
        }
    }

    // Notify the frontend so the attachments list refreshes without an app
    // restart. Sync and retroactive-scan paths both insert via this function;
    // a single event per email (only when something was saved) is enough.
    if count > 0 {
        if let Some(app) = app {
            let _ = app.emit("attachments-updated", &email.account_id);
        }
    }

    Ok(count)
}

/// Download all attachments for an email to disk (no rule required).
/// Used when the user has enabled auto-download for the email's category.
/// Updates `file_path` in `email_attachment_meta` so the frontend knows the file is local.
pub async fn auto_download_attachments(
    db: &Arc<Database>,
    provider: &dyn EmailProvider,
    email: &crate::models::Email,
    attachment_infos: &[AttachmentInfo],
    app_data_dir: &Path,
    app: &AppHandle,
) -> Result<u32> {
    let mut count = 0u32;

    for info in attachment_infos {
        // Get binary data
        let bytes = if let Some(ref inline_b64) = info.inline_data {
            match decode_inline_base64(inline_b64) {
                Ok(b) => b,
                Err(e) => {
                    let _ = app.emit(
                        "app-log",
                        AppLogEvent {
                            level: "error".to_string(),
                            source: "attachments".to_string(),
                            message: format!("Auto-download: failed to decode '{}': {}", info.filename, e),
                        },
                    );
                    continue;
                }
            }
        } else if !info.attachment_id.is_empty() {
            match provider.fetch_attachment_bytes(&email.id, &info.attachment_id).await {
                Ok(b) => b,
                Err(e) => {
                    let _ = app.emit(
                        "app-log",
                        AppLogEvent {
                            level: "error".to_string(),
                            source: "attachments".to_string(),
                            message: format!("Auto-download: failed to fetch '{}': {}", info.filename, e),
                        },
                    );
                    continue;
                }
            }
        } else {
            continue; // No data source available
        };

        let ext = info.filename.rsplit('.').next().unwrap_or("bin");
        let file_id = uuid::Uuid::new_v4().to_string();
        let relative_path = format!("attachments/{}/auto/{}.{}", email.account_id, file_id, ext);
        let absolute_path = app_data_dir.join(&relative_path);

        if let Some(parent) = absolute_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                let _ = app.emit(
                    "app-log",
                    AppLogEvent {
                        level: "error".to_string(),
                        source: "attachments".to_string(),
                        message: format!("Auto-download: mkdir failed: {}", e),
                    },
                );
                continue;
            }
        }
        if let Err(e) = tokio::fs::write(&absolute_path, &bytes).await {
            let _ = app.emit(
                "app-log",
                AppLogEvent {
                    level: "error".to_string(),
                    source: "attachments".to_string(),
                    message: format!("Auto-download: write failed for '{}': {}", info.filename, e),
                },
            );
            continue;
        }

        let _ = db.set_email_attachment_file_path(&email.id, &info.filename, &relative_path);
        count += 1;
    }

    Ok(count)
}

// --- CRUD for rules ---

pub fn create_rule(
    db: &Arc<Database>,
    account_id: &str,
    name: &str,
    sender_email_pattern: Option<&str>,
    subject_pattern: Option<&str>,
    filename_pattern: Option<&str>,
    tags: Vec<String>,
) -> Result<AttachmentRule> {
    let has_sender = sender_email_pattern.is_some_and(|p| !p.is_empty());
    let has_subject = subject_pattern.is_some_and(|p| !p.is_empty());
    let has_filename = filename_pattern.is_some_and(|p| !p.is_empty());
    if !has_sender && !has_subject && !has_filename {
        return Err(AppError::InvalidInput(
            "At least one pattern (sender, subject, or filename) must be specified".to_string(),
        ));
    }

    let now = chrono::Utc::now().timestamp();
    let rule = AttachmentRule {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        name: name.to_string(),
        sender_email_pattern: sender_email_pattern.filter(|p| !p.is_empty()).map(String::from),
        subject_pattern: subject_pattern.filter(|p| !p.is_empty()).map(String::from),
        filename_pattern: filename_pattern.filter(|p| !p.is_empty()).map(String::from),
        tags,
        enabled: true,
        created_at: now,
        updated_at: now,
    };

    db.insert_attachment_rule(&rule)?;
    Ok(rule)
}

pub fn update_rule(
    db: &Arc<Database>,
    rule_id: &str,
    name: &str,
    sender_email_pattern: Option<&str>,
    subject_pattern: Option<&str>,
    filename_pattern: Option<&str>,
    tags: Vec<String>,
    enabled: bool,
    app_data_dir: &Path,
) -> Result<AttachmentRule> {
    let existing = db
        .get_attachment_rule(rule_id)?
        .ok_or_else(|| AppError::NotFound(format!("Rule {} not found", rule_id)))?;

    let has_sender = sender_email_pattern.is_some_and(|p| !p.is_empty());
    let has_subject = subject_pattern.is_some_and(|p| !p.is_empty());
    let has_filename = filename_pattern.is_some_and(|p| !p.is_empty());
    if !has_sender && !has_subject && !has_filename {
        return Err(AppError::InvalidInput(
            "At least one pattern (sender, subject, or filename) must be specified".to_string(),
        ));
    }

    let rule = AttachmentRule {
        id: existing.id,
        account_id: existing.account_id,
        name: name.to_string(),
        sender_email_pattern: sender_email_pattern.filter(|p| !p.is_empty()).map(String::from),
        subject_pattern: subject_pattern.filter(|p| !p.is_empty()).map(String::from),
        filename_pattern: filename_pattern.filter(|p| !p.is_empty()).map(String::from),
        tags,
        enabled,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now().timestamp(),
    };

    db.update_attachment_rule(&rule)?;

    // Re-evaluate existing attachments: delete those that no longer match, update tags on keepers
    let existing = db.get_attachments_for_rule(&rule.id)?;
    for att in &existing {
        let still_matches =
            matches_rule(&rule, &att.sender_email, &att.subject) && matches_filename(&rule, &att.filename);
        if still_matches {
            continue;
        }
        // No longer matches — delete file and DB row
        if let Some(path) = db.delete_attachment_by_id(&att.id)? {
            let abs_path = app_data_dir.join(&path);
            if abs_path.exists() {
                let _ = std::fs::remove_file(&abs_path);
            }
        }
    }

    // Update tags on remaining attachments
    db.update_attachments_tags_for_rule(&rule.id, &rule.tags)?;

    Ok(rule)
}

pub fn delete_rule(db: &Arc<Database>, rule_id: &str, account_id: &str, app_data_dir: &Path) -> Result<()> {
    // Delete attachment files from disk first
    let file_paths = db.delete_attachments_for_rule(rule_id)?;
    for relative_path in &file_paths {
        let abs_path = app_data_dir.join(relative_path);
        if abs_path.exists() {
            let _ = std::fs::remove_file(&abs_path);
        }
    }
    db.delete_attachment_rule(rule_id, account_id)?;
    Ok(())
}

pub fn list_rules(db: &Arc<Database>, account_id: &str) -> Result<Vec<AttachmentRule>> {
    db.get_all_attachment_rules(account_id)
}

pub fn get_attachments(
    db: &Arc<Database>,
    account_id: &str,
    tag: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Attachment>> {
    db.get_attachments(account_id, tag, limit, offset)
}

pub fn count_attachments(db: &Arc<Database>, account_id: &str, tag: Option<&str>) -> Result<i32> {
    db.count_attachments(account_id, tag)
}

pub fn get_attachment(db: &Arc<Database>, attachment_id: &str) -> Result<Option<Attachment>> {
    db.get_attachment(attachment_id)
}

pub fn get_tags(db: &Arc<Database>, account_id: &str) -> Result<Vec<String>> {
    db.get_all_tags(account_id)
}

/// List every attachment surfaced on one email: both the canonical
/// `email_attachment_meta` rows (everything sync discovered) and the
/// rule-matched `attachments` rows. Two separate result lists since their
/// IDs use distinct namespaces (the UI / chat tool needs both). Backs the
/// chat `get_attachments` tool.
pub fn list_for_email(
    db: &Arc<Database>,
    email_id: &str,
) -> Result<(Vec<crate::models::EmailAttachmentMeta>, Vec<Attachment>)> {
    let metas = db.get_email_attachment_metas(email_id)?;
    let rule_matched = db.get_attachments_for_email(email_id)?;
    Ok((metas, rule_matched))
}

// --- Retroactive rule application ---

pub async fn apply_rule_retroactively(
    db: &Arc<Database>,
    rule_id: &str,
    account_id: &str,
    app_data_dir: &Path,
    app: Option<&AppHandle>,
) -> Result<u32> {
    let rule = db
        .get_attachment_rule(rule_id)?
        .ok_or_else(|| AppError::NotFound(format!("Rule {} not found", rule_id)))?;

    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    // Delegate provider construction to the canonical helper. Previously this
    // function had a hand-rolled `match account.provider.as_str()` that only
    // supported `"gmail"` and returned `Err(SyncError("Unsupported provider: ..."))`
    // for Outlook and IMAP accounts — so clicking "Sync now" on a rule attached
    // to a non-Gmail account always failed. `build_provider` dispatches on
    // provider and handles OAuth refresh consistently with the rest of the app.
    let provider = build_provider(&account, app.cloned()).await?;

    apply_rule_with_provider(db, &rule, account_id, provider.as_ref(), app_data_dir, app).await
}

/// Apply a rule retroactively using a caller-supplied provider. Extracted from
/// `apply_rule_retroactively` so unit tests can drive the loop with
/// `FakeEmailProvider` without a Tauri runtime or live OAuth tokens.
pub async fn apply_rule_with_provider(
    db: &Arc<Database>,
    rule: &AttachmentRule,
    account_id: &str,
    provider: &dyn EmailProvider,
    app_data_dir: &Path,
    app: Option<&AppHandle>,
) -> Result<u32> {
    // Get all emails for this account with basic info
    let email_rows = db.get_emails_matching_rule(account_id)?;

    emit_log(
        app,
        "info",
        "attachments",
        format!("Scanning {} emails for rule '{}'...", email_rows.len(), rule.name),
    );

    let mut total_attachments = 0u32;
    let mut emails_matching_criteria = 0u32;
    let mut emails_with_attachments = 0u32;
    let mut emails_with_filename_match = 0u32;
    let rules = std::slice::from_ref(rule);

    for (email_id, sender_email, subject) in &email_rows {
        if !matches_rule(rule, sender_email, subject) {
            continue;
        }
        emails_matching_criteria += 1;

        // Fetch full message to get attachment info
        let result = provider.get_message(email_id).await;
        let (email, _category, attachment_infos) = match result {
            Ok(r) => r,
            Err(e) => {
                emit_log(
                    app,
                    "warn",
                    "attachments",
                    format!("Skipping email {}: {}", email_id, e),
                );
                continue;
            }
        };

        if attachment_infos.is_empty() {
            continue;
        }
        emails_with_attachments += 1;

        if attachment_infos
            .iter()
            .any(|info| matches_filename(rule, &info.filename))
        {
            emails_with_filename_match += 1;
        }

        let mut email_with_account = email;
        email_with_account.account_id = account_id.to_string();

        let count = process_attachments_for_email(
            db,
            provider,
            &email_with_account,
            &attachment_infos,
            rules,
            app_data_dir,
            app,
        )
        .await?;
        total_attachments += count;
    }

    // Build a human-readable diagnostic summary so the user can tell which
    // stage filtered everything out (sender/subject vs. no attachments vs.
    // filename pattern). Without this, a "0 attachments" result is opaque.
    let summary = format!(
        "Rule '{}': sender/subject matched {}/{} emails; {} of those had attachments; {} had filename matches; saved {} new attachments.",
        rule.name,
        emails_matching_criteria,
        email_rows.len(),
        emails_with_attachments,
        emails_with_filename_match,
        total_attachments,
    );
    let level = if total_attachments > 0 { "success" } else { "warn" };
    emit_log(app, level, "attachments", summary);

    // If 0 emails matched the sender/subject criteria at all, surface a hint —
    // by far the most common cause is a too-strict pattern (e.g. `apple.com`
    // when the actual sender is `no_reply@email.apple.com`, where the user
    // needs `*apple.com*`).
    if emails_matching_criteria == 0 && !email_rows.is_empty() {
        emit_log(
            app,
            "warn",
            "attachments",
            format!(
                "Rule '{}' matched no emails. Patterns are exact-match unless they contain `*` — try `*apple.com*` instead of `apple.com`, or leave the sender field empty and filter by subject/filename.",
                rule.name
            ),
        );
    }

    Ok(total_attachments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AttachmentRule;

    fn make_rule(sender: Option<&str>, subject: Option<&str>) -> AttachmentRule {
        make_rule_with_filename(sender, subject, None)
    }

    fn make_rule_with_filename(sender: Option<&str>, subject: Option<&str>, filename: Option<&str>) -> AttachmentRule {
        AttachmentRule {
            id: "r1".into(),
            account_id: "a1".into(),
            name: "test".into(),
            sender_email_pattern: sender.map(String::from),
            subject_pattern: subject.map(String::from),
            filename_pattern: filename.map(String::from),
            tags: vec![],
            enabled: true,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn exact_sender_match() {
        let rule = make_rule(Some("invoices@aws.com"), None);
        assert!(matches_rule(&rule, "invoices@aws.com", "anything"));
        assert!(matches_rule(&rule, "Invoices@AWS.com", "anything"));
        assert!(!matches_rule(&rule, "other@aws.com", "anything"));
    }

    #[test]
    fn multiple_senders_comma_separated() {
        let rule = make_rule(Some("invoices@aws.com, billing@gcp.com"), None);
        assert!(matches_rule(&rule, "invoices@aws.com", "anything"));
        assert!(matches_rule(&rule, "billing@gcp.com", "anything"));
        assert!(!matches_rule(&rule, "noreply@azure.com", "anything"));
    }

    #[test]
    fn multiple_senders_with_globs() {
        let rule = make_rule(Some("*@aws.com, *@gcp.com"), None);
        assert!(matches_rule(&rule, "invoices@aws.com", "anything"));
        assert!(matches_rule(&rule, "billing@gcp.com", "anything"));
        assert!(!matches_rule(&rule, "billing@azure.com", "anything"));
    }

    #[test]
    fn glob_sender_match() {
        let rule = make_rule(Some("*@aws.com"), None);
        assert!(matches_rule(&rule, "invoices@aws.com", "anything"));
        assert!(matches_rule(&rule, "billing@aws.com", "anything"));
        assert!(!matches_rule(&rule, "invoices@gcp.com", "anything"));
    }

    #[test]
    fn glob_subject_match() {
        let rule = make_rule(None, Some("*monthly invoice*"));
        assert!(matches_rule(
            &rule,
            "anyone@example.com",
            "Your monthly invoice for March"
        ));
        assert!(matches_rule(&rule, "anyone@example.com", "MONTHLY INVOICE #123"));
        assert!(!matches_rule(&rule, "anyone@example.com", "Weekly report"));
    }

    #[test]
    fn combined_sender_and_subject() {
        let rule = make_rule(Some("invoices@aws.com"), Some("*monthly invoice*"));
        assert!(matches_rule(
            &rule,
            "invoices@aws.com",
            "Your monthly invoice for March"
        ));
        assert!(!matches_rule(&rule, "other@aws.com", "Your monthly invoice for March"));
        assert!(!matches_rule(&rule, "invoices@aws.com", "Some other subject"));
    }

    #[test]
    fn filename_glob_match() {
        let rule = make_rule_with_filename(None, None, Some("Invoice-*.pdf"));
        assert!(matches_filename(&rule, "Invoice-2024-03.pdf"));
        assert!(matches_filename(&rule, "invoice-march.pdf"));
        assert!(!matches_filename(&rule, "Receipt-2024.pdf"));
        assert!(!matches_filename(&rule, "Invoice-2024.xlsx"));
    }

    #[test]
    fn filename_only_rule() {
        let rule = make_rule_with_filename(None, None, Some("*.pdf"));
        // Email-level matching passes (no sender/subject constraints)
        assert!(matches_rule(&rule, "anyone@example.com", "any subject"));
        // Filename filter
        assert!(matches_filename(&rule, "report.pdf"));
        assert!(!matches_filename(&rule, "report.xlsx"));
    }

    #[test]
    fn no_filename_pattern_matches_all() {
        let rule = make_rule(Some("a@b.com"), None);
        assert!(matches_filename(&rule, "anything.pdf"));
    }

    #[test]
    fn glob_to_regex_basic() {
        assert_eq!(glob_to_regex("*test*"), "(?i)^.*test.*$");
        assert_eq!(glob_to_regex("hello.world"), "(?i)^hello\\.world$");
    }

    use crate::db::Database;
    use crate::models::Email;
    use crate::sync::provider::{AttachmentInfo, EmailCategory, FakeEmailProvider};

    fn make_account(db: &Database, id: &str, provider: &str, email: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled, sync_from_timestamp) \
                 VALUES (?1, ?2, ?3, 'Test', 0, 0, 1, NULL)",
                rusqlite::params![id, provider, email],
            )
            .expect("insert account");
    }

    fn make_email(account_id: &str, id: &str, sender_email: &str, subject: &str) -> Email {
        Email {
            id: id.into(),
            account_id: account_id.into(),
            thread_id: id.into(),
            message_id: None,
            subject: subject.into(),
            sender: "Sender".into(),
            sender_email: sender_email.into(),
            recipients: vec!["me@example.com".into()],
            cc: vec![],
            body: "body".into(),
            snippet: "snippet".into(),
            timestamp: 1_700_000_000,
            is_read: false,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            headers: None,
        }
    }

    // ── Inline base64 decoding — Outlook regression ─────────────────────────
    //
    // Outlook's Graph API returns `contentBytes` as **standard** base64
    // (alphabet `+/`), not URL-safe. The previous decoder only accepted
    // URL-safe input, so any Outlook inline attachment containing a `+` or
    // `/` symbol failed with errors like:
    //
    //     "Base64 decode error: Invalid symbol 43, offset 2571."
    //     "Base64 decode error: Invalid symbol 47, offset 39."
    //
    // (43 = `+`, 47 = `/`). The result was that retroactive rule sync on
    // Outlook accounts reported "saved 0 new attachments" for every inline
    // payload that happened to encode bytes whose base64 form needed
    // either of those symbols.

    #[test]
    fn decode_inline_base64_accepts_outlook_standard_alphabet() {
        // Plain "Hello+World/" — symbols 43 and 47 land in the encoded form
        // because the source bytes themselves contain values that map there.
        let original: &[u8] = b"\xfb\xff\xbf";
        // Encode with the STANDARD alphabet — guaranteed to include `+` and `/`.
        let encoded = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(original)
        };
        assert!(
            encoded.contains('+') || encoded.contains('/'),
            "test payload must exercise standard-alphabet symbols, got {}",
            encoded
        );
        let decoded = decode_inline_base64(&encoded).expect("standard base64 must decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_inline_base64_accepts_url_safe_alphabet() {
        let original: &[u8] = b"\xfb\xff\xbf";
        let encoded = {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE.encode(original)
        };
        assert!(
            encoded.contains('-') || encoded.contains('_'),
            "test payload must exercise url-safe-alphabet symbols, got {}",
            encoded
        );
        let decoded = decode_inline_base64(&encoded).expect("url-safe base64 must decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_inline_base64_tolerates_whitespace_and_missing_padding() {
        // Some providers split base64 across lines; the decoder must ignore
        // whitespace and tolerate missing `=` padding.
        let encoded = "SGVs\nbG8g\nV29y\nbGQ"; // "Hello World" without trailing `=`
        let decoded = decode_inline_base64(encoded).expect("lenient decode");
        assert_eq!(decoded, b"Hello World");
    }

    #[test]
    fn decode_inline_base64_surfaces_both_errors_for_genuinely_corrupt_input() {
        // `!` is in neither alphabet, so both passes must fail and the error
        // message should mention both flavors to aid debugging.
        let err = decode_inline_base64("!!!").expect_err("corrupt input must error");
        let msg = err.to_string();
        assert!(msg.contains("standard"), "missing standard hint: {}", msg);
        assert!(msg.contains("url-safe"), "missing url-safe hint: {}", msg);
    }

    /// End-to-end: `process_attachments_for_email` persists an inline
    /// attachment whose payload is encoded with the **standard** alphabet
    /// (the Outlook flavor). Before the fix this skipped the attachment with
    /// "Failed to decode inline attachment ...: Invalid symbol 43".
    #[tokio::test]
    async fn process_attachments_for_email_handles_standard_base64_from_outlook() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");

        let account_id = "acc-outlook";
        make_account(&db, account_id, "outlook", "me@outlook.example");
        let email = make_email(account_id, "msg-1", "me@outlook.example", "FYI");
        db.insert_email(&email).expect("insert email");

        let rule = create_rule(
            &db,
            account_id,
            "anything from me",
            Some("me@outlook.example"),
            None,
            None,
            vec!["self".into()],
        )
        .expect("create rule");

        // Payload chosen so its standard-base64 encoding contains both `+` (43)
        // and `/` (47) — the exact symbols the user's logs flagged.
        let payload: &[u8] = b"\xfb\xff\xbf\xff\xfe\xff";
        let standard_b64 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(payload)
        };
        assert!(standard_b64.contains('+') || standard_b64.contains('/'));

        let infos = vec![AttachmentInfo {
            attachment_id: String::new(),
            filename: "descripcion-proyecto.txt".into(),
            mime_type: "text/plain".into(),
            size: payload.len() as i64,
            inline_data: Some(standard_b64),
        }];

        let fake = FakeEmailProvider::new("me@outlook.example", "Me");
        let saved = process_attachments_for_email(
            &db,
            &fake,
            &email,
            &infos,
            std::slice::from_ref(&rule),
            tmp.path(),
            None,
        )
        .await
        .expect("process_attachments_for_email must not error on standard base64");
        assert_eq!(saved, 1, "Outlook standard-alphabet inline payload must be persisted");

        let stored = db.get_attachments_for_rule(&rule.id).expect("query");
        assert_eq!(stored.len(), 1);
        let on_disk = std::fs::read(tmp.path().join(&stored[0].file_path)).expect("read on-disk attachment");
        assert_eq!(on_disk, payload, "decoded bytes must match original payload");
    }

    /// Regression: an email synced WITHOUT its attachment (0 meta rows, despite
    /// the message having one) is repaired by re-fetching + re-extracting — the
    /// row count goes 0 → 1, and re-running is idempotent (no duplicate).
    #[tokio::test]
    async fn reextract_with_provider_backfills_missing_attachment() {
        use crate::sync::provider::{EmailCategory, FakeEmailProvider};

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let account_id = "acc-gmail";
        make_account(&db, account_id, "gmail", "me@gmail.example");
        let email = make_email(account_id, "msg-missing-att", "sender@example.com", "RE: docs");
        db.insert_email(&email).expect("insert email");

        // Precondition: the bug state — email present, zero attachment rows.
        assert_eq!(db.get_email_attachment_metas(&email.id).expect("metas").len(), 0);

        // On re-fetch the provider DOES report the attachment.
        let fake = FakeEmailProvider::new("me@gmail.example", "Me");
        fake.add_message(
            email.clone(),
            EmailCategory::Primary,
            vec![AttachmentInfo {
                attachment_id: "att-123".into(),
                filename: "report.pdf".into(),
                mime_type: "application/pdf".into(),
                size: 103_823,
                inline_data: None,
            }],
        );

        let found = reextract_with_provider(&db, &fake, account_id, &email.id)
            .await
            .expect("reextract");
        assert_eq!(found.len(), 1);

        let metas = db.get_email_attachment_metas(&email.id).expect("metas after");
        assert_eq!(metas.len(), 1, "the missing attachment must be backfilled");
        assert_eq!(metas[0].filename, "report.pdf");
        assert_eq!(metas[0].provider_attachment_id, "att-123");

        // Idempotent — ON CONFLICT(email_id, filename) DO NOTHING.
        reextract_with_provider(&db, &fake, account_id, &email.id)
            .await
            .expect("reextract again");
        assert_eq!(
            db.get_email_attachment_metas(&email.id).expect("metas").len(),
            1,
            "re-running must not duplicate rows"
        );
    }

    /// When the provider also reports zero attachments, re-extract is a no-op
    /// (the gap would be in the parser, not a missed fetch) — and never errors.
    #[tokio::test]
    async fn reextract_with_provider_noop_when_provider_has_none() {
        use crate::sync::provider::{EmailCategory, FakeEmailProvider};

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let account_id = "acc-gmail";
        make_account(&db, account_id, "gmail", "me@gmail.example");
        let email = make_email(account_id, "msg-no-att", "sender@example.com", "no attachments");
        db.insert_email(&email).expect("insert email");

        let fake = FakeEmailProvider::new("me@gmail.example", "Me");
        fake.add_message(email.clone(), EmailCategory::Primary, vec![]);

        let found = reextract_with_provider(&db, &fake, account_id, &email.id)
            .await
            .expect("reextract");
        assert!(found.is_empty());
        assert_eq!(db.get_email_attachment_metas(&email.id).expect("metas").len(), 0);
    }

    // ── Retroactive rule application — regression tests ─────────────────────
    //
    // Reproduces the bug where clicking "Sync now" on an attachment rule
    // failed for non-Gmail accounts (the function used to hard-code a
    // `match { "gmail" => ..., other => Err("Unsupported provider") }`
    // dispatch and a stale OAuth refresh path that diverged from the rest
    // of the app). The fix routes provider construction through
    // `services::emails::build_provider`, which supports gmail/outlook/imap.

    fn inline_attachment(filename: &str, mime: &str, content: &[u8]) -> AttachmentInfo {
        use base64::Engine;
        AttachmentInfo {
            attachment_id: String::new(),
            filename: filename.into(),
            mime_type: mime.into(),
            size: content.len() as i64,
            // Gmail's inline data uses URL-safe base64; our decoder accepts both.
            inline_data: Some(base64::engine::general_purpose::URL_SAFE.encode(content)),
        }
    }

    /// Happy path: `apply_rule_with_provider` walks the account's emails,
    /// fetches each via the (fake) provider, and persists matching
    /// attachments. This is the test seam introduced by the fix.
    #[tokio::test]
    async fn apply_rule_with_provider_persists_matching_attachments() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");

        let account_id = "acc1";
        make_account(&db, account_id, "gmail", "me@example.com");
        let email = make_email(account_id, "msg-1", "invoices@aws.com", "Your invoice");
        db.insert_email(&email).expect("insert email");

        let rule = create_rule(
            &db,
            account_id,
            "AWS invoices",
            Some("invoices@aws.com"),
            None,
            Some("*.pdf"),
            vec!["aws".into(), "invoices".into()],
        )
        .expect("create rule");

        let fake = FakeEmailProvider::new("me@example.com", "Me");
        fake.add_message(
            email.clone(),
            EmailCategory::Primary,
            vec![inline_attachment(
                "Invoice-2024-03.pdf",
                "application/pdf",
                b"%PDF-1.4 fake",
            )],
        );

        let saved = apply_rule_with_provider(&db, &rule, account_id, &fake, tmp.path(), None)
            .await
            .expect("apply_rule_with_provider should succeed");

        assert_eq!(saved, 1, "exactly one attachment should be persisted");
        let stored = db.get_attachments_for_rule(&rule.id).expect("query attachments");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].filename, "Invoice-2024-03.pdf");
        assert_eq!(stored[0].tags, vec!["aws".to_string(), "invoices".to_string()]);
    }

    /// Regression: clicking "Sync now" on a rule no longer matches anything
    /// returns Ok(0), not an error. Previously a missing rule or account
    /// produced a NotFound; the happy path with no matching emails should
    /// be a quiet success regardless of provider type.
    #[tokio::test]
    async fn apply_rule_with_provider_returns_zero_when_no_emails_match() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");

        let account_id = "acc-outlook";
        make_account(&db, account_id, "outlook", "me@outlook.example");
        // Insert a single email that will NOT match the rule's sender pattern.
        let email = make_email(account_id, "msg-1", "newsletter@other.com", "Hello");
        db.insert_email(&email).expect("insert email");

        let rule = create_rule(
            &db,
            account_id,
            "AWS invoices",
            Some("invoices@aws.com"),
            None,
            None,
            vec![],
        )
        .expect("create rule");

        let fake = FakeEmailProvider::new("me@outlook.example", "Me");

        let saved = apply_rule_with_provider(&db, &rule, account_id, &fake, tmp.path(), None)
            .await
            .expect("apply_rule_with_provider must not error when nothing matches");

        assert_eq!(saved, 0);
        assert_eq!(
            db.get_attachments_for_rule(&rule.id).expect("query attachments").len(),
            0
        );
    }

    // --- save_bytes_to_downloads ---

    #[test]
    fn save_bytes_writes_the_file_into_the_downloads_dir() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let path = save_bytes_to_downloads(tmp.path(), "report.pdf", b"pdf-bytes").expect("save");
        assert_eq!(path, tmp.path().join("report.pdf"));
        assert_eq!(std::fs::read(&path).expect("read back"), b"pdf-bytes");
    }

    #[test]
    fn save_bytes_dedupes_colliding_filenames_with_numeric_suffix() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        std::fs::write(tmp.path().join("report.pdf"), b"old").expect("seed");
        std::fs::write(tmp.path().join("report (1).pdf"), b"old").expect("seed");
        let path = save_bytes_to_downloads(tmp.path(), "report.pdf", b"new").expect("save");
        assert_eq!(path, tmp.path().join("report (2).pdf"));
        // The existing files are untouched.
        assert_eq!(std::fs::read(tmp.path().join("report.pdf")).expect("read"), b"old");
    }

    #[test]
    fn save_bytes_strips_path_components_from_the_filename() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let path = save_bytes_to_downloads(tmp.path(), "../../evil.sh", b"x").expect("save");
        assert_eq!(path, tmp.path().join("evil.sh"));
        let path = save_bytes_to_downloads(tmp.path(), "nested/dir/file.txt", b"x").expect("save");
        assert_eq!(path, tmp.path().join("file.txt"));
    }

    #[test]
    fn save_bytes_falls_back_to_a_generic_name_when_the_filename_is_unusable() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let path = save_bytes_to_downloads(tmp.path(), "", b"x").expect("save");
        assert_eq!(path, tmp.path().join("attachment"));
        let path = save_bytes_to_downloads(tmp.path(), "../..", b"x").expect("save");
        assert_eq!(path, tmp.path().join("attachment (1)"));
    }

    // --- validate_reveal_path ---

    #[test]
    fn validate_reveal_path_accepts_files_inside_the_downloads_dir() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let file = tmp.path().join("saved.pdf");
        std::fs::write(&file, b"x").expect("seed");
        let ok = validate_reveal_path(tmp.path(), &file).expect("must accept");
        assert!(ok.ends_with("saved.pdf"));
    }

    #[test]
    fn validate_reveal_path_accepts_the_downloads_dir_itself() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        validate_reveal_path(tmp.path(), tmp.path()).expect("must accept the dir itself");
    }

    #[test]
    fn validate_reveal_path_rejects_paths_outside_the_downloads_dir() {
        let downloads = tempfile::tempdir().expect("tmp dir");
        let elsewhere = tempfile::tempdir().expect("tmp dir");
        let file = elsewhere.path().join("secret.txt");
        std::fs::write(&file, b"x").expect("seed");
        let err = validate_reveal_path(downloads.path(), &file).expect_err("must reject");
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }

    #[test]
    fn validate_reveal_path_rejects_missing_files() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let err = validate_reveal_path(tmp.path(), &tmp.path().join("gone.pdf")).expect_err("must reject");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }
}
