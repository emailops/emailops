//! Files behind the attachments of drafts pulled from a provider.
//!
//! A draft is pushed back and sent with exactly the files it has here, so a
//! pulled draft needs its attachments on disk. They live at generated names
//! under `attachments/<account>/drafts/`, recorded relative to the data dir
//! like every other stored attachment, so moving the data dir moves them too.
//! The provider's filename is only ever a label.
//!
//! A row whose file is not there is a **waiting** row: its bytes have not been
//! downloaded yet, or the provider offers none. Reading it fails, which is what
//! keeps such a draft from being pushed or sent without the file; nothing
//! upstream changes until it is filled or the user removes the attachment. Its
//! path never changes, so a composer holding the draft stays valid.
//!
//! The rest of the app's draft attachments are the user's own files, at paths
//! the user chose. Only files under the directory above are ever deleted here.

use std::path::{Path, PathBuf};

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{DraftAttachment, ProviderDraft, ProviderDraftAttachment};
use crate::services::attachment_safety::quarantine_saved_file;
use crate::services::attachments::{
    decode_inline_base64, remove_attachment_file, safe_attachment_path, safe_extension,
};
use crate::services::logger;
use crate::sync::provider::EmailProvider;

/// `<data dir>`, derived from where the database lives. `None` for an in-memory
/// test database, which has no data dir.
fn data_dir(db: &Database) -> Option<&Path> {
    db.db_path().parent().filter(|dir| !dir.as_os_str().is_empty())
}

fn new_pulled_path(account_id: &str, filename: &str) -> String {
    format!(
        "attachments/{account_id}/drafts/{}.{}",
        uuid::Uuid::new_v4(),
        safe_extension(filename)
    )
}

/// Whether a draft attachment's path is one of the files this module stores.
fn is_pulled_file(file_path: &str) -> bool {
    matches!(
        file_path.split('/').collect::<Vec<_>>()[..],
        ["attachments", account, "drafts", name]
            if !matches!(account, "" | "." | "..") && !matches!(name, "" | "." | "..")
    )
}

/// The bytes of a draft attachment: the user's own file, or a pulled one.
pub(super) fn read_draft_file(db: &Database, att: &DraftAttachment) -> Result<Vec<u8>> {
    let path = if is_pulled_file(&att.file_path) {
        pulled_path(db, att)?
    } else {
        PathBuf::from(&att.file_path)
    };
    std::fs::read(&path).map_err(|e| AppError::IoError(format!("Failed to read attachment {}: {e}", att.file_path)))
}

fn pulled_path(db: &Database, att: &DraftAttachment) -> Result<PathBuf> {
    let waiting = || {
        AppError::InvalidInput(format!(
            "Attachment \"{}\" of this draft is not on this device yet. Try again after the next sync, \
             or remove it from the draft.",
            att.filename
        ))
    };
    match safe_attachment_path(data_dir(db).ok_or_else(waiting)?, &att.file_path) {
        Err(AppError::NotFound(_)) => Err(waiting()),
        resolved => resolved,
    }
}

async fn write_pulled_file(db: &Database, file_path: &str, bytes: &[u8]) -> Result<()> {
    let dir = data_dir(db).ok_or_else(|| AppError::IoError("This database has no data directory".to_string()))?;
    let path = dir.join(file_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::IoError(format!("Failed to create attachment directory: {e}")))?;
    }
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| AppError::IoError(format!("Failed to write attachment file: {e}")))?;
    quarantine_saved_file(&path);
    Ok(())
}

/// Delete the files among `file_paths` that this module stored. The others are
/// the user's own and stay. A waiting row has no file to delete.
pub(super) fn remove_pulled_files(db: &Database, file_paths: &[String]) {
    let Some(dir) = data_dir(db) else { return };
    for file_path in file_paths.iter().filter(|path| is_pulled_file(path)) {
        match safe_attachment_path(dir, file_path) {
            Ok(path) => remove_attachment_file(&path),
            Err(AppError::NotFound(_)) => {}
            Err(e) => logger::log("warn", "drafts", format!("A pulled attachment was not removed: {e}")),
        }
    }
}

/// The bytes of a provider attachment. `None` when the provider offers none.
async fn provider_bytes(provider: &dyn EmailProvider, att: &ProviderDraftAttachment) -> Result<Option<Vec<u8>>> {
    if let Some(data) = att.inline_data.as_deref().filter(|data| !data.is_empty()) {
        return decode_inline_base64(data).map(Some);
    }
    if att.attachment_id.is_empty() {
        return Ok(None);
    }
    provider
        .fetch_attachment_bytes(&att.message_id, &att.attachment_id)
        .await
        .map(Some)
}

/// Take the first row called `filename` out of `rows`.
fn claim_row(rows: &mut Vec<DraftAttachment>, filename: &str) -> Option<DraftAttachment> {
    let index = rows.iter().position(|row| row.filename == filename)?;
    Some(rows.remove(index))
}

/// A pulled draft's attachments, ready to record.
struct PulledFiles {
    rows: Vec<DraftAttachment>,
    /// Files this pass wrote.
    written: Vec<String>,
    /// False when an attachment the provider does hold could not be stored.
    complete: bool,
}

/// Download `upstream` against the rows the draft already has.
///
/// An attachment whose bytes match its row keeps that row's path: every pull
/// after a push reads the same files back, and a composer open on the draft
/// holds those paths. One that could not be downloaded keeps its row as well,
/// and the draft is read again on the next pull.
async fn download(
    db: &Database,
    account_id: &str,
    provider: &dyn EmailProvider,
    draft: &ProviderDraft,
    mut existing: Vec<DraftAttachment>,
) -> PulledFiles {
    let mut files = PulledFiles {
        rows: Vec::with_capacity(draft.attachments.len()),
        written: Vec::new(),
        complete: true,
    };
    for att in &draft.attachments {
        let row = claim_row(&mut existing, &att.filename);
        // A row that cannot be read compares as different, and a new copy
        // takes its place.
        let stored = row.as_ref().and_then(|row| read_draft_file(db, row).ok());
        let waiting = row
            .as_ref()
            .filter(|row| is_pulled_file(&row.file_path) && stored.is_none());
        let fallback = row.as_ref().map_or_else(
            || new_pulled_path(account_id, &att.filename),
            |row| row.file_path.clone(),
        );
        let file_path = match provider_bytes(provider, att).await {
            Ok(Some(bytes)) if stored.as_deref() == Some(bytes.as_slice()) => fallback,
            Ok(Some(bytes)) => {
                let target = waiting.map_or_else(
                    || new_pulled_path(account_id, &att.filename),
                    |row| row.file_path.clone(),
                );
                match write_pulled_file(db, &target, &bytes).await {
                    Ok(()) => {
                        files.written.push(target.clone());
                        target
                    }
                    Err(e) => {
                        report_failure(draft, &e);
                        files.complete = false;
                        fallback
                    }
                }
            }
            Ok(None) => fallback,
            Err(e) => {
                report_failure(draft, &e);
                files.complete = false;
                fallback
            }
        };
        files.rows.push(DraftAttachment {
            id: String::new(),
            draft_id: String::new(),
            file_path,
            filename: att.filename.clone(),
            mime_type: att.mime_type.clone(),
        });
    }
    files
}

fn report_failure(draft: &ProviderDraft, error: &AppError) {
    logger::log(
        "warn",
        "drafts",
        format!(
            "An attachment of provider draft {} is not stored yet and will be retried: {error}",
            draft.provider_draft_id
        ),
    );
}

/// Store a clean draft pulled from the provider together with its attachments.
/// `local_id` is its row when it has one. A failed download is no failure here:
/// the draft is stored with that row waiting and its change token unrecorded.
pub(super) async fn import_draft(
    db: &Database,
    account_id: &str,
    provider: &dyn EmailProvider,
    draft: &ProviderDraft,
    local_id: Option<&str>,
) -> Result<()> {
    let existing = match local_id {
        Some(id) => db.list_draft_attachments(id)?,
        None => Vec::new(),
    };
    let files = download(db, account_id, provider, draft, existing).await;
    let stored = match db.store_pulled_draft(account_id, draft, &files.rows, files.complete) {
        Ok(stored) => stored,
        Err(e) => {
            remove_pulled_files(db, &files.written);
            return Err(e);
        }
    };
    // A save got in first: the local draft wins and keeps what it has.
    if !stored.applied {
        remove_pulled_files(db, &files.written);
    }
    remove_pulled_files(db, &stored.dropped_files);
    Ok(())
}

/// Give a dirty draft with no attachments the ones its provider copy has, as
/// waiting rows. For a draft stored by a version that left them behind: it has
/// nothing of its own to overwrite, and pushing it as it stands would remove
/// them from the provider.
pub(super) fn adopt_attachments(
    db: &Database,
    account_id: &str,
    draft_id: &str,
    upstream: &[ProviderDraftAttachment],
) -> Result<()> {
    if upstream.is_empty() || !db.list_draft_attachments(draft_id)?.is_empty() {
        return Ok(());
    }
    let rows: Vec<DraftAttachment> = upstream
        .iter()
        .map(|att| DraftAttachment {
            id: String::new(),
            draft_id: draft_id.to_string(),
            file_path: new_pulled_path(account_id, &att.filename),
            filename: att.filename.clone(),
            mime_type: att.mime_type.clone(),
        })
        .collect();
    db.replace_draft_attachments(draft_id, &rows)?;
    Ok(())
}

/// Download the waiting files of a dirty draft. The draft wins over the
/// provider, so its rows stay as they are: this only brings the bytes behind
/// the ones that asked for them, matched by filename. Returns false when a
/// download failed and is worth retrying.
pub(super) async fn fill_missing_files(
    db: &Database,
    provider: &dyn EmailProvider,
    draft_id: &str,
    upstream: &[ProviderDraftAttachment],
) -> Result<bool> {
    let mut complete = true;
    let mut candidates: Vec<&ProviderDraftAttachment> = upstream.iter().collect();
    for row in db.list_draft_attachments(draft_id)? {
        if !is_pulled_file(&row.file_path) || read_draft_file(db, &row).is_ok() {
            continue;
        }
        let Some(index) = candidates.iter().position(|att| att.filename == row.filename) else {
            continue;
        };
        let att = candidates.remove(index);
        let filled = match provider_bytes(provider, att).await {
            Ok(Some(bytes)) => write_pulled_file(db, &row.file_path, &bytes).await,
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        if let Err(e) = filled {
            complete = false;
            logger::log(
                "warn",
                "drafts",
                format!("An attachment of draft {draft_id} is not stored yet and will be retried: {e}"),
            );
        }
    }
    Ok(complete)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use base64::Engine;

    use super::{import_draft, is_pulled_file};
    use crate::db::Database;
    use crate::models::error::AppError;
    use crate::models::{Account, Draft, DraftAttachmentInput, ProviderDraft, ProviderDraftAttachment};
    use crate::services::emails::{compose_draft, delete_draft, pull_provider_drafts, send_draft, ComposeInput};
    use crate::sync::provider::{EmailProvider, FakeEmailProvider};

    const REPORT: &[u8] = b"quarterly report";

    /// A database on disk: pulled files live next to it, which an in-memory
    /// database has no place for.
    fn database() -> (tempfile::TempDir, Arc<Database>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = Database::new(tmp.path().to_path_buf()).expect("db");
        (tmp, Arc::new(db))
    }

    fn account(db: &Database, provider: &str) -> Account {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES ('a1', ?1, 'a1@example.com', 'A One', 0, 0, 1)",
                rusqlite::params![provider],
            )
            .expect("seed account");
        db.get_account("a1").expect("get account").expect("account present")
    }

    fn fake() -> FakeEmailProvider {
        FakeEmailProvider::new("a1@example.com", "A One")
    }

    fn report(filename: &str) -> ProviderDraftAttachment {
        ProviderDraftAttachment {
            message_id: "msg-1".to_string(),
            attachment_id: "att-1".to_string(),
            filename: filename.to_string(),
            mime_type: "application/pdf".to_string(),
            inline_data: None,
        }
    }

    fn upstream(token: &str, attachments: Vec<ProviderDraftAttachment>) -> ProviderDraft {
        ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["dest@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "With a file".to_string(),
            body: "see attached".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
            provider_message_id: Some(token.to_string()),
            in_reply_to: None,
            attachments,
        }
    }

    fn file_at(data_dir: &Path, file_path: &str) -> Vec<u8> {
        std::fs::read(data_dir.join(file_path)).expect("attachment file on disk")
    }

    /// Every file stored for pulled drafts.
    fn stored_files(data_dir: &Path) -> Vec<PathBuf> {
        match std::fs::read_dir(data_dir.join("attachments/a1/drafts")) {
            Ok(entries) => entries.map(|entry| entry.expect("entry").path()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn base64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn only_draft(db: &Database) -> Draft {
        let mut drafts = db.list_drafts("a1").expect("list");
        assert_eq!(drafts.len(), 1, "exactly one draft expected");
        drafts.remove(0)
    }

    /// A composer save of a text edit: attachments are left as they are.
    fn text_edit(draft: &Draft, subject: &str) -> ComposeInput {
        ComposeInput {
            draft_id: Some(draft.id.clone()),
            account_id: "a1".to_string(),
            email_id: None,
            to: draft.to_addresses.clone(),
            cc: Vec::new(),
            subject: subject.to_string(),
            body: draft.body.clone(),
            body_html: None,
            attachments: None,
        }
    }

    /// Pull a draft with `report.pdf` stored.
    async fn pulled(db: &Arc<Database>, account: &Account, provider: &FakeEmailProvider) -> Draft {
        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        provider.add_provider_draft(upstream("msg-1", vec![report("report.pdf")]));
        pull_provider_drafts(db, account, provider).await.expect("pull");
        only_draft(db)
    }

    /// Pull a draft with `report.pdf` whose download fails.
    async fn pulled_without_its_file(db: &Arc<Database>, account: &Account, provider: &FakeEmailProvider) -> Draft {
        provider.add_provider_draft(upstream("msg-1", vec![report("report.pdf")]));
        pull_provider_drafts(db, account, provider).await.expect("pull");
        only_draft(db)
    }

    #[tokio::test]
    async fn a_pulled_draft_keeps_its_attachment() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        provider.add_provider_draft(upstream("msg-1", vec![report("report.pdf")]));

        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        let drafts = db.list_drafts("a1").expect("list");
        assert_eq!(
            drafts[0].attachments.len(),
            1,
            "the attachment must come with the draft"
        );
        let attachment = &drafts[0].attachments[0];
        assert_eq!(attachment.filename, "report.pdf");
        assert_eq!(attachment.mime_type, "application/pdf");
        assert_eq!(file_at(tmp.path(), &attachment.file_path), REPORT);
    }

    #[tokio::test]
    async fn an_attachment_that_comes_with_the_listing_is_stored_without_a_download() {
        let (tmp, db) = database();
        let account = account(&db, "outlook");
        let provider = fake();
        let mut small = report("small.pdf");
        small.attachment_id = String::new();
        small.inline_data = Some(base64(REPORT));
        provider.add_provider_draft(upstream("msg-1", vec![small]));

        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        let draft = only_draft(&db);
        assert_eq!(file_at(tmp.path(), &draft.attachments[0].file_path), REPORT);
    }

    #[tokio::test]
    async fn a_filename_from_the_provider_never_becomes_a_path() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        let hostile = ["../../../escape/evil", "..\\..\\evil.sh", "/etc/cron.d/evil"];
        for name in hostile {
            provider.add_provider_draft(upstream("msg-1", vec![report(name)]));
            pull_provider_drafts(&db, &account, &provider).await.expect("pull");

            let draft = only_draft(&db);
            let attachment = &draft.attachments[0];
            assert!(is_pulled_file(&attachment.file_path), "{}", attachment.file_path);
            assert!(!attachment.file_path.contains("evil"), "{}", attachment.file_path);
            assert_eq!(attachment.filename, name, "the name stays as the label it is");
            assert_eq!(file_at(tmp.path(), &attachment.file_path), REPORT);
            delete_draft(&db, &account, &draft.id, Some(&provider))
                .await
                .expect("delete");
        }
        assert!(!tmp.path().join("escape").exists());
        assert!(!tmp.path().parent().expect("parent").join("escape").exists());
    }

    #[tokio::test]
    async fn an_edit_pushes_the_attachment_back() {
        // The reported loss: the edit replaced the provider copy with one that
        // had no attachments.
        let (_tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;

        compose_draft(&db, &account, text_edit(&draft, "Edited"), Some(&provider))
            .await
            .expect("edit");

        let pushed = provider.provider_drafts();
        assert_eq!(pushed[0].subject, "Edited");
        assert_eq!(pushed[0].attachments.len(), 1, "the provider copy keeps its file");
        assert_eq!(pushed[0].attachments[0].filename, "report.pdf");
        assert_eq!(
            pushed[0].attachments[0].inline_data.as_deref(),
            Some(base64(REPORT).as_str())
        );
    }

    #[tokio::test]
    async fn sending_carries_the_attachment_and_leaves_no_file_behind() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;

        send_draft(&db, &account, &draft.id, &provider).await.expect("send");

        let sent = provider.sent();
        assert_eq!(sent[0].attachments.len(), 1, "the message goes out with its file");
        assert_eq!(sent[0].attachments[0].filename, "report.pdf");
        assert_eq!(sent[0].attachments[0].data, base64(REPORT));
        assert!(stored_files(tmp.path()).is_empty(), "the sent draft's file is gone");
    }

    #[tokio::test]
    async fn a_failed_download_is_retried_until_the_file_is_here() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();

        let draft = pulled_without_its_file(&db, &account, &provider).await;
        assert_eq!(
            draft.attachments.len(),
            1,
            "the draft must not look like it has no attachments"
        );
        assert!(stored_files(tmp.path()).is_empty());
        assert!(
            db.provider_draft_change_tokens("a1").expect("tokens").is_empty(),
            "a draft read in part must not count as read"
        );

        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        assert_eq!(pull_provider_drafts(&db, &account, &provider).await.expect("retry"), 1);

        let after = only_draft(&db);
        assert_eq!(
            after.attachments[0].file_path, draft.attachments[0].file_path,
            "same row"
        );
        assert_eq!(file_at(tmp.path(), &after.attachments[0].file_path), REPORT);
        assert_eq!(
            pull_provider_drafts(&db, &account, &provider).await.expect("steady"),
            0,
            "once complete it is not read again"
        );
    }

    #[tokio::test]
    async fn a_draft_missing_a_file_is_not_pushed_and_its_edit_goes_up_once_the_file_is_here() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled_without_its_file(&db, &account, &provider).await;

        let result = compose_draft(&db, &account, text_edit(&draft, "Edited"), Some(&provider)).await;

        assert!(matches!(result, Err(AppError::InvalidInput(_))), "{result:?}");
        let upstream_copy = &provider.provider_drafts()[0];
        assert_eq!(upstream_copy.subject, "With a file", "the provider copy is untouched");
        assert_eq!(upstream_copy.attachments.len(), 1, "its attachment too");
        assert_eq!(only_draft(&db).subject, "Edited", "the edit is kept locally");

        // Still missing on the next pass: nothing goes up.
        pull_provider_drafts(&db, &account, &provider)
            .await
            .expect("still failing");
        assert_eq!(provider.provider_drafts()[0].subject, "With a file");

        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        pull_provider_drafts(&db, &account, &provider).await.expect("recovered");

        let pushed = &provider.provider_drafts()[0];
        assert_eq!(pushed.subject, "Edited", "the local draft wins");
        assert_eq!(
            pushed.attachments[0].inline_data.as_deref(),
            Some(base64(REPORT).as_str())
        );
        assert_eq!(only_draft(&db).attachments[0].file_path, draft.attachments[0].file_path);
        assert_eq!(stored_files(tmp.path()).len(), 1);
    }

    #[tokio::test]
    async fn a_draft_missing_a_file_is_not_sent() {
        let (_tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled_without_its_file(&db, &account, &provider).await;

        let result = send_draft(&db, &account, &draft.id, &provider).await;

        assert!(matches!(result, Err(AppError::InvalidInput(_))), "{result:?}");
        assert!(provider.sent().is_empty(), "nothing goes out without the file");
        assert_eq!(only_draft(&db).id, draft.id, "the local draft stays");
        assert_eq!(provider.provider_drafts().len(), 1, "the provider copy stays");
    }

    #[tokio::test]
    async fn an_attachment_with_no_bytes_to_download_blocks_the_draft_without_polling() {
        // Outlook's item and reference attachments: nothing here can hold them,
        // and pushing would delete them upstream.
        let (_tmp, db) = database();
        let account = account(&db, "outlook");
        let provider = fake();
        let mut item = report("forwarded message");
        item.attachment_id = String::new();
        provider.add_provider_draft(upstream("msg-1", vec![item]));

        pull_provider_drafts(&db, &account, &provider).await.expect("pull");
        assert_eq!(
            pull_provider_drafts(&db, &account, &provider).await.expect("steady"),
            0,
            "no retry can bring it, so the draft is not read again"
        );

        let draft = only_draft(&db);
        assert_eq!(draft.attachments[0].filename, "forwarded message");
        let result = send_draft(&db, &account, &draft.id, &provider).await;
        assert!(matches!(result, Err(AppError::InvalidInput(_))), "{result:?}");

        // Removing it from the draft is the way out, and it is the user's call.
        let mut without = text_edit(&draft, "Without it");
        without.attachments = Some(Vec::new());
        compose_draft(&db, &account, without, Some(&provider))
            .await
            .expect("save without the attachment");
        assert!(provider.provider_drafts()[0].attachments.is_empty());
    }

    #[tokio::test]
    async fn a_dirty_draft_keeps_its_attachments_over_the_providers() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;
        provider.fail_draft_writes(Some("offline"));
        let saved = compose_draft(&db, &account, text_edit(&draft, "Edited here"), Some(&provider)).await;
        assert!(saved.is_err(), "the push fails, the edit stays dirty");
        provider.fail_draft_writes(None);

        // Edited upstream as well, with another file.
        provider.set_attachment_bytes("msg-2", "att-2", b"other".to_vec());
        let mut other = report("other.pdf");
        other.message_id = "msg-2".to_string();
        other.attachment_id = "att-2".to_string();
        provider.add_provider_draft(upstream("msg-2", vec![other]));

        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        let after = only_draft(&db);
        assert_eq!(after.attachments.len(), 1);
        assert_eq!(after.attachments[0].filename, "report.pdf");
        assert_eq!(stored_files(tmp.path()).len(), 1, "the provider's file is not fetched");
        assert_eq!(provider.provider_drafts()[0].attachments[0].filename, "report.pdf");
    }

    #[tokio::test]
    async fn a_save_racing_the_pull_keeps_its_attachments_and_the_download_is_dropped() {
        // The sync planned a pull; the user saved before it was stored.
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;
        let stored_before = stored_files(tmp.path());
        db.save_user_draft(&crate::models::SaveDraftRequest {
            id: Some(draft.id.clone()),
            email_id: None,
            account_id: "a1".to_string(),
            to_addresses: Vec::new(),
            cc_addresses: Vec::new(),
            subject: "Saved".to_string(),
            body: "saved".to_string(),
            body_html: None,
            provider_draft_id: None,
            attachments: None,
        })
        .expect("save");

        provider.set_attachment_bytes("msg-2", "att-1", b"newer".to_vec());
        let mut newer = report("report.pdf");
        newer.message_id = "msg-2".to_string();
        import_draft(&db, "a1", &provider, &upstream("msg-2", vec![newer]), Some(&draft.id))
            .await
            .expect("import");

        let after = only_draft(&db);
        assert_eq!(after.subject, "Saved");
        assert_eq!(after.attachments[0].file_path, draft.attachments[0].file_path);
        assert_eq!(stored_files(tmp.path()), stored_before, "no file left over");
    }

    #[tokio::test]
    async fn re_reading_the_same_bytes_keeps_the_path_and_adds_no_file() {
        // Every push drops the change token, so the next pull reads the draft
        // back. A composer open on it holds this path; replacing it would make
        // its next save fail.
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;
        compose_draft(&db, &account, text_edit(&draft, "Edited"), Some(&provider))
            .await
            .expect("edit");

        assert_eq!(pull_provider_drafts(&db, &account, &provider).await.expect("pull"), 1);

        assert_eq!(only_draft(&db).attachments[0].file_path, draft.attachments[0].file_path);
        assert_eq!(stored_files(tmp.path()).len(), 1);
    }

    #[tokio::test]
    async fn a_pull_that_replaces_a_file_removes_the_old_one() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;

        provider.set_attachment_bytes("msg-2", "att-1", b"second version".to_vec());
        let mut newer = report("report.pdf");
        newer.message_id = "msg-2".to_string();
        provider.add_provider_draft(upstream("msg-2", vec![newer]));
        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        let after = only_draft(&db);
        assert_ne!(after.attachments[0].file_path, draft.attachments[0].file_path);
        assert_eq!(file_at(tmp.path(), &after.attachments[0].file_path), b"second version");
        assert_eq!(stored_files(tmp.path()).len(), 1, "the old file is gone");
    }

    #[tokio::test]
    async fn an_attachment_removed_upstream_goes_with_its_file() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        pulled(&db, &account, &provider).await;

        provider.add_provider_draft(upstream("msg-2", Vec::new()));
        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        assert!(only_draft(&db).attachments.is_empty());
        assert!(stored_files(tmp.path()).is_empty());
    }

    #[tokio::test]
    async fn a_failed_download_keeps_the_file_of_the_same_name() {
        // Read back after a push with the network down: the local copy is
        // fine, and losing it would block a draft that has nothing wrong.
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;

        let mut newer = report("report.pdf");
        newer.message_id = "msg-2".to_string();
        provider.add_provider_draft(upstream("msg-2", vec![newer]));
        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        let after = only_draft(&db);
        assert_eq!(after.attachments[0].file_path, draft.attachments[0].file_path);
        assert_eq!(file_at(tmp.path(), &after.attachments[0].file_path), REPORT);
        assert!(
            db.provider_draft_change_tokens("a1").expect("tokens").is_empty(),
            "and the draft is read again"
        );
    }

    #[tokio::test]
    async fn deleting_a_draft_removes_its_file() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;

        delete_draft(&db, &account, &draft.id, Some(&provider))
            .await
            .expect("delete");

        assert!(stored_files(tmp.path()).is_empty());
    }

    #[tokio::test]
    async fn a_pruned_draft_leaves_no_file_behind() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        pulled(&db, &account, &provider).await;

        provider.delete_draft("srv-1").await.expect("sent elsewhere");
        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        assert!(db.list_drafts("a1").expect("list").is_empty());
        assert!(stored_files(tmp.path()).is_empty());
    }

    #[tokio::test]
    async fn dropping_an_attachment_in_the_composer_removes_its_file() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let draft = pulled(&db, &account, &provider).await;

        let mut without = text_edit(&draft, "No file");
        without.attachments = Some(Vec::new());
        compose_draft(&db, &account, without, Some(&provider))
            .await
            .expect("save");

        assert!(stored_files(tmp.path()).is_empty());
    }

    #[tokio::test]
    async fn a_file_the_user_attached_is_never_deleted() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let own_dir = tempfile::tempdir().expect("tempdir");
        let own_file = own_dir.path().join("mine.pdf");
        std::fs::write(&own_file, b"my own file").expect("write");
        let own_path = own_file.to_string_lossy().to_string();

        let mut input = text_edit(&pulled(&db, &account, &provider).await, "Mine");
        input.attachments = Some(vec![DraftAttachmentInput {
            file_path: own_path.clone(),
            filename: None,
            mime_type: None,
        }]);
        let draft = compose_draft(&db, &account, input, Some(&provider))
            .await
            .expect("attach");
        assert!(
            stored_files(tmp.path()).is_empty(),
            "the pulled file it replaced is gone"
        );

        // Same file read back: the row keeps pointing at the user's copy.
        pull_provider_drafts(&db, &account, &provider).await.expect("pull");
        assert_eq!(only_draft(&db).attachments[0].file_path, own_path);

        // Replaced upstream, then deleted: the user's file outlives both.
        provider.set_attachment_bytes("msg-3", "att-3", b"changed upstream".to_vec());
        let mut changed = report("mine.pdf");
        changed.message_id = "msg-3".to_string();
        changed.attachment_id = "att-3".to_string();
        let mut replacement = upstream("msg-3", vec![changed]);
        replacement.provider_draft_id = draft.provider_draft_id.clone().expect("linked");
        provider.add_provider_draft(replacement);
        pull_provider_drafts(&db, &account, &provider).await.expect("pull");
        assert!(own_file.exists());

        delete_draft(&db, &account, &draft.id, Some(&provider))
            .await
            .expect("delete");
        assert!(own_file.exists());
        assert_eq!(std::fs::read(&own_file).expect("read"), b"my own file");
    }

    #[tokio::test]
    async fn a_file_the_user_attached_from_inside_the_data_dir_is_never_deleted() {
        // The CLI records `--attach` paths as given, and a relative one resolves
        // against the data dir here, where the database itself lives. Only the
        // directory this module writes to counts as its own.
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        let relative = tmp.path().join("notes.txt");
        std::fs::write(&relative, b"relative").expect("write");
        let absolute = tmp.path().join("kept.txt");
        std::fs::write(&absolute, b"absolute").expect("write");

        let mut input = text_edit(&pulled(&db, &account, &provider).await, "Mine");
        input.attachments = Some(
            ["notes.txt".to_string(), absolute.to_string_lossy().to_string()]
                .map(|file_path| DraftAttachmentInput {
                    file_path,
                    filename: None,
                    mime_type: None,
                })
                .to_vec(),
        );
        let draft = compose_draft(&db, &account, input, None).await.expect("attach");

        delete_draft(&db, &account, &draft.id, None).await.expect("delete");

        assert!(relative.exists(), "a relative path is not a pulled file");
        assert!(absolute.exists(), "nor is an absolute one");
    }

    #[tokio::test]
    async fn a_draft_pulled_before_attachments_were_is_read_once_more() {
        // Upgrade path: the row holds a matching change token and no attachment
        // rows, so without a fresh read the next edit deletes the file upstream.
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        let copy = upstream("msg-1", vec![report("report.pdf")]);
        provider.add_provider_draft(copy.clone());
        db.upsert_provider_draft("a1", &copy)
            .expect("row from an older version");

        assert_eq!(pull_provider_drafts(&db, &account, &provider).await.expect("pull"), 1);

        let draft = only_draft(&db);
        assert_eq!(file_at(tmp.path(), &draft.attachments[0].file_path), REPORT);
        assert_eq!(
            pull_provider_drafts(&db, &account, &provider).await.expect("steady"),
            0,
            "once is enough"
        );
    }

    /// An upgrade with an edit not pushed yet: the row came from a version that
    /// left attachments behind, so it has none, and it is dirty, so it wins.
    async fn edited_before_attachments_were_pulled(db: &Arc<Database>, provider: &FakeEmailProvider) -> String {
        let copy = upstream("msg-1", vec![report("report.pdf")]);
        provider.add_provider_draft(copy.clone());
        let id = db
            .upsert_provider_draft("a1", &copy)
            .expect("row from an older version");
        db.save_user_draft(&crate::models::SaveDraftRequest {
            id: Some(id.clone()),
            email_id: None,
            account_id: "a1".to_string(),
            to_addresses: vec!["dest@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Edited offline".to_string(),
            body: "edited".to_string(),
            body_html: None,
            provider_draft_id: None,
            attachments: None,
        })
        .expect("save");
        id
    }

    #[tokio::test]
    async fn an_edit_made_before_attachments_were_pulled_does_not_cost_the_file() {
        let (tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        edited_before_attachments_were_pulled(&db, &provider).await;

        pull_provider_drafts(&db, &account, &provider).await.expect("pull");

        let pushed = &provider.provider_drafts()[0];
        assert_eq!(pushed.subject, "Edited offline", "the local draft still wins");
        assert_eq!(pushed.attachments.len(), 1, "the provider copy keeps its file");
        assert_eq!(
            pushed.attachments[0].inline_data.as_deref(),
            Some(base64(REPORT).as_str())
        );
        assert_eq!(stored_files(tmp.path()).len(), 1);
    }

    #[tokio::test]
    async fn such_an_edit_waits_for_its_file_across_passes() {
        // The row keeps a change token that still matches, so the pass that
        // adopted the attachment must not be the last one to look at it.
        let (_tmp, db) = database();
        let account = account(&db, "gmail");
        let provider = fake();
        edited_before_attachments_were_pulled(&db, &provider).await;

        pull_provider_drafts(&db, &account, &provider)
            .await
            .expect("download fails");
        pull_provider_drafts(&db, &account, &provider)
            .await
            .expect("download fails again");
        assert_eq!(provider.provider_drafts()[0].subject, "With a file", "nothing goes up");

        provider.set_attachment_bytes("msg-1", "att-1", REPORT.to_vec());
        pull_provider_drafts(&db, &account, &provider).await.expect("recovered");

        let pushed = &provider.provider_drafts()[0];
        assert_eq!(pushed.subject, "Edited offline");
        assert_eq!(pushed.attachments.len(), 1);
    }

    #[test]
    fn only_the_directory_this_module_writes_to_counts_as_pulled() {
        assert!(is_pulled_file("attachments/a1/drafts/5f1e.pdf"));
        for own in [
            "/home/user/report.pdf",
            "report.pdf",
            "attachments/a1/5f1e.pdf",
            "attachments/a1/auto/5f1e.pdf",
            "attachments/a1/drafts/nested/5f1e.pdf",
            "/attachments/a1/drafts/5f1e.pdf",
            "attachments/../drafts/5f1e.pdf",
            "attachments/a1/drafts/..",
            "",
        ] {
            assert!(!is_pulled_file(own), "{own}");
        }
    }
}
