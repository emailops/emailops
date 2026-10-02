use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::services::app_handle::AppHandle;
use regex::Regex;
#[cfg(feature = "desktop")]
use tauri::Emitter;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Attachment, AttachmentRule};
use crate::services::attachment_safety::{self, quarantine_saved_file};
use crate::services::emails::build_provider;
use crate::sync::provider::{AttachmentInfo, EmailProvider};

/// Emit an `app-log` event when an `AppHandle` is available.
///
/// `apply_rule_retroactively` and `process_attachments_for_email` accept
/// `Option<&AppHandle>` so they remain unit-testable without a Tauri runtime
/// — in those tests `app` is `None` and the events are silently dropped.
/// Report to the output panel through the process logger (the app's
/// `app-log` events on desktop, stderr in the CLI, a `VecLogger` in tests).
fn emit_log(level: &str, source: &str, message: impl Into<String>) {
    crate::services::logger::log(level, source, message);
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

/// Derive a filesystem-safe extension from a (possibly attacker-controlled)
/// attachment filename, for embedding into a generated `<uuid>.<ext>` path.
///
/// The naive `filename.rsplit('.').next()` this replaced returns the whole
/// filename verbatim whenever it contains no `.` (or the tail after the last
/// `.` when it does), slashes included — a crafted attachment name like
/// `malicious/nested/injected-payload` (no dot) injected arbitrary nested
/// directory structure into the write path instead of the intended flat
/// `attachments/<account_id>/<file>` layout. Sanitizing through
/// `sanitize_download_filename` first strips any path separators, so the
/// result can never contain `/` or `\`.
fn safe_extension(filename: &str) -> String {
    let safe_name = sanitize_download_filename(filename);
    Path::new(&safe_name)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "bin".to_string())
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

/// Destination filename for a bulk download: `<RuleName>_<original>`, or just
/// the original when the rule has no name. Both parts are reduced to a single
/// path component so the result always stays inside the Downloads folder.
pub fn bulk_download_name(rule_name: Option<&str>, filename: &str) -> String {
    let filename = sanitize_download_filename(filename);
    match rule_name
        .map(|n| n.replace([' ', '/', '\\'], "_"))
        .filter(|n| !n.is_empty())
    {
        Some(prefix) => format!("{prefix}_{filename}"),
        None => filename,
    }
}

/// Write attachment bytes into `dir` (the user's Downloads folder) under a
/// sanitized, collision-free name. Returns the path actually written.
pub fn save_bytes_to_downloads(dir: &Path, filename: &str, bytes: &[u8]) -> Result<PathBuf> {
    let safe_name = sanitize_download_filename(filename);
    let dest = unique_download_path(dir, &safe_name);
    std::fs::write(&dest, bytes)
        .map_err(|e| AppError::IoError(format!("Failed to save {} to Downloads: {e}", dest.display())))?;
    quarantine_saved_file(&dest);
    Ok(dest)
}

/// Copy a stored attachment into `dir` (the user's Downloads folder) under a
/// collision-free variant of `download_name`, quarantined like every other
/// attachment file (a copy of a file stored before attachments were marked
/// carries no mark of its own). Returns the path written.
pub fn copy_attachment_to_downloads(src: &Path, dir: &Path, download_name: &str) -> Result<PathBuf> {
    let dest = unique_download_path(dir, download_name);
    std::fs::copy(src, &dest).map_err(|e| AppError::IoError(format!("Failed to copy {download_name}: {e}")))?;
    quarantine_saved_file(&dest);
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

/// How to reveal a path in the host file manager.
///
/// Split out as a pure decision so each platform's argument shape can be
/// table-tested from any platform — the Windows and Linux arms are never
/// executed during macOS development.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevealAction {
    /// Launch a file manager that highlights the file inside its folder.
    Select { program: String, args: Vec<String> },
    /// No file-selecting helper is reliably available; open the folder instead.
    OpenDirectory(std::path::PathBuf),
}

/// Decide how to reveal `path` on `os`.
///
/// `is_dir` is passed in rather than probed so the decision stays pure.
///
/// A directory is opened — except one whose name is a type the OS launches
/// (an app bundle, a `.workflow`): "opening" that would run it, so it is
/// selected in its folder like a file.
pub fn plan_reveal(os: &str, path: &Path, is_dir: bool) -> RevealAction {
    if is_dir && attachment_safety::classify(&path.to_string_lossy(), None).is_none() {
        return RevealAction::OpenDirectory(path.to_path_buf());
    }

    match os {
        "macos" => RevealAction::Select {
            program: "open".into(),
            args: vec!["-R".into(), path.to_string_lossy().into_owned()],
        },
        // Explorer wants the path glued to the switch as a single argument:
        // `explorer /select,C:\dir\file.txt`. It also exits non-zero on
        // success, which is why the executor spawns rather than waits.
        //
        // Named by absolute path: `CreateProcessW` searches the application and
        // current directory before PATH, so a bare "explorer" could resolve to a
        // planted `explorer.exe`. Explorer lives directly under %SystemRoot%,
        // not System32.
        "windows" => RevealAction::Select {
            program: format!(
                "{}\\explorer.exe",
                std::env::var("SystemRoot")
                    .unwrap_or_else(|_| "C:\\Windows".to_string())
                    .trim_end_matches('\\')
            ),
            args: vec![format!("/select,{}", path.to_string_lossy())],
        },
        // No file manager is guaranteed across desktop environments, and the
        // org.freedesktop.FileManager1 D-Bus interface is widely unimplemented,
        // so opening the containing folder is the dependable behaviour.
        _ => RevealAction::OpenDirectory(containing_dir(path)),
    }
}

/// Directory holding `path`, falling back to `path` itself when it has none.
///
/// `Path::parent` yields `Some("")` — not `None` — for a bare filename, and an
/// empty path is not something any file manager can open.
fn containing_dir(path: &Path) -> std::path::PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => path.to_path_buf(),
    }
}

/// Reveal an already-validated path in the OS file manager.
///
/// A file is highlighted where the platform supports it (Finder on macOS,
/// Explorer on Windows); otherwise the containing folder is opened. If the
/// selecting helper cannot be launched, this degrades to opening the folder
/// rather than failing — but the reason is logged, never swallowed.
pub fn reveal_in_file_manager(path: &Path) -> Result<()> {
    let action = plan_reveal(std::env::consts::OS, path, path.is_dir());

    let fallback_dir = match action {
        RevealAction::Select { program, args } => match std::process::Command::new(&program).args(&args).spawn() {
            Ok(_) => return Ok(()),
            Err(e) => {
                crate::services::logger::log(
                    "debug",
                    "system",
                    format!("Could not launch '{program}' to select the file ({e}); opening the folder instead"),
                );
                containing_dir(path)
            }
        },
        RevealAction::OpenDirectory(dir) => dir,
    };

    open::that(&fallback_dir)
        .map_err(|e| AppError::IoError(format!("Failed to open folder '{}': {e}", fallback_dir.display())))?;
    Ok(())
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

pub(crate) fn matches_glob(pattern: &str, value: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        let regex_str = glob_to_regex(pattern);
        Regex::new(&regex_str).map(|re| re.is_match(value)).unwrap_or(false)
    } else {
        value.eq_ignore_ascii_case(pattern)
    }
}

/// What a path that stores mail needs to run attachment rules on it: the
/// sync's Sent / folder passes, the attachment backfill, a manual mailbox
/// resync. `None` where a caller runs no rules.
pub struct RuleSyncCtx<'a> {
    pub rules: &'a [AttachmentRule],
    pub app_data_dir: &'a Path,
    pub app: Option<&'a AppHandle>,
}

/// Run the rules of `ctx` on one stored email's attachments, if its mailbox
/// is one rules reach. Failures are logged against the account and the
/// caller carries on — one email never stops a sync.
pub async fn apply_rules_to_stored_email(
    db: &Database,
    provider: Option<&dyn EmailProvider>,
    email: &crate::models::Email,
    infos: &[AttachmentInfo],
    ctx: &RuleSyncCtx<'_>,
    account_email: &str,
) {
    if ctx.rules.is_empty() || infos.is_empty() || !rules_apply_to_mailbox(&email.mailbox) {
        return;
    }
    if let Err(e) =
        process_attachments_for_email(db, provider, email, infos, ctx.rules, ctx.app_data_dir, ctx.app).await
    {
        crate::services::emails::emit_account_log(
            "error",
            "attachments",
            account_email,
            &format!("Attachment rule processing error: {e}"),
        );
    }
}

/// Whether attachment rules collect mail filed in `mailbox`: the inbox,
/// Sent, the Archive and the user's own folders, never Spam or Trash — a sender rule must
/// not pick up the copy of a message the user junked or deleted.
pub fn rules_apply_to_mailbox(mailbox: &str) -> bool {
    matches!(mailbox, "inbox" | "sent" | "archive") || mailbox.starts_with("folder:")
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

// --- Retroactive application planning ---

/// Where the attachments of one email come from when a rule is applied to
/// mail already stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetroSource {
    /// The recorded `email_attachment_meta` rows: only matching files are
    /// read from disk or fetched, one attachment at a time.
    RecordedAttachments,
    /// The whole message from the provider — only when the record may be
    /// incomplete (an account the attachment backfill has not covered yet).
    FetchMessage,
}

/// Pure planner: which emails a retroactive rule application has to visit.
///
/// `rows` are `(email_id, sender_email, subject)`; `recorded` maps an email
/// to the filenames recorded for it. With `record_complete`, an email with no
/// recorded attachment has none, so it is skipped instead of fetched — a
/// sender like `noreply@github.com` sends thousands of notifications for a
/// handful of receipts, and fetching each one made the scan take minutes.
pub fn plan_retroactive_apply(
    rule: &AttachmentRule,
    rows: &[(String, String, String)],
    recorded: &std::collections::HashMap<String, Vec<String>>,
    record_complete: bool,
) -> Vec<(String, RetroSource)> {
    rows.iter()
        .filter(|(_, sender, subject)| matches_rule(rule, sender, subject))
        .filter_map(|(id, _, _)| match recorded.get(id) {
            Some(files) if files.iter().any(|f| matches_filename(rule, f)) => {
                Some((id.clone(), RetroSource::RecordedAttachments))
            }
            Some(_) => None,
            None if record_complete => None,
            None => Some((id.clone(), RetroSource::FetchMessage)),
        })
        .collect()
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
    db: &Database,
    provider: Option<&dyn EmailProvider>,
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
                            "error",
                            "attachments",
                            format!("Failed to decode inline attachment '{}': {}", info.filename, e),
                        );
                        continue;
                    }
                }
            } else {
                let fetched = match provider {
                    Some(provider) => provider.fetch_attachment_bytes(&email.id, &info.attachment_id).await,
                    None => Err(AppError::SyncError("no provider available to download it".into())),
                };
                match fetched {
                    Ok(b) => b,
                    Err(e) => {
                        emit_log(
                            "error",
                            "attachments",
                            format!("Failed to download attachment '{}': {}", info.filename, e),
                        );
                        continue;
                    }
                }
            };

            if store_collected_attachment(db, email, info, rule, &bytes, app_data_dir).await? {
                count += 1;
            }
        }
    }

    // Notify the frontend so the attachments list refreshes without an app
    // restart. Sync and retroactive-scan paths both insert via this function;
    // a single event per email (only when something was saved) is enough.
    if count > 0 {
        if let Some(app) = app {
            if let Err(e) = app.emit("attachments-updated", &email.account_id) {
                eprintln!("[attachments] could not emit attachments-updated: {e}");
            }
        }
    }

    Ok(count)
}

/// Write one collected attachment to disk and record it. The row is only
/// inserted while `rule` is still the version the caller planned with (not
/// edited or deleted since), the email still exists and no concurrent run
/// stored the same file — otherwise the file just written is removed again.
/// Returns whether the attachment was stored.
pub(crate) async fn store_collected_attachment(
    db: &Database,
    email: &crate::models::Email,
    info: &AttachmentInfo,
    rule: &AttachmentRule,
    bytes: &[u8],
    app_data_dir: &Path,
) -> Result<bool> {
    let ext = safe_extension(&info.filename);
    let file_id = uuid::Uuid::new_v4().to_string();
    let relative_path = format!("attachments/{}/{}.{}", email.account_id, file_id, ext);
    let absolute_path = app_data_dir.join(&relative_path);

    if let Some(parent) = absolute_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::IoError(format!("Failed to create attachment directory: {}", e)))?;
    }
    tokio::fs::write(&absolute_path, bytes)
        .await
        .map_err(|e| AppError::IoError(format!("Failed to write attachment file: {}", e)))?;
    quarantine_saved_file(&absolute_path);

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

    let inserted = match db.insert_attachment_for_rule_version(&attachment, rule.updated_at) {
        Ok(inserted) => inserted,
        Err(e) => {
            remove_attachment_file(&absolute_path);
            return Err(e);
        }
    };
    if !inserted {
        remove_attachment_file(&absolute_path);
    }
    Ok(inserted)
}

/// Remove a stored attachment file, logging (not failing on) an error: the
/// row it belonged to is already gone.
fn remove_attachment_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[attachments] could not remove {}: {e}", path.display()),
    }
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
) -> Result<u32> {
    let mut count = 0u32;

    for info in attachment_infos {
        // Get binary data
        let bytes = if let Some(ref inline_b64) = info.inline_data {
            match decode_inline_base64(inline_b64) {
                Ok(b) => b,
                Err(e) => {
                    emit_log(
                        "error",
                        "attachments",
                        format!("Auto-download: failed to decode '{}': {}", info.filename, e),
                    );
                    continue;
                }
            }
        } else if !info.attachment_id.is_empty() {
            match provider.fetch_attachment_bytes(&email.id, &info.attachment_id).await {
                Ok(b) => b,
                Err(e) => {
                    emit_log(
                        "error",
                        "attachments",
                        format!("Auto-download: failed to fetch '{}': {}", info.filename, e),
                    );
                    continue;
                }
            }
        } else {
            continue; // No data source available
        };

        let ext = safe_extension(&info.filename);
        let file_id = uuid::Uuid::new_v4().to_string();
        let relative_path = format!("attachments/{}/auto/{}.{}", email.account_id, file_id, ext);
        let absolute_path = app_data_dir.join(&relative_path);

        if let Some(parent) = absolute_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                emit_log("error", "attachments", format!("Auto-download: mkdir failed: {}", e));
                continue;
            }
        }
        if let Err(e) = tokio::fs::write(&absolute_path, &bytes).await {
            emit_log(
                "error",
                "attachments",
                format!("Auto-download: write failed for '{}': {}", info.filename, e),
            );
            continue;
        }
        quarantine_saved_file(&absolute_path);

        if let Err(e) = db.set_email_attachment_file_path(&email.id, &info.filename, &relative_path) {
            // The file is on disk but the row does not point at it: the UI
            // would offer to download it again.
            emit_log(
                "error",
                "attachments",
                format!("Auto-download: could not record '{}': {e}", info.filename),
            );
            continue;
        }
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
            remove_attachment_file(&app_data_dir.join(&path));
        }
    }

    // Update tags on remaining attachments
    db.update_attachments_tags_for_rule(&rule.id, &rule.tags)?;

    Ok(rule)
}

pub fn delete_rule(db: &Arc<Database>, rule_id: &str, account_id: &str, app_data_dir: &Path) -> Result<()> {
    // Rows and rule go in one transaction, so an apply storing a file at the
    // same time either lands before (its file is listed here) or finds the
    // rule gone and removes the file itself.
    for relative_path in db.delete_attachment_rule_with_attachments(rule_id, account_id)? {
        remove_attachment_file(&app_data_dir.join(relative_path));
    }
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

/// The retroactive applies running right now, one per rule: starting another
/// apply of a rule, editing it or deleting it cancels the one in flight, so
/// two scans never race and none keeps collecting for an outdated pattern.
#[derive(Default)]
pub struct RuleApplies {
    running: std::sync::Mutex<std::collections::HashMap<String, (u64, Arc<std::sync::atomic::AtomicBool>)>>,
    next_generation: std::sync::atomic::AtomicU64,
}

/// A registered apply. Dropping it unregisters the apply unless a newer one
/// for the same rule has replaced it. It owns its registry so it can travel
/// into the background task that runs the apply.
pub struct RuleApplyTicket {
    applies: Arc<RuleApplies>,
    rule_id: String,
    generation: u64,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl RuleApplies {
    /// Register a new apply of `rule_id`, cancelling the one already running.
    pub fn begin(self: &Arc<Self>, rule_id: &str) -> RuleApplyTicket {
        use std::sync::atomic::Ordering;
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let previous = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(rule_id.to_string(), (generation, Arc::clone(&cancelled)));
        if let Some((_, flag)) = previous {
            flag.store(true, Ordering::Relaxed);
        }
        RuleApplyTicket {
            applies: Arc::clone(self),
            rule_id: rule_id.to_string(),
            generation,
            cancelled,
        }
    }

    /// Cancel the apply of `rule_id`, if one is running.
    pub fn cancel(&self, rule_id: &str) {
        if let Some((_, flag)) = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(rule_id)
        {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl RuleApplyTicket {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for RuleApplyTicket {
    fn drop(&mut self) {
        let mut running = self
            .applies
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if running.get(&self.rule_id).is_some_and(|(g, _)| *g == self.generation) {
            running.remove(&self.rule_id);
        }
    }
}

/// Attachments of `email_id` that are already stored on disk (auto-download
/// or an on-demand save), read back as inline data so a rule can collect them
/// without asking the provider. `None` when the email has no stored
/// attachment metadata or any of it has no local file: the provider has to be
/// asked then.
async fn stored_attachment_infos(
    db: &Database,
    email_id: &str,
    app_data_dir: &Path,
) -> Result<Option<Vec<AttachmentInfo>>> {
    use base64::Engine;

    let metas = db.get_email_attachment_metas(email_id)?;
    if metas.is_empty() {
        return Ok(None);
    }
    let mut infos = Vec::with_capacity(metas.len());
    for meta in metas {
        let Some(relative_path) = meta.file_path.as_deref() else {
            return Ok(None);
        };
        let path = match safe_attachment_path(app_data_dir, relative_path) {
            Ok(path) => path,
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| AppError::IoError(format!("Failed to read stored attachment '{}': {e}", meta.filename)))?;
        infos.push(AttachmentInfo {
            attachment_id: meta.provider_attachment_id,
            filename: meta.filename,
            mime_type: meta.mime_type,
            size: bytes.len() as i64,
            inline_data: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
        });
    }
    Ok(Some(infos))
}

/// Whether every attachment of `email_id` has a local file — the cheap
/// question [`stored_attachment_infos`] answers by reading them all.
fn attachments_stored_on_disk(db: &Database, email_id: &str, app_data_dir: &Path) -> Result<bool> {
    let metas = db.get_email_attachment_metas(email_id)?;
    if metas.is_empty() {
        return Ok(false);
    }
    for meta in metas {
        let Some(relative_path) = meta.file_path.as_deref() else {
            return Ok(false);
        };
        match safe_attachment_path(app_data_dir, relative_path) {
            Ok(_) => {}
            Err(AppError::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

pub async fn apply_rule_retroactively(
    db: &Arc<Database>,
    rule_id: &str,
    account_id: &str,
    app_data_dir: &Path,
    app: Option<&AppHandle>,
    on_progress: &(dyn Fn(RetroProgress) + Send + Sync),
    should_abort: &(dyn Fn() -> bool + Send + Sync),
) -> Result<u32> {
    let rule = db
        .get_attachment_rule(rule_id)?
        .ok_or_else(|| AppError::NotFound(format!("Rule {} not found", rule_id)))?;

    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    // Attachments already stored on disk need no provider, so it is only built
    // (and, for OAuth, its token refreshed) when some matching email is
    // missing locally. An account without usable credentials, or offline,
    // still collects what it already has. `build_provider` dispatches on
    // provider and handles OAuth refresh consistently with the rest of the app.
    let plan = plan_for(db, &rule, account_id)?;
    let provider = if rule_needs_provider(db, &rule, &plan.emails, app_data_dir)? {
        Some(build_provider(&account, app.cloned()).await?)
    } else {
        None
    };

    apply_planned(
        db,
        &rule,
        account_id,
        plan,
        provider.as_deref(),
        app_data_dir,
        app,
        on_progress,
        should_abort,
    )
    .await
}

/// How a retroactive apply ended, as the `attachment-rule-apply-finished`
/// event reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyStatus {
    Done,
    Failed,
    /// Superseded by a newer apply of the rule, or the rule was edited or
    /// deleted meanwhile — not a failure.
    Cancelled,
}

/// The `AppError` wire shape (`code`, `params`, `message`), owned.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApplyErrorPayload {
    pub code: String,
    pub params: std::collections::BTreeMap<String, String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    pub status: ApplyStatus,
    /// New attachments collected (0 unless done).
    pub saved: u32,
    pub error: Option<ApplyErrorPayload>,
}

impl ApplyOutcome {
    pub fn from_result(result: &Result<u32>) -> Self {
        match result {
            Ok(saved) => Self {
                status: ApplyStatus::Done,
                saved: *saved,
                error: None,
            },
            Err(AppError::Cancelled) => Self {
                status: ApplyStatus::Cancelled,
                saved: 0,
                error: None,
            },
            Err(e) => Self {
                status: ApplyStatus::Failed,
                saved: 0,
                error: Some(ApplyErrorPayload {
                    code: e.code().to_string(),
                    params: e.params().into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
                    message: e.to_string(),
                }),
            },
        }
    }
}

/// Event with the progress of a queued apply (`ruleId`, `runId`, `processed`,
/// `total`, `saved`).
pub const APPLY_PROGRESS_EVENT: &str = "attachment-rule-apply-progress";
/// Event with how a queued apply ended (`ruleId`, `runId`, `status`, `saved`,
/// `error`).
pub const APPLY_FINISHED_EVENT: &str = "attachment-rule-apply-finished";

/// Run one queued apply of `rule_id` and report it through `emit`: progress
/// as it goes, then exactly one finished event. `run_id` is the caller's tag
/// for this run, echoed in every event; `ticket` cancels the run when a newer
/// apply, an edit or a delete of the rule supersedes it.
#[allow(clippy::too_many_arguments)]
pub async fn run_rule_apply(
    db: &Arc<Database>,
    rule_id: &str,
    account_id: &str,
    run_id: &str,
    app_data_dir: &Path,
    app: Option<&AppHandle>,
    ticket: &RuleApplyTicket,
    emit: &(dyn Fn(&'static str, serde_json::Value) + Send + Sync),
) -> ApplyOutcome {
    let on_progress = |p: RetroProgress| {
        emit(
            APPLY_PROGRESS_EVENT,
            serde_json::json!({
                "ruleId": rule_id,
                "runId": run_id,
                "processed": p.processed,
                "total": p.total,
                "saved": p.saved,
            }),
        );
    };
    let result = apply_rule_retroactively(db, rule_id, account_id, app_data_dir, app, &on_progress, &|| {
        ticket.is_cancelled()
    })
    .await;
    let outcome = ApplyOutcome::from_result(&result);
    emit(
        APPLY_FINISHED_EVENT,
        serde_json::json!({
            "ruleId": rule_id,
            "runId": run_id,
            "status": outcome.status,
            "saved": outcome.saved,
            "error": outcome.error,
        }),
    );
    outcome
}

/// Progress of a retroactive rule application: `processed` of the `total`
/// planned emails visited, `saved` attachments collected so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetroProgress {
    pub processed: usize,
    pub total: usize,
    pub saved: u32,
}

/// Whether the account's attachment record can be trusted to be complete:
/// true once the one-time attachment backfill has covered it.
fn attachment_record_complete(db: &Database, account_id: &str) -> Result<bool> {
    Ok(db
        .get_preference(&crate::services::emails::backfill_done_key(account_id))?
        .is_some())
}

/// The emails a retroactive apply visits, and how many it looked through.
struct RetroPlan {
    scanned: usize,
    emails: Vec<(String, RetroSource)>,
}

fn plan_for(db: &Database, rule: &AttachmentRule, account_id: &str) -> Result<RetroPlan> {
    let rows = db.get_emails_matching_rule(account_id)?;
    let recorded = db.get_attachment_filenames_by_email(account_id)?;
    let complete = attachment_record_complete(db, account_id)?;
    Ok(RetroPlan {
        scanned: rows.len(),
        emails: plan_retroactive_apply(rule, &rows, &recorded, complete),
    })
}

/// Whether applying `rule` to the account's existing emails has to ask the
/// provider: a whole message must be fetched, or a matching recorded
/// attachment is neither on disk nor carried inline.
fn rule_needs_provider(
    db: &Database,
    rule: &AttachmentRule,
    plan: &[(String, RetroSource)],
    app_data_dir: &Path,
) -> Result<bool> {
    for (email_id, source) in plan {
        match source {
            RetroSource::FetchMessage => return Ok(true),
            RetroSource::RecordedAttachments => {
                if attachments_stored_on_disk(db, email_id, app_data_dir)? {
                    continue;
                }
                let missing_bytes = db
                    .get_attachment_infos(email_id)?
                    .iter()
                    .any(|i| matches_filename(rule, &i.filename) && i.inline_data.is_none());
                if missing_bytes {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Apply a rule retroactively using a caller-supplied provider. Extracted from
/// `apply_rule_retroactively` so unit tests can drive the loop with
/// `FakeEmailProvider` without a Tauri runtime or live OAuth tokens.
/// `provider` is `None` when nothing has to be fetched; an email that still
/// needs a fetch is then skipped with a warning. `on_progress` is called once
/// before the first email and after each one. `should_abort` is checked
/// before each email; once it returns true the apply ends with
/// [`AppError::Cancelled`]. An email that fails to store is logged and
/// skipped, so one bad message never ends a scan of thousands.
#[allow(clippy::too_many_arguments)]
pub async fn apply_rule_with_provider(
    db: &Arc<Database>,
    rule: &AttachmentRule,
    account_id: &str,
    provider: Option<&dyn EmailProvider>,
    app_data_dir: &Path,
    app: Option<&AppHandle>,
    on_progress: &(dyn Fn(RetroProgress) + Send + Sync),
    should_abort: &(dyn Fn() -> bool + Send + Sync),
) -> Result<u32> {
    let plan = plan_for(db, rule, account_id)?;
    apply_planned(
        db,
        rule,
        account_id,
        plan,
        provider,
        app_data_dir,
        app,
        on_progress,
        should_abort,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn apply_planned(
    db: &Arc<Database>,
    rule: &AttachmentRule,
    account_id: &str,
    RetroPlan { scanned, emails: plan }: RetroPlan,
    provider: Option<&dyn EmailProvider>,
    app_data_dir: &Path,
    app: Option<&AppHandle>,
    on_progress: &(dyn Fn(RetroProgress) + Send + Sync),
    should_abort: &(dyn Fn() -> bool + Send + Sync),
) -> Result<u32> {
    let total = plan.len();
    on_progress(RetroProgress {
        processed: 0,
        total,
        saved: 0,
    });
    emit_log(
        "info",
        "attachments",
        format!("Rule '{}': checking {total} of {scanned} emails...", rule.name),
    );

    let mut total_attachments = 0u32;
    let mut emails_with_filename_match = 0u32;
    let mut failed_emails = 0u32;
    let rules = std::slice::from_ref(rule);

    for (index, (email_id, source)) in plan.iter().enumerate() {
        if should_abort() {
            emit_log(
                "info",
                "attachments",
                format!(
                    "Rule '{}': scan stopped after {index} of {total} emails ({total_attachments} attachments saved).",
                    rule.name
                ),
            );
            return Err(AppError::Cancelled);
        }
        // Attachments already on disk are collected from there; otherwise
        // recorded attachments are fetched one by one, and only a message
        // with no trustworthy record is fetched whole.
        let loaded = match source {
            RetroSource::RecordedAttachments => match db.get_email(email_id)? {
                Some(email) => {
                    let infos = match stored_attachment_infos(db, email_id, app_data_dir).await? {
                        Some(infos) => infos,
                        None => db.get_attachment_infos(email_id)?,
                    };
                    Some((email, infos))
                }
                None => None,
            },
            RetroSource::FetchMessage => match provider {
                None => {
                    emit_log(
                        "warn",
                        "attachments",
                        format!("Skipping email {email_id}: its attachments are not stored locally"),
                    );
                    None
                }
                Some(provider) => match provider.get_message(email_id).await {
                    Ok((email, _category, infos)) => Some((email, infos)),
                    Err(e) => {
                        emit_log("warn", "attachments", format!("Skipping email {}: {}", email_id, e));
                        None
                    }
                },
            },
        };

        if let Some((email, attachment_infos)) = loaded.filter(|(_, infos)| !infos.is_empty()) {
            if attachment_infos
                .iter()
                .any(|info| matches_filename(rule, &info.filename))
            {
                emails_with_filename_match += 1;
            }
            let mut email_with_account = email;
            email_with_account.account_id = account_id.to_string();
            match process_attachments_for_email(
                db,
                provider,
                &email_with_account,
                &attachment_infos,
                rules,
                app_data_dir,
                app,
            )
            .await
            {
                Ok(saved) => total_attachments += saved,
                Err(e) => {
                    failed_emails += 1;
                    emit_log(
                        "error",
                        "attachments",
                        format!(
                            "Rule '{}': could not store the attachments of {email_id}: {e}",
                            rule.name
                        ),
                    );
                }
            }
        }

        on_progress(RetroProgress {
            processed: index + 1,
            total,
            saved: total_attachments,
        });
    }

    let mut summary = format!(
        "Rule '{}': {} of {} emails had matching attachments; saved {} new attachments.",
        rule.name, emails_with_filename_match, scanned, total_attachments,
    );
    if failed_emails > 0 {
        summary.push_str(&format!(" {failed_emails} emails could not be stored."));
    }
    let level = match (failed_emails, total_attachments) {
        (0, 0) => "warn",
        (0, _) => "success",
        _ => "error",
    };
    emit_log(level, "attachments", summary);

    // Zero candidates is almost always a too-strict pattern (e.g. `apple.com`
    // when the sender is `no_reply@email.apple.com` and needs `*apple.com*`).
    if total == 0 && scanned > 0 {
        emit_log("warn",
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

    // ── plan_reveal ───────────────────────────────────────────────────────────

    #[test]
    fn macos_selects_the_file_in_finder() {
        let path = Path::new("/Users/x/Downloads/invoice.pdf");
        assert_eq!(
            plan_reveal("macos", path, false),
            RevealAction::Select {
                program: "open".into(),
                args: vec!["-R".into(), "/Users/x/Downloads/invoice.pdf".into()],
            }
        );
    }

    #[test]
    fn windows_selects_the_file_with_the_glued_switch() {
        // Explorer only understands `/select,<path>` as one argument; passing
        // "/select," and the path separately silently opens the wrong thing.
        let path = Path::new(r"C:\Users\x\Downloads\invoice.pdf");
        let action = plan_reveal("windows", path, false);
        match action {
            RevealAction::Select { program, args } => {
                // Absolute path, not a bare name: Windows searches the app and
                // current directory before PATH, so a bare "explorer" is a
                // binary-planting vector.
                assert!(
                    program.to_lowercase().ends_with("\\explorer.exe"),
                    "expected an absolute path to explorer.exe, got {program:?}"
                );
                assert!(
                    program.contains(':'),
                    "expected a drive-qualified absolute path, got {program:?}"
                );
                assert_eq!(args.len(), 1, "the switch and path must be one argument");
                assert_eq!(args[0], r"/select,C:\Users\x\Downloads\invoice.pdf");
            }
            other => panic!("expected Select, got {other:?}"),
        }
    }

    #[test]
    fn linux_opens_the_containing_folder() {
        let path = Path::new("/home/x/Downloads/invoice.pdf");
        assert_eq!(
            plan_reveal("linux", path, false),
            RevealAction::OpenDirectory(Path::new("/home/x/Downloads").to_path_buf())
        );
    }

    #[test]
    fn a_directory_is_opened_directly_on_every_platform() {
        let dir = Path::new("/home/x/Downloads");
        for os in ["macos", "windows", "linux", "freebsd"] {
            assert_eq!(
                plan_reveal(os, dir, true),
                RevealAction::OpenDirectory(dir.to_path_buf()),
                "{os} should open a directory rather than select it"
            );
        }
    }

    #[test]
    fn a_parentless_path_falls_back_to_itself() {
        // Guards against handing an empty path to the file manager.
        let path = Path::new("invoice.pdf");
        assert_eq!(
            plan_reveal("linux", path, false),
            RevealAction::OpenDirectory(path.to_path_buf())
        );
    }

    #[test]
    fn unknown_platforms_still_get_a_usable_action() {
        let path = Path::new("/home/x/Downloads/invoice.pdf");
        assert_eq!(
            plan_reveal("freebsd", path, false),
            RevealAction::OpenDirectory(Path::new("/home/x/Downloads").to_path_buf())
        );
    }

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

    // ── plan_retroactive_apply ────────────────────────────────────────────────

    fn rows(v: &[(&str, &str)]) -> Vec<(String, String, String)> {
        v.iter()
            .map(|(id, sender)| (id.to_string(), sender.to_string(), "subject".to_string()))
            .collect()
    }

    fn names(v: &[(&str, &[&str])]) -> std::collections::HashMap<String, Vec<String>> {
        v.iter()
            .map(|(id, files)| (id.to_string(), files.iter().map(|f| f.to_string()).collect()))
            .collect()
    }

    #[test]
    fn retro_plan_skips_emails_the_rule_does_not_match() {
        let rule = make_rule_with_filename(Some("billing@acme.com"), None, None);
        let plan = plan_retroactive_apply(&rule, &rows(&[("e1", "other@x.com")]), &names(&[]), true);
        assert!(plan.is_empty());
    }

    #[test]
    fn retro_plan_takes_emails_whose_recorded_attachments_match_the_filename() {
        let rule = make_rule_with_filename(Some("noreply@github.com"), None, Some("github-*.pdf"));
        let plan = plan_retroactive_apply(
            &rule,
            &rows(&[
                ("receipt", "noreply@github.com"),
                ("notification", "noreply@github.com"),
            ]),
            &names(&[
                ("receipt", &["github-receipt-1.pdf"]),
                ("notification", &["avatar.png"]),
            ]),
            true,
        );
        assert_eq!(plan, vec![("receipt".to_string(), RetroSource::RecordedAttachments)]);
    }

    #[test]
    fn retro_plan_skips_emails_without_attachments_when_the_record_is_complete() {
        let rule = make_rule_with_filename(Some("noreply@github.com"), None, None);
        let plan = plan_retroactive_apply(&rule, &rows(&[("e1", "noreply@github.com")]), &names(&[]), true);
        assert!(
            plan.is_empty(),
            "a complete attachment record means no attachments, no fetch"
        );
    }

    #[test]
    fn retro_plan_fetches_emails_without_a_record_when_it_may_be_incomplete() {
        let rule = make_rule_with_filename(Some("noreply@github.com"), None, None);
        let plan = plan_retroactive_apply(&rule, &rows(&[("e1", "noreply@github.com")]), &names(&[]), false);
        assert_eq!(plan, vec![("e1".to_string(), RetroSource::FetchMessage)]);
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
            references: None,
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
            is_starred: false,
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
            Some(&fake),
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

    /// Regression: the destination path used to be built from
    /// `info.filename.rsplit('.').next()` directly. A dot-free attacker
    /// filename (a crafted attachment name, applied the moment the email is
    /// synced — no user interaction required) makes that "extension" the
    /// *entire* filename, slashes included, injecting arbitrary nested
    /// directory structure into the write path instead of a flat
    /// `attachments/<account_id>/<file>.<ext>` layout. The stored `file_path`
    /// must always be exactly 3 segments deep, with no attacker-controlled
    /// path separators surviving into it.
    #[tokio::test]
    async fn process_attachments_for_email_confines_hostile_filename_extension() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");

        let account_id = "acc-evil";
        make_account(&db, account_id, "gmail", "me@example.com");
        let email = make_email(account_id, "msg-evil", "attacker@example.com", "Invoice");
        db.insert_email(&email).expect("insert email");

        let rule = create_rule(
            &db,
            account_id,
            "anything from attacker",
            Some("attacker@example.com"),
            None,
            None,
            vec!["self".into()],
        )
        .expect("create rule");

        // No '.' anywhere: `rsplit('.').next()` on a dot-free string returns
        // the string unchanged, so every slash in it used to land straight in
        // the constructed path.
        let infos = vec![AttachmentInfo {
            attachment_id: String::new(),
            filename: "malicious/nested/injected-payload".into(),
            mime_type: "application/octet-stream".into(),
            size: 4,
            inline_data: Some({
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(b"evil")
            }),
        }];

        let fake = FakeEmailProvider::new("me@example.com", "Me");
        process_attachments_for_email(
            &db,
            Some(&fake),
            &email,
            &infos,
            std::slice::from_ref(&rule),
            tmp.path(),
            None,
        )
        .await
        .expect("must not error even on a hostile filename");

        let stored = db.get_attachments_for_rule(&rule.id).expect("query");
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].file_path.matches('/').count(),
            2,
            "file_path must stay exactly `attachments/<account_id>/<file>`, no injected nesting: {}",
            stored[0].file_path
        );
        assert!(
            stored[0].file_path.starts_with(&format!("attachments/{account_id}/")),
            "file_path must live directly under attachments/<account_id>/: {}",
            stored[0].file_path
        );

        let written = tmp.path().join(&stored[0].file_path);
        assert!(
            written.is_file(),
            "attachment must be written to the expected flat path"
        );
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

        let saved = apply_rule_with_provider(&db, &rule, account_id, Some(&fake), tmp.path(), None, &|_| {}, &|| {
            false
        })
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

        let saved = apply_rule_with_provider(&db, &rule, account_id, Some(&fake), tmp.path(), None, &|_| {}, &|| {
            false
        })
        .await
        .expect("apply_rule_with_provider must not error when nothing matches");

        assert_eq!(saved, 0);
        assert_eq!(
            db.get_attachments_for_rule(&rule.id).expect("query attachments").len(),
            0
        );
    }

    /// Record `filename` on `email` as already downloaded to `data_dir`: the
    /// state auto-download (or an on-demand save) leaves behind.
    fn store_local_attachment(db: &Database, data_dir: &Path, email: &Email, filename: &str, content: &[u8]) {
        let relative = format!("attachments/{}/auto/{}", email.account_id, filename);
        let absolute = data_dir.join(&relative);
        std::fs::create_dir_all(absolute.parent().expect("parent dir")).expect("mkdir");
        std::fs::write(&absolute, content).expect("write local file");
        db.insert_email_attachment_metas_batch(&[(
            email.id.clone(),
            email.account_id.clone(),
            String::new(),
            filename.into(),
            "application/pdf".into(),
            content.len() as i64,
            None,
        )])
        .expect("insert meta");
        db.set_email_attachment_file_path(&email.id, filename, &relative)
            .expect("set file path");
    }

    /// An attachment already stored on disk is collected from the local file.
    /// The provider knows nothing about the message here, so the old
    /// fetch-first path skipped the email and saved nothing.
    #[tokio::test]
    async fn apply_rule_with_provider_collects_locally_stored_attachments() {
        use crate::sync::provider::FakeEmailProvider;

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let account_id = "acc-imap";
        make_account(&db, account_id, "imap", "me@example.com");
        let email = make_email(
            account_id,
            "msg-local",
            "billing@vendor.example",
            "Your invoice for March",
        );
        db.insert_email(&email).expect("insert email");
        store_local_attachment(&db, tmp.path(), &email, "invoice-mar.pdf", b"%PDF-1.4 local");

        let rule = create_rule(
            &db,
            account_id,
            "Vendor invoices",
            Some("billing@vendor.example"),
            Some("*invoice*"),
            Some("*.pdf"),
            vec!["facturas".into()],
        )
        .expect("create rule");

        let fake = FakeEmailProvider::new("me@example.com", "Me");
        let saved = apply_rule_with_provider(&db, &rule, account_id, Some(&fake), tmp.path(), None, &|_| {}, &|| {
            false
        })
        .await
        .expect("apply_rule_with_provider should succeed");

        assert_eq!(saved, 1, "the locally stored attachment should be collected");
        let stored = db.get_attachments_for_rule(&rule.id).expect("query attachments");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].filename, "invoice-mar.pdf");
        assert_eq!(stored[0].tags, vec!["facturas".to_string()]);
        let bytes = std::fs::read(tmp.path().join(&stored[0].file_path)).expect("collected file");
        assert_eq!(bytes, b"%PDF-1.4 local");
    }

    fn recorded_pdf(db: &Database, email_id: &str, account_id: &str, filename: &str) {
        db.insert_attachment_infos(
            email_id,
            account_id,
            &[AttachmentInfo {
                attachment_id: format!("att-{filename}"),
                filename: filename.into(),
                mime_type: "application/pdf".into(),
                size: 10,
                inline_data: None,
            }],
        )
        .expect("record attachment");
    }

    /// Regression: with a complete attachment record, a sender's thousands of
    /// attachment-less notifications must not each be fetched from the
    /// provider — only the emails whose recorded files match are visited, and
    /// only those files are downloaded.
    #[tokio::test]
    async fn apply_rule_with_a_complete_record_fetches_only_matching_attachments() {
        use crate::sync::provider::FakeEmailProvider;

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let account_id = "acc-gh";
        make_account(&db, account_id, "gmail", "me@example.com");
        db.set_preference(&crate::services::emails::backfill_done_key(account_id), "1")
            .expect("backfill done");
        for id in ["receipt", "notification-1", "notification-2"] {
            db.insert_email(&make_email(account_id, id, "noreply@github.com", "GitHub"))
                .expect("insert email");
        }
        recorded_pdf(&db, "receipt", account_id, "github-receipt-1.pdf");

        let rule = create_rule(
            &db,
            account_id,
            "GitHub receipts",
            Some("noreply@github.com"),
            None,
            Some("github-*.pdf"),
            vec![],
        )
        .expect("create rule");

        let fake = FakeEmailProvider::new("me@example.com", "Me");
        fake.set_attachment_bytes("receipt", "att-github-receipt-1.pdf", b"%PDF-1.4".to_vec());
        let saved = apply_rule_with_provider(&db, &rule, account_id, Some(&fake), tmp.path(), None, &|_| {}, &|| {
            false
        })
        .await
        .expect("apply");

        assert_eq!(saved, 1);
        assert!(
            !fake.calls().iter().any(|c| c == "get_message"),
            "no whole-message fetch with a complete record, got {:?}",
            fake.calls()
        );
    }

    #[tokio::test]
    async fn apply_rule_reports_progress_up_to_the_total() {
        use crate::sync::provider::FakeEmailProvider;
        use std::sync::Mutex;

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let account_id = "acc-p";
        make_account(&db, account_id, "gmail", "me@example.com");
        db.set_preference(&crate::services::emails::backfill_done_key(account_id), "1")
            .expect("backfill done");
        let fake = FakeEmailProvider::new("me@example.com", "Me");
        for id in ["r1", "r2"] {
            db.insert_email(&make_email(account_id, id, "billing@acme.com", "Invoice"))
                .expect("insert email");
            recorded_pdf(&db, id, account_id, &format!("{id}.pdf"));
            fake.set_attachment_bytes(id, format!("att-{id}.pdf"), b"%PDF".to_vec());
        }
        let rule =
            create_rule(&db, account_id, "Acme", Some("billing@acme.com"), None, None, vec![]).expect("create rule");

        let seen = Mutex::new(Vec::new());
        apply_rule_with_provider(
            &db,
            &rule,
            account_id,
            Some(&fake),
            tmp.path(),
            None,
            &|p| seen.lock().expect("lock").push(p),
            &|| false,
        )
        .await
        .expect("apply");

        let seen = seen.into_inner().expect("lock");
        assert_eq!(
            seen.first(),
            Some(&RetroProgress {
                processed: 0,
                total: 2,
                saved: 0
            })
        );
        assert_eq!(
            seen.last(),
            Some(&RetroProgress {
                processed: 2,
                total: 2,
                saved: 2
            })
        );
    }

    /// A rule whose matching emails all have their attachments stored on
    /// disk collects them without asking the provider, so an account with no
    /// usable credentials (or offline) still gets its attachments.
    #[tokio::test]
    async fn apply_rule_retroactively_collects_stored_attachments_without_credentials() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let account_id = "acc-imap-no-creds";
        make_account(&db, account_id, "imap", "me@example.com");
        for (id, month) in [("msg-jan", "January"), ("msg-feb", "February")] {
            let email = make_email(
                account_id,
                id,
                "billing@vendor.example",
                &format!("Your invoice for {month}"),
            );
            db.insert_email(&email).expect("insert email");
            store_local_attachment(&db, tmp.path(), &email, &format!("invoice-{id}.pdf"), b"%PDF-1.4 local");
        }
        let rule = create_rule(
            &db,
            account_id,
            "Vendor invoices",
            Some("billing@vendor.example"),
            Some("*invoice*"),
            Some("*.pdf"),
            vec!["facturas".into()],
        )
        .expect("create rule");

        let saved = apply_rule_retroactively(&db, &rule.id, account_id, tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("stored attachments must not need credentials");

        assert_eq!(saved, 2);
        assert_eq!(
            db.get_attachments_for_rule(&rule.id).expect("query attachments").len(),
            2
        );
    }

    /// When a matching email has no local copy the provider is still needed,
    /// and an account without credentials still reports that error.
    #[tokio::test]
    async fn apply_rule_retroactively_still_needs_credentials_for_attachments_not_stored() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let account_id = "acc-imap-no-creds-remote";
        make_account(&db, account_id, "imap", "me@example.com");
        let email = make_email(
            account_id,
            "msg-remote",
            "billing@vendor.example",
            "Your invoice for March",
        );
        db.insert_email(&email).expect("insert email");
        let rule = create_rule(
            &db,
            account_id,
            "Vendor invoices",
            Some("billing@vendor.example"),
            None,
            None,
            vec![],
        )
        .expect("create rule");

        let result = apply_rule_retroactively(&db, &rule.id, account_id, tmp.path(), None, &|_| {}, &|| false).await;

        assert!(
            result.is_err(),
            "a fetch is needed, so missing credentials must surface: {result:?}"
        );
    }

    // ── One apply per rule, and applies racing edits / deletes ─────────────

    #[test]
    fn an_apply_ticket_can_move_into_a_background_task() {
        fn assert_background<T: Send + 'static>(_: T) {}
        let applies = Arc::new(RuleApplies::default());

        assert_background(applies.begin("rule-1"));
    }

    #[test]
    fn a_finished_apply_reports_what_it_saved() {
        assert_eq!(
            ApplyOutcome::from_result(&Ok(3)),
            ApplyOutcome {
                status: ApplyStatus::Done,
                saved: 3,
                error: None
            }
        );
    }

    #[test]
    fn a_cancelled_apply_is_reported_as_cancelled_not_failed() {
        assert_eq!(
            ApplyOutcome::from_result(&Err(AppError::Cancelled)).status,
            ApplyStatus::Cancelled
        );
    }

    #[test]
    fn a_failed_apply_carries_its_error() {
        let outcome = ApplyOutcome::from_result(&Err(AppError::NotFound("Rule r1 not found".into())));

        assert_eq!(outcome.status, ApplyStatus::Failed);
        assert!(outcome.error.is_some_and(|e| e.message.contains("r1")));
    }

    #[test]
    fn beginning_an_apply_cancels_the_one_already_running_for_the_rule() {
        let applies = Arc::new(RuleApplies::default());
        let first = applies.begin("rule-1");

        let second = applies.begin("rule-1");

        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
    }

    #[test]
    fn cancelling_a_rule_leaves_other_rules_running() {
        let applies = Arc::new(RuleApplies::default());
        let one = applies.begin("rule-1");
        let two = applies.begin("rule-2");

        applies.cancel("rule-1");

        assert!(one.is_cancelled());
        assert!(!two.is_cancelled());
    }

    #[test]
    fn a_finished_older_apply_does_not_unregister_the_newer_one() {
        let applies = Arc::new(RuleApplies::default());
        let first = applies.begin("rule-1");
        let second = applies.begin("rule-1");
        drop(first);

        applies.cancel("rule-1");

        assert!(second.is_cancelled());
    }

    /// One email with a recorded, downloadable PDF from billing@acme.com, and
    /// a rule collecting it.
    fn acme_invoice_setup(db: &Arc<Database>, account_id: &str) -> (AttachmentRule, FakeEmailProvider) {
        make_account(db, account_id, "gmail", "me@example.com");
        db.set_preference(&crate::services::emails::backfill_done_key(account_id), "1")
            .expect("backfill done");
        db.insert_email(&make_email(account_id, "inv-1", "billing@acme.com", "Invoice"))
            .expect("insert email");
        recorded_pdf(db, "inv-1", account_id, "invoice-1.pdf");
        let fake = FakeEmailProvider::new("me@example.com", "Me");
        fake.set_attachment_bytes("inv-1", "att-invoice-1.pdf", b"%PDF".to_vec());
        let rule =
            create_rule(db, account_id, "Acme", Some("billing@acme.com"), None, None, vec![]).expect("create rule");
        (rule, fake)
    }

    fn files_under(dir: &Path) -> usize {
        walkdir_count(dir)
    }

    fn walkdir_count(dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
        entries
            .flatten()
            .map(|e| {
                let path = e.path();
                if path.is_dir() {
                    walkdir_count(&path)
                } else {
                    1
                }
            })
            .sum()
    }

    #[tokio::test]
    async fn a_cancelled_apply_stops_and_reports_cancelled() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-c");

        let result =
            apply_rule_with_provider(&db, &rule, "acc-c", Some(&fake), tmp.path(), None, &|_| {}, &|| true).await;

        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
        assert!(db.get_attachments_for_rule(&rule.id).expect("query").is_empty());
    }

    #[tokio::test]
    async fn an_apply_of_a_rule_edited_meanwhile_keeps_nothing_it_downloaded() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-e");
        // The apply planned with this snapshot; the user saved an edit since.
        let mut stale = rule.clone();
        stale.updated_at -= 60;

        let saved = apply_rule_with_provider(&db, &stale, "acc-e", Some(&fake), tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("apply");

        assert_eq!(saved, 0);
        assert!(db.get_attachments_for_rule(&rule.id).expect("query").is_empty());
        assert_eq!(
            files_under(tmp.path()),
            0,
            "the downloaded file must not be left behind"
        );
    }

    #[tokio::test]
    async fn an_apply_of_a_rule_deleted_meanwhile_ends_quietly_without_orphans() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-d");
        delete_rule(&db, &rule.id, "acc-d", tmp.path()).expect("delete rule");

        let saved = apply_rule_with_provider(&db, &rule, "acc-d", Some(&fake), tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("a deleted rule is not an error");

        assert_eq!(saved, 0);
        assert_eq!(files_under(tmp.path()), 0);
    }

    #[tokio::test]
    async fn an_attachment_stored_by_a_concurrent_run_is_not_counted_twice() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-dup");
        let email = db.get_email("inv-1").expect("get").expect("email");
        let infos = db.get_attachment_infos("inv-1").expect("infos");
        let first = process_attachments_for_email(
            &db,
            Some(&fake),
            &email,
            &infos,
            std::slice::from_ref(&rule),
            tmp.path(),
            None,
        )
        .await
        .expect("first run");

        // A second run that already passed its dedup check (sync racing the
        // apply) must neither count nor keep its copy.
        let again = store_collected_attachment(&db, &email, &infos[0], &rule, b"%PDF", tmp.path())
            .await
            .expect("second store");

        assert_eq!(first, 1);
        assert!(!again);
        assert_eq!(files_under(tmp.path()), 1);
    }

    #[tokio::test]
    async fn one_email_that_fails_to_store_does_not_abort_the_apply() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-f");
        db.insert_email(&make_email("acc-f", "inv-2", "billing@acme.com", "Invoice"))
            .expect("insert email");
        recorded_pdf(&db, "inv-2", "acc-f", "invoice-2.pdf");
        // inv-1's bytes download; inv-2's do not exist on the provider.
        let seen = std::sync::Mutex::new(Vec::new());

        let saved = apply_rule_with_provider(
            &db,
            &rule,
            "acc-f",
            Some(&fake),
            tmp.path(),
            None,
            &|p| seen.lock().expect("lock").push(p),
            &|| false,
        )
        .await
        .expect("apply");

        assert_eq!(saved, 1);
        assert_eq!(seen.into_inner().expect("lock").last().map(|p| p.processed), Some(2));
    }

    #[tokio::test]
    async fn a_write_failure_is_reported_per_email_and_the_apply_finishes() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-w");
        // `attachments/acc-w` is a file, so no attachment can be written.
        std::fs::create_dir_all(tmp.path().join("attachments")).expect("mkdir");
        std::fs::write(tmp.path().join("attachments/acc-w"), b"not a dir").expect("block");

        let saved = apply_rule_with_provider(&db, &rule, "acc-w", Some(&fake), tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("a per-email failure must not abort the apply");

        assert_eq!(saved, 0);
    }

    #[test]
    fn deleting_a_rule_removes_its_rows_and_files_together() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        make_account(&db, "acc-del", "gmail", "me@example.com");
        db.insert_email(&make_email("acc-del", "m1", "billing@acme.com", "Invoice"))
            .expect("insert email");
        let rule = create_rule(&db, "acc-del", "Acme", Some("billing@acme.com"), None, None, vec![]).expect("rule");
        let relative = "attachments/acc-del/f.pdf";
        std::fs::create_dir_all(tmp.path().join("attachments/acc-del")).expect("mkdir");
        std::fs::write(tmp.path().join(relative), b"%PDF").expect("write");
        db.insert_attachment(&Attachment {
            id: "a1".into(),
            account_id: "acc-del".into(),
            email_id: "m1".into(),
            rule_id: rule.id.clone(),
            gmail_attachment_id: String::new(),
            filename: "f.pdf".into(),
            mime_type: "application/pdf".into(),
            file_size: 4,
            file_path: relative.into(),
            tags: vec![],
            sender_email: "billing@acme.com".into(),
            subject: "Invoice".into(),
            email_timestamp: 0,
            created_at: 0,
        })
        .expect("insert attachment");

        delete_rule(&db, &rule.id, "acc-del", tmp.path()).expect("delete");

        assert!(db.get_attachment_rule(&rule.id).expect("get").is_none());
        assert_eq!(db.count_attachments_for_rule(&rule.id).expect("count"), 0);
        assert!(!tmp.path().join(relative).exists());
    }

    // ── The queued apply: what it tells the frontend ───────────────────────

    type Emitted = std::sync::Mutex<Vec<(&'static str, serde_json::Value)>>;

    fn recorder(emitted: &Emitted) -> impl Fn(&'static str, serde_json::Value) + Send + Sync + '_ {
        move |name, payload| emitted.lock().expect("lock").push((name, payload))
    }

    /// Two emails whose attachments are already on disk: an apply needs no
    /// provider (and so no credentials).
    fn stored_invoices(db: &Arc<Database>, data_dir: &Path, account_id: &str) -> AttachmentRule {
        make_account(db, account_id, "imap", "me@example.com");
        for id in ["q1", "q2"] {
            let email = make_email(account_id, id, "billing@vendor.example", "Invoice");
            db.insert_email(&email).expect("insert email");
            store_local_attachment(db, data_dir, &email, &format!("invoice-{id}.pdf"), b"%PDF");
        }
        create_rule(
            db,
            account_id,
            "Vendor",
            Some("billing@vendor.example"),
            None,
            None,
            vec![],
        )
        .expect("rule")
    }

    #[tokio::test]
    async fn a_queued_apply_reports_progress_and_its_outcome_under_its_run_id() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let rule = stored_invoices(&db, tmp.path(), "acc-q");
        let applies = Arc::new(RuleApplies::default());
        let emitted = Emitted::default();

        run_rule_apply(
            &db,
            &rule.id,
            "acc-q",
            "run-7",
            tmp.path(),
            None,
            &applies.begin(&rule.id),
            &recorder(&emitted),
        )
        .await;

        let emitted = emitted.into_inner().expect("lock");
        let progress: Vec<_> = emitted.iter().filter(|(n, _)| *n == APPLY_PROGRESS_EVENT).collect();
        assert!(!progress.is_empty());
        assert!(progress
            .iter()
            .all(|(_, p)| p["runId"] == "run-7" && p["ruleId"] == rule.id.as_str()));
        let (name, finished) = emitted.last().expect("finished");
        assert_eq!(*name, APPLY_FINISHED_EVENT);
        assert_eq!(finished["runId"], "run-7");
        assert_eq!(finished["status"], "done");
        assert_eq!(finished["saved"], 2);
    }

    #[tokio::test]
    async fn a_superseded_queued_apply_reports_cancelled() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let rule = stored_invoices(&db, tmp.path(), "acc-q2");
        let applies = Arc::new(RuleApplies::default());
        let older = applies.begin(&rule.id);
        let _newer = applies.begin(&rule.id);
        let emitted = Emitted::default();

        run_rule_apply(
            &db,
            &rule.id,
            "acc-q2",
            "run-old",
            tmp.path(),
            None,
            &older,
            &recorder(&emitted),
        )
        .await;

        let emitted = emitted.into_inner().expect("lock");
        assert_eq!(
            emitted.last().map(|(_, p)| p["status"].clone()),
            Some("cancelled".into())
        );
        assert!(db.get_attachments_for_rule(&rule.id).expect("query").is_empty());
    }

    #[tokio::test]
    async fn a_queued_apply_of_a_missing_rule_reports_failed_with_the_error() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        make_account(&db, "acc-q3", "imap", "me@example.com");
        let applies = Arc::new(RuleApplies::default());
        let emitted = Emitted::default();

        run_rule_apply(
            &db,
            "ghost",
            "acc-q3",
            "run-1",
            tmp.path(),
            None,
            &applies.begin("ghost"),
            &recorder(&emitted),
        )
        .await;

        let emitted = emitted.into_inner().expect("lock");
        let (_, finished) = emitted.last().expect("finished");
        assert_eq!(finished["status"], "failed");
        assert_eq!(finished["error"]["code"], "not_found");
    }

    /// Sync test on a throwaway runtime: it holds the global seam lock for its
    /// whole body, which must not span an await (`clippy::await_holding_lock`).
    #[test]
    fn a_scan_reports_its_summary_to_the_output_panel_without_an_app_handle() {
        let _seam = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let rule = stored_invoices(&db, tmp.path(), "acc-log");

        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(apply_rule_with_provider(
                &db,
                &rule,
                "acc-log",
                None,
                tmp.path(),
                None,
                &|_| {},
                &|| false,
            ))
            .expect("apply");

        assert!(
            logger
                .events()
                .iter()
                .any(|e| e.source == "attachments" && e.level == "success" && e.message.contains("saved 2")),
            "{:?}",
            logger.events()
        );
    }

    // ── Which mailboxes rules reach ────────────────────────────────────────

    #[test]
    fn rules_reach_the_inbox_sent_archive_and_filed_folders_not_spam_or_trash() {
        for (mailbox, applies) in [
            ("inbox", true),
            ("sent", true),
            ("folder:INBOX.Facturas", true),
            ("folder:Archive", true),
            ("archive", true),
            ("spam", false),
            ("trash", false),
        ] {
            assert_eq!(rules_apply_to_mailbox(mailbox), applies, "{mailbox}");
        }
    }

    #[tokio::test]
    async fn applying_a_rule_to_existing_mail_skips_spam_and_trash() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        make_account(&db, "acc-mb", "imap", "me@example.com");
        for mailbox in ["inbox", "sent", "folder:INBOX.Facturas", "spam", "trash"] {
            let mut email = make_email("acc-mb", &format!("m-{mailbox}"), "billing@vendor.example", "Invoice");
            email.mailbox = mailbox.into();
            email.is_sent = mailbox == "sent";
            db.insert_email(&email).expect("insert email");
            store_local_attachment(
                &db,
                tmp.path(),
                &email,
                &format!("invoice-{}.pdf", email.id.len()),
                b"%PDF",
            );
        }
        let rule = create_rule(
            &db,
            "acc-mb",
            "Vendor",
            Some("billing@vendor.example"),
            None,
            None,
            vec![],
        )
        .expect("rule");

        apply_rule_with_provider(&db, &rule, "acc-mb", None, tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("apply");

        let mut collected: Vec<String> = db
            .get_attachments_for_rule(&rule.id)
            .expect("query")
            .into_iter()
            .map(|a| a.email_id)
            .collect();
        collected.sort();
        assert_eq!(collected, vec!["m-folder:INBOX.Facturas", "m-inbox", "m-sent"]);
    }

    // ── Editing a rule drops what it no longer matches ─────────────────────

    #[tokio::test]
    async fn narrowing_a_rule_removes_the_attachments_it_no_longer_matches() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let rule = stored_invoices(&db, tmp.path(), "acc-n");
        apply_rule_with_provider(&db, &rule, "acc-n", None, tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("apply");
        let before = db.get_attachments_for_rule(&rule.id).expect("query");
        assert_eq!(before.len(), 2);

        update_rule(
            &db,
            &rule.id,
            "Vendor",
            Some("billing@vendor.example"),
            None,
            Some("invoice-q1.pdf"),
            vec!["kept".into()],
            true,
            tmp.path(),
        )
        .expect("update");

        let after = db.get_attachments_for_rule(&rule.id).expect("query");
        assert_eq!(
            after.iter().map(|a| a.filename.as_str()).collect::<Vec<_>>(),
            vec!["invoice-q1.pdf"]
        );
        assert_eq!(after[0].tags, vec!["kept".to_string()], "keepers are retagged");
        let dropped = before.iter().find(|a| a.filename == "invoice-q2.pdf").expect("q2");
        assert!(
            !tmp.path().join(&dropped.file_path).exists(),
            "the dropped file is removed from disk"
        );
    }

    // ── Emails the apply has to fetch ──────────────────────────────────────

    #[tokio::test]
    async fn without_a_provider_an_email_that_needs_fetching_is_skipped() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        make_account(&db, "acc-s", "imap", "me@example.com");
        // No backfill: the record is not trusted, so the message must be fetched.
        db.insert_email(&make_email("acc-s", "m1", "billing@acme.com", "Invoice"))
            .expect("insert email");
        let rule = create_rule(&db, "acc-s", "Acme", Some("billing@acme.com"), None, None, vec![]).expect("rule");

        let saved = apply_rule_with_provider(&db, &rule, "acc-s", None, tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("a skipped email is not an error");

        assert_eq!(saved, 0);
    }

    #[tokio::test]
    async fn an_email_the_provider_cannot_return_is_skipped() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        make_account(&db, "acc-p2", "gmail", "me@example.com");
        db.insert_email(&make_email("acc-p2", "m1", "billing@acme.com", "Invoice"))
            .expect("insert email");
        let rule = create_rule(&db, "acc-p2", "Acme", Some("billing@acme.com"), None, None, vec![]).expect("rule");
        let fake = FakeEmailProvider::new("me@example.com", "Me");
        fake.fail_message("m1");

        let saved = apply_rule_with_provider(&db, &rule, "acc-p2", Some(&fake), tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("a failed fetch is not an error");

        assert_eq!(saved, 0);
    }

    #[test]
    fn a_rule_needs_the_provider_only_for_attachments_not_on_disk() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let rule = stored_invoices(&db, tmp.path(), "acc-np");
        db.set_preference(&crate::services::emails::backfill_done_key("acc-np"), "1")
            .expect("backfill done");
        let plan = plan_for(&db, &rule, "acc-np").expect("plan");
        assert!(!rule_needs_provider(&db, &rule, &plan.emails, tmp.path()).expect("on disk"));

        db.insert_email(&make_email("acc-np", "remote", "billing@vendor.example", "Invoice"))
            .expect("insert email");
        recorded_pdf(&db, "remote", "acc-np", "remote.pdf");
        let plan = plan_for(&db, &rule, "acc-np").expect("plan");
        assert!(rule_needs_provider(&db, &rule, &plan.emails, tmp.path()).expect("remote"));
    }

    // --- bulk_download_name ---

    #[test]
    fn bulk_download_name_prefixes_the_rule_name() {
        assert_eq!(
            bulk_download_name(Some("Monthly invoices"), "march.pdf"),
            "Monthly_invoices_march.pdf"
        );
    }

    #[test]
    fn bulk_download_name_strips_path_components_from_the_email_filename() {
        assert_eq!(
            bulk_download_name(None, "../../Library/LaunchAgents/x.plist"),
            "x.plist"
        );
        assert_eq!(bulk_download_name(None, "/etc/evil.sh"), "evil.sh");
        assert_eq!(
            bulk_download_name(Some("Invoices"), "..\\..\\evil.sh"),
            "Invoices_evil.sh"
        );
    }

    #[test]
    fn bulk_download_name_keeps_a_rule_name_with_slashes_in_one_component() {
        assert_eq!(bulk_download_name(Some("a/../b"), "x.pdf"), "a_.._b_x.pdf");
    }

    // --- quarantine of every attachment file written to disk ---

    #[cfg(target_os = "macos")]
    use crate::services::attachment_safety::read_quarantine;

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_saved_to_downloads_is_quarantined() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let path = save_bytes_to_downloads(tmp.path(), "run.command", b"#!/bin/sh\n").expect("save");
        assert!(read_quarantine(&path).is_some());
    }

    #[test]
    fn a_bulk_download_copies_the_file_under_a_free_name() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let src = tmp.path().join("stored.pdf");
        std::fs::write(&src, b"%PDF").expect("seed");
        let downloads = tmp.path().join("Downloads");
        std::fs::create_dir_all(&downloads).expect("mkdir");
        std::fs::write(downloads.join("Rule_report.pdf"), b"old").expect("seed");

        let dest = copy_attachment_to_downloads(&src, &downloads, "Rule_report.pdf").expect("copy");

        assert_eq!(dest, downloads.join("Rule_report (1).pdf"));
        assert_eq!(std::fs::read(&dest).expect("read"), b"%PDF");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_bulk_downloaded_copy_is_quarantined_even_when_the_stored_file_was_not() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let src = tmp.path().join("stored.html");
        std::fs::write(&src, b"<p>x</p>").expect("seed");
        assert!(read_quarantine(&src).is_none());

        let dest = copy_attachment_to_downloads(&src, tmp.path(), "page.html").expect("copy");

        assert!(read_quarantine(&dest).is_some());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn an_attachment_collected_by_a_rule_is_quarantined() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        let (rule, fake) = acme_invoice_setup(&db, "acc-q");

        apply_rule_with_provider(&db, &rule, "acc-q", Some(&fake), tmp.path(), None, &|_| {}, &|| false)
            .await
            .expect("apply");

        let stored = db.get_attachments_for_rule(&rule.id).expect("attachments");
        assert!(!stored.is_empty());
        for attachment in stored {
            assert!(read_quarantine(&tmp.path().join(&attachment.file_path)).is_some());
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn an_auto_downloaded_attachment_is_quarantined() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let tmp = tempfile::tempdir().expect("tmp dir");
        make_account(&db, "acc-auto", "gmail", "me@example.com");
        let email = make_email("acc-auto", "msg-auto", "sender@example.com", "Files");
        db.insert_email(&email).expect("insert email");
        let info = inline_attachment("setup.command", "application/x-sh", b"#!/bin/sh\n");
        db.insert_email_attachment_meta(&email.id, "acc-auto", "", &info.filename, &info.mime_type, info.size)
            .expect("meta");
        let fake = FakeEmailProvider::new("me@example.com", "Me");

        let saved = auto_download_attachments(&db, &fake, &email, &[info], tmp.path())
            .await
            .expect("auto download");

        assert_eq!(saved, 1);
        let metas = db.get_email_attachment_metas(&email.id).expect("metas");
        let file_path = metas[0].file_path.clone().expect("stored path");
        assert!(read_quarantine(&tmp.path().join(file_path)).is_some());
    }

    #[test]
    fn a_directory_that_would_launch_is_selected_never_opened() {
        let bundle = Path::new("/home/x/Downloads/Tool.app");
        assert_eq!(
            plan_reveal("macos", bundle, true),
            RevealAction::Select {
                program: "open".into(),
                args: vec!["-R".into(), bundle.to_string_lossy().into_owned()],
            }
        );
        assert_eq!(
            plan_reveal("linux", bundle, true),
            RevealAction::OpenDirectory(PathBuf::from("/home/x/Downloads"))
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
