//! Compose / draft plumbing: build a draft from raw inputs, persist it,
//! optionally push it to the provider's Drafts folder, send it, and pull the
//! provider's drafts back on sync.
//!
//! Split into a **pure planner** ([`plan_compose`]) that turns raw inputs into a
//! [`SaveDraftRequest`] + resolved attachment records with zero I/O, and thin
//! executors ([`compose_draft`], [`send_draft`], [`pull_provider_drafts`]) that
//! do the DB writes, file reads, and provider calls.

use std::sync::Arc;

use base64::Engine;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Draft, DraftAttachment, DraftAttachmentInput, SaveDraftRequest};
use crate::sync::provider::{provider_supports_drafts, EmailAttachment, EmailBody, EmailProvider};

/// An attachment with filename + mime resolved from its path (still just a
/// reference — no bytes read yet).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAttachment {
    pub file_path: String,
    pub filename: String,
    pub mime_type: String,
}

/// The pure output of [`plan_compose`]: exactly what to persist. `attachments`
/// is `None` when the caller opted not to manage them (leave existing files
/// intact), `Some(list)` when it replaces them.
#[derive(Debug, Clone)]
pub struct ComposePlan {
    pub save_req: SaveDraftRequest,
    pub attachments: Option<Vec<ResolvedAttachment>>,
}

/// Inputs for composing/saving a draft.
pub struct ComposeInput {
    /// Existing draft id to update, or `None` to create a new one.
    pub draft_id: Option<String>,
    pub account_id: String,
    /// Set when this draft is a reply, so it links back to the inbound email.
    pub email_id: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub body_html: Option<String>,
    /// `None` = leave the draft's existing attachments untouched; `Some(list)`
    /// = replace them (empty clears).
    pub attachments: Option<Vec<DraftAttachmentInput>>,
}

/// Resolve a file path into (filename, mime_type) without touching disk.
/// Filename is the path's final component; mime is guessed from the extension.
fn resolve_attachment(input: &DraftAttachmentInput) -> ResolvedAttachment {
    let filename = input.filename.clone().unwrap_or_else(|| basename(&input.file_path));
    let mime_type = input
        .mime_type
        .clone()
        .unwrap_or_else(|| guess_mime(&filename).to_string());
    ResolvedAttachment {
        file_path: input.file_path.clone(),
        filename,
        mime_type,
    }
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Minimal extension→MIME map for the common attachment types; everything else
/// falls back to the generic binary type (providers still deliver it fine).
fn guess_mime(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Pure planner: build the persistable draft request + resolved attachment
/// records from raw compose inputs. No validation that would reject a partial
/// draft — drafts are allowed to be incomplete; recipient/subject guards live
/// in the send path.
pub fn plan_compose(input: &ComposeInput) -> ComposePlan {
    let attachments: Option<Vec<ResolvedAttachment>> = input
        .attachments
        .as_ref()
        .map(|list| list.iter().map(resolve_attachment).collect());
    let save_req = SaveDraftRequest {
        id: input.draft_id.clone(),
        email_id: input.email_id.clone(),
        account_id: input.account_id.clone(),
        to_addresses: input.to.clone(),
        cc_addresses: input.cc.clone(),
        subject: input.subject.clone(),
        body: input.body.clone(),
        body_html: input.body_html.clone(),
        // The provider link is preserved on the DB row via COALESCE; never
        // cleared by a plain re-save.
        provider_draft_id: None,
        attachments: input.attachments.clone(),
    };
    ComposePlan { save_req, attachments }
}

/// Read a resolved attachment's bytes and base64-encode them for the provider
/// send/draft payloads.
fn load_attachment(att: &DraftAttachment) -> Result<EmailAttachment> {
    let bytes = std::fs::read(&att.file_path)
        .map_err(|e| AppError::IoError(format!("Failed to read attachment {}: {e}", att.file_path)))?;
    Ok(EmailAttachment {
        filename: att.filename.clone(),
        mime_type: att.mime_type.clone(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        content_id: None,
        is_inline: false,
    })
}

fn load_attachments(drafts: &[DraftAttachment]) -> Result<Vec<EmailAttachment>> {
    drafts.iter().map(load_attachment).collect()
}

/// Build the footer-free [`EmailBody`] for a draft push. The footer is added
/// only when the draft is actually sent.
fn draft_body(body: &str, body_html: Option<&str>) -> EmailBody {
    match body_html {
        Some(html) => EmailBody::with_html(body, html).without_footer(),
        None => EmailBody::plain(body).without_footer(),
    }
}

/// Save a composed draft locally and, when the account's provider supports
/// server-side drafts, push it to the Drafts folder (create or update) and
/// store the returned provider draft id. Returns the persisted draft.
pub async fn compose_draft(
    db: &Arc<Database>,
    account: &Account,
    input: ComposeInput,
    provider: Option<&dyn EmailProvider>,
) -> Result<Draft> {
    let plan = plan_compose(&input);
    let saved = db.save_draft(&plan.save_req)?;

    // Persist attachment references (full swap) only when the caller manages
    // them — `None` leaves the existing files intact so a text-only auto-save
    // from the composer can't wipe a draft's attachments.
    if let Some(resolved) = &plan.attachments {
        let att_records: Vec<DraftAttachment> = resolved
            .iter()
            .map(|a| DraftAttachment {
                id: String::new(),
                draft_id: saved.id.clone(),
                file_path: a.file_path.clone(),
                filename: a.filename.clone(),
                mime_type: a.mime_type.clone(),
            })
            .collect();
        db.replace_draft_attachments(&saved.id, &att_records)?;
    }

    // Push to the provider when supported, using the draft's *current* persisted
    // attachments (which may pre-date this save when `attachments` was `None`).
    if provider_supports_drafts(&account.provider) {
        if let Some(provider) = provider {
            let current = db.list_draft_attachments(&saved.id)?;
            let email_atts = load_attachments(&current)?;
            let body = draft_body(&input.body, input.body_html.as_deref());
            let provider_id = match saved.provider_draft_id.as_deref() {
                Some(existing) => {
                    provider
                        .update_draft(
                            existing,
                            &account.email,
                            &input.to,
                            &input.cc,
                            &input.subject,
                            &body,
                            &email_atts,
                        )
                        .await?
                }
                None => {
                    provider
                        .create_draft(&account.email, &input.to, &input.cc, &input.subject, &body, &email_atts)
                        .await?
                }
            };
            db.set_provider_draft_id(&saved.id, Some(&provider_id))?;
        }
    }

    // Re-fetch so the returned draft carries the provider id + attachments.
    db.get_draft(&saved.id)?
        .ok_or_else(|| AppError::NotFound(format!("Draft {} vanished after save", saved.id)))
}

/// Send a saved draft: deliver it via the provider, then delete it locally and
/// (when linked) from the provider's Drafts folder.
pub async fn send_draft(
    db: &Arc<Database>,
    account: &Account,
    draft_id: &str,
    provider: &dyn EmailProvider,
) -> Result<()> {
    let draft = db
        .get_draft(draft_id)?
        .ok_or_else(|| AppError::NotFound(format!("Draft {draft_id} not found")))?;
    if draft.account_id != account.id {
        return Err(AppError::InvalidInput(
            "Draft does not belong to the given account".to_string(),
        ));
    }

    let attachments = load_attachments(&draft.attachments)?;
    // Footer-free body; `send_new_email_with_provider` appends the footer once.
    let body = match draft.body_html.as_deref() {
        Some(html) => EmailBody::with_html(&draft.body, html),
        None => EmailBody::plain(&draft.body),
    };

    super::send::send_new_email_with_provider(
        db,
        &account.id,
        draft.to_addresses.clone(),
        draft.cc_addresses.clone(),
        &draft.subject,
        &body,
        attachments,
        provider,
    )
    .await?;

    // Best-effort cleanup of the provider-side draft; a failure here must not
    // make a successful send look failed.
    if let Some(provider_id) = draft.provider_draft_id.as_deref() {
        if provider_supports_drafts(&account.provider) {
            if let Err(e) = provider.delete_draft(provider_id).await {
                crate::services::logger::log(
                    "debug",
                    "drafts",
                    format!("Sent draft but could not remove provider copy {provider_id}: {e}"),
                );
            }
        }
    }
    db.delete_draft(draft_id, &account.id)?;
    Ok(())
}

/// Delete a draft locally and, when it is linked to a provider draft, from the
/// provider's Drafts folder too. Provider deletion is best-effort so a network
/// hiccup never blocks the local delete. `provider` may be `None` (offline / no
/// provider built) — the local row is still removed.
pub async fn delete_draft(
    db: &Arc<Database>,
    account: &Account,
    draft_id: &str,
    provider: Option<&dyn EmailProvider>,
) -> Result<()> {
    if let Some(draft) = db.get_draft(draft_id)? {
        if let (Some(provider_id), Some(provider)) = (draft.provider_draft_id.as_deref(), provider) {
            if provider_supports_drafts(&account.provider) {
                if let Err(e) = provider.delete_draft(provider_id).await {
                    crate::services::logger::log(
                        "debug",
                        "drafts",
                        format!("Deleted draft locally but not on provider ({provider_id}): {e}"),
                    );
                }
            }
        }
    }
    db.delete_draft(draft_id, &account.id)
}

/// Pull the provider's drafts into the local table, keyed by provider draft id,
/// and prune local provider-linked drafts that no longer exist upstream.
/// Returns the number of drafts pulled. Best-effort — the caller (sync) logs
/// and continues on error.
pub async fn pull_provider_drafts(db: &Arc<Database>, account_id: &str, provider: &dyn EmailProvider) -> Result<usize> {
    let provider_drafts = provider.list_drafts().await?;
    let mut keep_ids = Vec::with_capacity(provider_drafts.len());
    for pd in &provider_drafts {
        db.upsert_provider_draft(account_id, pd)?;
        keep_ids.push(pd.provider_draft_id.clone());
    }
    db.prune_provider_drafts(account_id, &keep_ids)?;
    Ok(provider_drafts.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::provider::FakeEmailProvider;

    fn seed_account(db: &Database, id: &str, provider: &str) -> Account {
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, ?2, ?3, ?3, 0, 0, 1)",
                rusqlite::params![id, provider, format!("{id}@example.com")],
            )
            .expect("seed account");
        db.get_account(id).expect("get account").expect("account present")
    }

    fn input(account_id: &str, subject: &str, body: &str) -> ComposeInput {
        ComposeInput {
            draft_id: None,
            account_id: account_id.to_string(),
            email_id: None,
            to: vec!["dest@example.com".to_string()],
            cc: Vec::new(),
            subject: subject.to_string(),
            body: body.to_string(),
            body_html: None,
            attachments: None,
        }
    }

    #[test]
    fn plan_compose_resolves_filename_and_mime_from_path() {
        let mut inp = input("a1", "Hi", "hello");
        inp.attachments = Some(vec![
            DraftAttachmentInput {
                file_path: "/tmp/dir/report.PDF".to_string(),
                filename: None,
                mime_type: None,
            },
            DraftAttachmentInput {
                file_path: "/weird/path/data".to_string(),
                filename: Some("custom.bin".to_string()),
                mime_type: Some("application/x-thing".to_string()),
            },
        ]);
        let plan = plan_compose(&inp);
        assert_eq!(plan.save_req.subject, "Hi");
        let atts = plan.attachments.as_ref().expect("attachments managed");
        assert_eq!(atts[0].filename, "report.PDF");
        assert_eq!(atts[0].mime_type, "application/pdf");
        assert_eq!(atts[1].filename, "custom.bin");
        assert_eq!(atts[1].mime_type, "application/x-thing");
    }

    #[tokio::test]
    async fn compose_draft_none_attachments_preserves_existing_files() {
        // Regression: a text-only auto-save (attachments: None) must not wipe the
        // files a prior save (e.g. CLI compose) attached to the draft.
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "imap");

        let mut first = input("a1", "With file", "body");
        first.attachments = Some(vec![DraftAttachmentInput {
            file_path: "/tmp/report.pdf".to_string(),
            filename: None,
            mime_type: None,
        }]);
        let created = compose_draft(&db, &account, first, None).await.expect("compose");
        assert_eq!(created.attachments.len(), 1);

        // Re-save the same draft with attachments: None (text-only edit).
        let mut edit = input("a1", "Edited subject", "body");
        edit.draft_id = Some(created.id.clone());
        edit.attachments = None;
        let edited = compose_draft(&db, &account, edit, None).await.expect("re-compose");
        assert_eq!(edited.subject, "Edited subject");
        assert_eq!(edited.attachments.len(), 1, "attachments must survive a None save");
    }

    #[tokio::test]
    async fn compose_draft_pushes_to_provider_and_stores_id() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");

        let draft = compose_draft(&db, &account, input("a1", "First", "body one"), Some(&provider))
            .await
            .expect("compose");
        assert!(draft.provider_draft_id.is_some(), "should store provider id");
        assert_eq!(provider.provider_drafts().len(), 1);

        // Re-saving the same draft updates the existing provider draft in place.
        let mut second = input("a1", "First edited", "body two");
        second.draft_id = Some(draft.id.clone());
        let updated = compose_draft(&db, &account, second, Some(&provider))
            .await
            .expect("update");
        assert_eq!(updated.provider_draft_id, draft.provider_draft_id);
        assert_eq!(provider.provider_drafts().len(), 1, "update, not a second create");
        assert_eq!(provider.provider_drafts()[0].subject, "First edited");
    }

    #[tokio::test]
    async fn compose_draft_stays_local_for_unsupported_provider() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "imap");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");

        let draft = compose_draft(&db, &account, input("a1", "Local", "body"), Some(&provider))
            .await
            .expect("compose");
        assert!(draft.provider_draft_id.is_none(), "imap draft stays local");
        assert_eq!(provider.provider_drafts().len(), 0);
    }

    #[tokio::test]
    async fn send_draft_delivers_and_removes_both_copies() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");

        let draft = compose_draft(&db, &account, input("a1", "Send me", "the body"), Some(&provider))
            .await
            .expect("compose");
        assert_eq!(provider.provider_drafts().len(), 1);

        send_draft(&db, &account, &draft.id, &provider).await.expect("send");

        let sent = provider.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "Send me");
        assert_eq!(sent[0].to_emails, vec!["dest@example.com".to_string()]);
        // The send path leaves the footer enabled (real providers append it once
        // at MIME-build time); the raw draft body carries no footer itself.
        assert!(sent[0].body.append_footer, "send must keep the footer enabled");
        assert_eq!(sent[0].body.text, "the body");
        // Both the local and provider copies are gone.
        assert!(db.get_draft(&draft.id).expect("get").is_none());
        assert_eq!(provider.provider_drafts().len(), 0);
    }

    #[tokio::test]
    async fn delete_draft_removes_local_and_provider_copies() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");

        let draft = compose_draft(&db, &account, input("a1", "Bye", "body"), Some(&provider))
            .await
            .expect("compose");
        assert_eq!(provider.provider_drafts().len(), 1);

        delete_draft(&db, &account, &draft.id, Some(&provider))
            .await
            .expect("delete");
        assert!(db.get_draft(&draft.id).expect("get").is_none());
        assert_eq!(provider.provider_drafts().len(), 0, "provider copy removed too");
    }

    #[tokio::test]
    async fn pull_provider_drafts_upserts_and_prunes() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Server draft".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
        });

        let pulled = pull_provider_drafts(&db, "a1", &provider).await.expect("pull");
        assert_eq!(pulled, 1);
        let drafts = db.list_drafts("a1").expect("list");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].subject, "Server draft");
        assert_eq!(drafts[0].provider_draft_id.as_deref(), Some("srv-1"));
        assert_eq!(
            drafts[0].updated_at, 1_700_000_000,
            "pull carries the provider's date through to the local row"
        );

        // Remove it upstream → next pull prunes the local copy.
        provider.delete_draft("srv-1").await.expect("del");
        let pulled2 = pull_provider_drafts(&db, "a1", &provider).await.expect("pull2");
        assert_eq!(pulled2, 0);
        assert!(db.list_drafts("a1").expect("list").is_empty());
    }
}
