//! In-app folder management (IMAP-only in v1): create / rename / delete
//! custom folders and move messages between the inbox and custom folders.
//!
//! Every op is provider-first: the server mutation happens before any local
//! row changes, so a provider failure leaves the local DB untouched. Local
//! state is then migrated in place (rename/move re-key ids so tags,
//! embeddings and FTS survive) — see `db/emails/folder_ops.rs`.

use std::sync::Arc;

use crate::db::folders::FolderUpsert;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Folder, FolderRole};
use crate::services::logger;
use crate::sync::folder_plan::{compose_folder_path, decode_imap_utf7, rename_sibling_path, validate_folder_name};
use crate::sync::imap::folder_email_id_prefix;
use crate::sync::provider::{EmailProvider, MoveTarget};

/// The `emails.mailbox` value for a custom folder's messages.
fn folder_mailbox_value(server_path: &str) -> String {
    format!("folder:{server_path}")
}

/// Look up a folder row and require it to be a user-managed (custom) folder —
/// role folders (Sent/Spam/Trash) are owned by the server layout, not the user.
fn require_custom_folder(db: &Arc<Database>, account_id: &str, folder_id: &str) -> Result<Folder> {
    let folder = db
        .get_folder(account_id, folder_id)?
        .ok_or_else(|| AppError::NotFound(format!("Folder not found: {folder_id}")))?;
    if folder.role != FolderRole::Custom.as_str() {
        return Err(AppError::InvalidInput(
            "Only custom folders can be renamed or deleted".to_string(),
        ));
    }
    Ok(folder)
}

/// Reject a new folder whose wire path or display name collides with an
/// existing folder of the account (display names compared case-insensitively,
/// on the last path segment users actually see).
fn ensure_no_collision(existing: &[Folder], server_path: &str, display_name: &str) -> Result<()> {
    let display_lower = display_name.to_lowercase();
    for folder in existing {
        let existing_last = folder
            .delimiter
            .as_deref()
            .filter(|d| !d.is_empty())
            .and_then(|d| folder.display_name.rsplit(d).next())
            .unwrap_or(&folder.display_name);
        if folder.server_path == server_path || existing_last.to_lowercase() == display_lower {
            return Err(AppError::InvalidInput(format!(
                "A folder named '{display_name}' already exists"
            )));
        }
    }
    Ok(())
}

/// Create a folder on the server and persist its row. `name` is the display
/// name of a single path segment; placement (top-level vs under `INBOX.`) and
/// UTF-7 wire encoding are derived from the account's existing layout.
pub async fn create_folder(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    name: &str,
) -> Result<Folder> {
    let name = name.trim();
    let existing = db.list_folders(&account.id, None)?;
    let delimiter = existing.iter().find_map(|f| f.delimiter.clone());

    validate_folder_name(name, delimiter.as_deref()).map_err(|e| AppError::InvalidInput(e.to_string()))?;
    let existing_paths: Vec<String> = existing.iter().map(|f| f.server_path.clone()).collect();
    let server_path = compose_folder_path(name, delimiter.as_deref(), &existing_paths);
    ensure_no_collision(&existing, &server_path, name)?;

    email_provider.create_folder(&server_path).await?;

    let upsert = FolderUpsert {
        server_path: server_path.clone(),
        display_name: decode_imap_utf7(&server_path),
        role: FolderRole::Custom,
        delimiter,
    };
    db.upsert_folder(&account.id, &upsert)?;
    logger::log(
        "success",
        "sync",
        format!("[{}] Created folder '{}'", account.email, name),
    );

    let folder_id = format!("{}:{}", account.id, server_path);
    db.get_folder(&account.id, &folder_id)?
        .ok_or_else(|| AppError::NotFound(format!("Folder not found after create: {folder_id}")))
}

/// Rename a custom folder on the server and migrate all local state in place:
/// the folder row, every email's id prefix + mailbox (tags/embeddings/FTS
/// follow via `migrate_folder_emails`), and the sync watermarks — so nothing
/// re-downloads. Assumes the server keeps UIDs stable across RENAME (true for
/// mainstream servers); if it doesn't, the next sync re-fetches the folder
/// and the id dedup treats the new UIDs as new messages.
pub async fn rename_folder(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    folder_id: &str,
    new_name: &str,
) -> Result<Folder> {
    let new_name = new_name.trim();
    let folder = require_custom_folder(db, &account.id, folder_id)?;
    validate_folder_name(new_name, folder.delimiter.as_deref()).map_err(|e| AppError::InvalidInput(e.to_string()))?;

    let new_path = rename_sibling_path(&folder.server_path, folder.delimiter.as_deref(), new_name);
    if new_path == folder.server_path {
        return Ok(folder);
    }
    let others: Vec<Folder> = db
        .list_folders(&account.id, None)?
        .into_iter()
        .filter(|f| f.id != folder.id)
        .collect();
    ensure_no_collision(&others, &new_path, new_name)?;

    email_provider.rename_folder(&folder.server_path, &new_path).await?;

    let old_mailbox = folder_mailbox_value(&folder.server_path);
    let new_mailbox = folder_mailbox_value(&new_path);
    let old_prefix = folder_email_id_prefix(&account.id, &folder.server_path);
    let new_prefix = folder_email_id_prefix(&account.id, &new_path);
    db.migrate_folder_emails(&account.id, &old_mailbox, &new_mailbox, &old_prefix, &new_prefix)?;
    let new_id = db.rename_folder_row(&account.id, folder_id, &new_path, &decode_imap_utf7(&new_path))?;

    // Carry the sync watermarks to the new path so the renamed folder resumes
    // incrementally instead of re-walking its history.
    let old_keys = super::sync::custom_folder_pref_keys(&account.id, &folder.server_path);
    let new_keys = super::sync::custom_folder_pref_keys(&account.id, &new_path);
    for (old_key, new_key) in old_keys.iter().zip(new_keys.iter()) {
        if let Some(value) = db.get_preference(old_key)? {
            db.set_preference(new_key, &value)?;
        }
        db.delete_preference(old_key)?;
    }

    logger::log(
        "success",
        "sync",
        format!(
            "[{}] Renamed folder '{}' to '{}'",
            account.email, folder.display_name, new_name
        ),
    );
    db.get_folder(&account.id, &new_id)?
        .ok_or_else(|| AppError::NotFound(format!("Folder not found after rename: {new_id}")))
}

/// Delete a custom folder on the server, then hard-delete its local emails
/// (they are gone server-side too), its folder row, and its sync watermarks.
pub async fn delete_folder(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    folder_id: &str,
) -> Result<()> {
    let folder = require_custom_folder(db, &account.id, folder_id)?;

    email_provider.delete_folder(&folder.server_path).await?;

    let deleted = db.delete_emails_in_mailbox(&account.id, &folder_mailbox_value(&folder.server_path))?;
    db.delete_folder_row(&account.id, folder_id)?;
    for key in super::sync::custom_folder_pref_keys(&account.id, &folder.server_path) {
        db.delete_preference(&key)?;
    }

    logger::log(
        "success",
        "sync",
        format!(
            "[{}] Deleted folder '{}' ({} email(s) removed locally)",
            account.email, folder.display_name, deleted
        ),
    );
    Ok(())
}

/// Move one message between the inbox and a custom folder (either direction,
/// or folder → folder). Provider-first; the local row is then re-keyed to the
/// message's new provider id so all AI state survives. When the provider
/// cannot report the new id, the row keeps its old id and only changes
/// mailbox — the id is opaque to the UI, and IMAP attachment bytes are stored
/// inline, so nothing user-facing breaks.
pub async fn move_email(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    email_id: &str,
    target_mailbox: &str,
) -> Result<()> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email not found: {email_id}")))?;
    if email.account_id != account.id {
        return Err(AppError::InvalidInput(
            "Email does not belong to this account".to_string(),
        ));
    }
    if email.mailbox != "inbox" && !email.mailbox.starts_with("folder:") {
        return Err(AppError::InvalidInput(
            "Only inbox and folder messages can be moved".to_string(),
        ));
    }

    let target = match target_mailbox {
        "inbox" => MoveTarget::Inbox,
        other => match other.strip_prefix("folder:") {
            Some(path) if !path.is_empty() => {
                // The target must be a known custom folder of this account.
                let folder_id = format!("{}:{}", account.id, path);
                require_custom_folder(db, &account.id, &folder_id)?;
                MoveTarget::Folder(path.to_string())
            }
            _ => return Err(AppError::InvalidInput(format!("Invalid move target: {target_mailbox}"))),
        },
    };
    let new_mailbox = target.mailbox_value();
    if email.mailbox == new_mailbox {
        return Ok(());
    }

    let moved_ref = email_provider
        .move_message(&email.id, email.message_id.as_deref(), &target)
        .await?;

    let new_id = match moved_ref {
        Some(r) => r.id,
        None => {
            logger::log(
                "debug",
                "sync",
                format!(
                    "[{}] Move succeeded but the new message id is unknown; keeping local id",
                    account.email
                ),
            );
            email.id.clone()
        }
    };
    // If a concurrent sync already ingested the moved message under its new
    // id, re-keying would collide — drop our stale source row instead.
    if new_id != email.id && db.get_email(&new_id)?.is_some() {
        db.hard_delete_email(&email.id)?;
    } else {
        db.migrate_email_id(&email.id, &new_id, &new_mailbox)?;
    }

    logger::log(
        "info",
        "sync",
        format!("[{}] Moved email to {}", account.email, new_mailbox),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;
    use crate::sync::provider::{EmailCategory, FakeEmailProvider, FakeFolderOp};

    fn imap_account(id: &str) -> Account {
        Account {
            id: id.to_string(),
            provider: "imap".to_string(),
            email: format!("{id}@example.com"),
            name: "Test".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    fn test_db(account_id: &str) -> Arc<Database> {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account(account_id);
        Arc::new(db)
    }

    fn seed_folder(db: &Arc<Database>, account_id: &str, path: &str) {
        db.upsert_folder(
            account_id,
            &FolderUpsert {
                server_path: path.to_string(),
                display_name: decode_imap_utf7(path),
                role: FolderRole::Custom,
                delimiter: Some(".".to_string()),
            },
        )
        .unwrap();
    }

    /// A fake provider whose server-side LIST already knows `paths`.
    fn provider_with_folders(paths: &[&str]) -> FakeEmailProvider {
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folders(
            paths
                .iter()
                .map(|p| crate::sync::folder_plan::ListedFolder {
                    raw_name: (*p).to_string(),
                    delimiter: Some(".".to_string()),
                    attributes: vec![],
                })
                .collect(),
        );
        provider
    }

    fn email(id: &str, account: &str, mailbox: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: account.to_string(),
            thread_id: format!("t-{id}"),
            message_id: Some(format!("<{id}@example.com>")),
            subject: "s".to_string(),
            sender: "Sender".to_string(),
            sender_email: "sender@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "body".to_string(),
            snippet: "body".to_string(),
            timestamp: 1_000,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
        }
    }

    // ── create ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_folder_creates_on_server_and_persists_row() {
        let db = test_db("acc-1");
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        let folder = create_folder(&db, &imap_account("acc-1"), &provider, "Patienten")
            .await
            .unwrap();

        assert_eq!(
            provider.folder_ops(),
            vec![FakeFolderOp::Create("Patienten".to_string())]
        );
        assert_eq!(folder.server_path, "Patienten");
        assert_eq!(folder.role, "custom");
        assert!(db.get_folder("acc-1", "acc-1:Patienten").unwrap().is_some());
    }

    #[tokio::test]
    async fn create_folder_nests_under_inbox_when_layout_does() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "INBOX.Alt");
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        let folder = create_folder(&db, &imap_account("acc-1"), &provider, "Neu")
            .await
            .unwrap();

        assert_eq!(folder.server_path, "INBOX.Neu");
        assert_eq!(
            provider.folder_ops(),
            vec![FakeFolderOp::Create("INBOX.Neu".to_string())]
        );
    }

    #[tokio::test]
    async fn create_folder_encodes_non_ascii_and_decodes_display_name() {
        let db = test_db("acc-1");
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        let folder = create_folder(&db, &imap_account("acc-1"), &provider, "Verträge")
            .await
            .unwrap();

        assert_eq!(folder.server_path, "Vertr&AOQ-ge");
        assert_eq!(folder.display_name, "Verträge");
    }

    #[tokio::test]
    async fn create_folder_rejects_invalid_and_duplicate_names() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "INBOX.Patienten");
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let account = imap_account("acc-1");

        assert!(create_folder(&db, &account, &provider, "  ").await.is_err());
        assert!(
            create_folder(&db, &account, &provider, "a.b").await.is_err(),
            "delimiter char"
        );
        // Same display name as the existing folder's last segment (layout
        // nests under INBOX so the composed path collides too).
        assert!(create_folder(&db, &account, &provider, "patienten").await.is_err());
        assert!(provider.folder_ops().is_empty(), "server never touched on rejects");
    }

    #[tokio::test]
    async fn create_folder_provider_failure_leaves_db_untouched() {
        let db = test_db("acc-1");
        // BareProvider-style failure: default trait impl errors.
        struct FailingProvider;
        #[async_trait::async_trait]
        impl EmailProvider for FailingProvider {
            async fn get_profile(&self) -> Result<(String, String)> {
                Ok((String::new(), String::new()))
            }
            async fn list_messages(
                &self,
                _m: u32,
                _p: Option<&str>,
                _a: Option<i64>,
                _b: Option<i64>,
                _l: Option<&str>,
            ) -> Result<(Vec<crate::sync::provider::MessageRef>, Option<String>)> {
                Ok((Vec::new(), None))
            }
            async fn get_message(
                &self,
                id: &str,
            ) -> Result<(Email, EmailCategory, Vec<crate::sync::provider::AttachmentInfo>)> {
                Err(AppError::NotFound(id.to_string()))
            }
            async fn send_reply(
                &self,
                _f: &str,
                _t: &[String],
                _c: &[String],
                _th: &str,
                _o: Option<&str>,
                _s: &str,
                _b: &crate::sync::provider::EmailBody,
                _a: &[crate::sync::provider::EmailAttachment],
            ) -> Result<crate::sync::provider::SentMessageMeta> {
                Ok(Default::default())
            }
            async fn send_new_email(
                &self,
                _f: &str,
                _t: &[String],
                _c: &[String],
                _s: &str,
                _b: &crate::sync::provider::EmailBody,
                _a: &[crate::sync::provider::EmailAttachment],
            ) -> Result<crate::sync::provider::SentMessageMeta> {
                Ok(Default::default())
            }
            async fn fetch_attachment_bytes(&self, _m: &str, _a: &str) -> Result<Vec<u8>> {
                Ok(Vec::new())
            }
        }

        let result = create_folder(&db, &imap_account("acc-1"), &FailingProvider, "Neu").await;

        assert!(result.is_err());
        assert!(db.list_folders("acc-1", None).unwrap().is_empty(), "no row persisted");
    }

    // ── rename ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rename_folder_migrates_row_emails_and_watermarks() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Kunden");
        let old_prefix = folder_email_id_prefix("acc-1", "Kunden");
        db.insert_emails_batch(&[email(&format!("{old_prefix}5"), "acc-1", "folder:Kunden")])
            .unwrap();
        let [old_fwd, _, _] = super::super::sync::custom_folder_pref_keys("acc-1", "Kunden");
        db.set_preference(&old_fwd, "12345").unwrap();
        let provider = provider_with_folders(&["Kunden"]);

        let renamed = rename_folder(&db, &imap_account("acc-1"), &provider, "acc-1:Kunden", "Klienten")
            .await
            .unwrap();

        assert_eq!(
            provider.folder_ops(),
            vec![FakeFolderOp::Rename("Kunden".to_string(), "Klienten".to_string())]
        );
        assert_eq!(renamed.server_path, "Klienten");
        assert!(db.get_folder("acc-1", "acc-1:Kunden").unwrap().is_none());

        // Email re-keyed to the new prefix + mailbox.
        let new_prefix = folder_email_id_prefix("acc-1", "Klienten");
        let migrated = db
            .get_email(&format!("{new_prefix}5"))
            .unwrap()
            .expect("re-keyed email");
        assert_eq!(migrated.mailbox, "folder:Klienten");

        // Watermark carried over, old key removed.
        let [new_fwd, _, _] = super::super::sync::custom_folder_pref_keys("acc-1", "Klienten");
        assert_eq!(db.get_preference(&new_fwd).unwrap().as_deref(), Some("12345"));
        assert!(db.get_preference(&old_fwd).unwrap().is_none());
    }

    #[tokio::test]
    async fn rename_folder_rejects_role_folders_and_unknown_ids() {
        let db = test_db("acc-1");
        db.upsert_folder(
            "acc-1",
            &FolderUpsert {
                server_path: "Gesendete Objekte".to_string(),
                display_name: "Gesendete Objekte".to_string(),
                role: FolderRole::Sent,
                delimiter: Some(".".to_string()),
            },
        )
        .unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let account = imap_account("acc-1");

        assert!(
            rename_folder(&db, &account, &provider, "acc-1:Gesendete Objekte", "X")
                .await
                .is_err(),
            "role folders are not user-renamable"
        );
        assert!(rename_folder(&db, &account, &provider, "acc-1:Nope", "X")
            .await
            .is_err());
        assert!(provider.folder_ops().is_empty());
    }

    #[tokio::test]
    async fn rename_folder_same_name_is_a_noop() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Kunden");
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        let folder = rename_folder(&db, &imap_account("acc-1"), &provider, "acc-1:Kunden", "Kunden")
            .await
            .unwrap();

        assert_eq!(folder.server_path, "Kunden");
        assert!(provider.folder_ops().is_empty(), "no server call for a no-op");
    }

    // ── delete ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_folder_removes_server_folder_local_emails_and_watermarks() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Alt");
        let prefix = folder_email_id_prefix("acc-1", "Alt");
        db.insert_emails_batch(&[email(&format!("{prefix}9"), "acc-1", "folder:Alt")])
            .unwrap();
        let [fwd, done, cursor] = super::super::sync::custom_folder_pref_keys("acc-1", "Alt");
        db.set_preference(&fwd, "1").unwrap();
        db.set_preference(&done, "1").unwrap();
        db.set_preference(&cursor, "1").unwrap();
        let provider = provider_with_folders(&["Alt"]);

        delete_folder(&db, &imap_account("acc-1"), &provider, "acc-1:Alt")
            .await
            .unwrap();

        assert_eq!(provider.folder_ops(), vec![FakeFolderOp::Delete("Alt".to_string())]);
        assert!(db.get_folder("acc-1", "acc-1:Alt").unwrap().is_none());
        assert!(
            db.get_email(&format!("{prefix}9")).unwrap().is_none(),
            "emails hard-deleted"
        );
        for key in [fwd, done, cursor] {
            assert!(db.get_preference(&key).unwrap().is_none(), "{key} cleaned up");
        }
    }

    // ── move ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn move_email_to_folder_rekeys_row_via_provider_ref() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        db.insert_emails_batch(&[email("acc-1::10", "acc-1", "inbox")]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(email("acc-1::10", "acc-1", "inbox"), EmailCategory::Primary, vec![]);

        move_email(&db, &imap_account("acc-1"), &provider, "acc-1::10", "folder:Archiv")
            .await
            .unwrap();

        assert_eq!(
            provider.folder_ops(),
            vec![FakeFolderOp::Move {
                message_id: "acc-1::10".to_string(),
                mailbox_value: "folder:Archiv".to_string(),
            }]
        );
        // The fake reports the same id back; the row's mailbox migrates.
        let moved = db.get_email("acc-1::10").unwrap().expect("row kept");
        assert_eq!(moved.mailbox, "folder:Archiv");
    }

    #[tokio::test]
    async fn move_email_back_to_inbox() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        db.insert_emails_batch(&[email("acc-1::11", "acc-1", "folder:Archiv")])
            .unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(
            email("acc-1::11", "acc-1", "folder:Archiv"),
            EmailCategory::Primary,
            vec![],
        );

        move_email(&db, &imap_account("acc-1"), &provider, "acc-1::11", "inbox")
            .await
            .unwrap();

        assert_eq!(db.get_email("acc-1::11").unwrap().unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn move_email_with_rekeying_provider_migrates_to_new_id() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        db.insert_emails_batch(&[email("acc-1::14", "acc-1", "inbox")]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(email("acc-1::14", "acc-1", "inbox"), EmailCategory::Primary, vec![]);
        let new_id = format!("{}77", folder_email_id_prefix("acc-1", "Archiv"));
        provider.set_move_result(Some(crate::sync::provider::MessageRef {
            id: new_id.clone(),
            thread_id: String::new(),
        }));

        move_email(&db, &imap_account("acc-1"), &provider, "acc-1::14", "folder:Archiv")
            .await
            .unwrap();

        assert!(db.get_email("acc-1::14").unwrap().is_none(), "old id gone");
        let moved = db.get_email(&new_id).unwrap().expect("row re-keyed");
        assert_eq!(moved.mailbox, "folder:Archiv");
        assert_eq!(moved.subject, "s", "local content preserved without re-download");
    }

    #[tokio::test]
    async fn move_email_with_unknown_new_id_keeps_id_and_updates_mailbox() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        db.insert_emails_batch(&[email("acc-1::15", "acc-1", "inbox")]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(email("acc-1::15", "acc-1", "inbox"), EmailCategory::Primary, vec![]);
        provider.set_move_result(None);

        move_email(&db, &imap_account("acc-1"), &provider, "acc-1::15", "folder:Archiv")
            .await
            .unwrap();

        let moved = db.get_email("acc-1::15").unwrap().expect("row kept under old id");
        assert_eq!(moved.mailbox, "folder:Archiv");
    }

    #[tokio::test]
    async fn move_email_drops_stale_source_when_target_id_already_ingested() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        let new_id = format!("{}88", folder_email_id_prefix("acc-1", "Archiv"));
        db.insert_emails_batch(&[
            email("acc-1::16", "acc-1", "inbox"),
            // A concurrent sync already ingested the moved message.
            email(&new_id, "acc-1", "folder:Archiv"),
        ])
        .unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(email("acc-1::16", "acc-1", "inbox"), EmailCategory::Primary, vec![]);
        provider.set_move_result(Some(crate::sync::provider::MessageRef {
            id: new_id.clone(),
            thread_id: String::new(),
        }));

        move_email(&db, &imap_account("acc-1"), &provider, "acc-1::16", "folder:Archiv")
            .await
            .unwrap();

        assert!(db.get_email("acc-1::16").unwrap().is_none(), "stale source dropped");
        assert!(db.get_email(&new_id).unwrap().is_some(), "ingested target row kept");
    }

    #[tokio::test]
    async fn move_email_rejects_bad_sources_and_targets() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        db.insert_emails_batch(&[
            email("acc-1::SENT::1", "acc-1", "sent"),
            email("acc-1::12", "acc-1", "inbox"),
        ])
        .unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let account = imap_account("acc-1");

        assert!(
            move_email(&db, &account, &provider, "acc-1::SENT::1", "folder:Archiv")
                .await
                .is_err(),
            "sent messages cannot be moved"
        );
        assert!(
            move_email(&db, &account, &provider, "acc-1::12", "folder:Nope")
                .await
                .is_err(),
            "unknown target folder"
        );
        assert!(
            move_email(&db, &account, &provider, "acc-1::12", "spam").await.is_err(),
            "role mailboxes are not move targets"
        );
        assert!(provider.folder_ops().is_empty());
    }

    #[tokio::test]
    async fn move_email_same_mailbox_is_a_noop() {
        let db = test_db("acc-1");
        seed_folder(&db, "acc-1", "Archiv");
        db.insert_emails_batch(&[email("acc-1::13", "acc-1", "folder:Archiv")])
            .unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        move_email(&db, &imap_account("acc-1"), &provider, "acc-1::13", "folder:Archiv")
            .await
            .unwrap();

        assert!(provider.folder_ops().is_empty());
    }
}
