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
use crate::sync::draft_plan::{plan_draft_sync, DraftSyncAction, LocalDraftState, UpstreamDraftState};
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
/// in the send path. The HTML body is sanitized here, at the service boundary,
/// so neither the stored draft nor the copy pushed to the provider carries
/// markup the compose editor could never have produced.
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
        body_html: input.body_html.as_deref().map(super::sanitize_outgoing_html),
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

/// Serializes draft pushes. The composer's save and the sync can both decide to
/// push the same draft; without this, two pushes of a draft that has no
/// provider id yet would each create one upstream and leave a duplicate.
static DRAFT_PUSH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Push a dirty local draft to the provider's Drafts folder and mark the
/// revision that was sent as clean. A no-op when the draft is already clean or
/// no longer exists (another push got there first, or it was sent/deleted).
///
/// `gone_upstream` is the provider draft id the caller already knows is absent
/// from the provider's listing; a draft still linked to it is created afresh
/// rather than updated. An update the provider answers with not-found (the
/// draft was sent or deleted from another device since) falls back to a create
/// as well. Either way the stale id is replaced by the new one.
async fn push_draft(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    draft_id: &str,
    gone_upstream: Option<&str>,
) -> Result<()> {
    let _pushing = DRAFT_PUSH_LOCK.lock().await;
    let Some((draft, revision)) = db.draft_for_push(draft_id)? else {
        return Ok(());
    };
    if revision == 0 {
        return Ok(());
    }

    let attachments = load_attachments(&draft.attachments)?;
    let body = draft_body(&draft.body, draft.body_html.as_deref());
    let from = account.email.as_str();
    let (to, cc, subject) = (&draft.to_addresses, &draft.cc_addresses, draft.subject.as_str());

    let linked = draft
        .provider_draft_id
        .as_deref()
        .filter(|id| Some(*id) != gone_upstream);
    let updated = match linked {
        Some(existing) => match provider
            .update_draft(existing, from, to, cc, subject, &body, &attachments)
            .await
        {
            Ok(id) => Some(id),
            Err(AppError::NotFound(_)) => None,
            Err(e) => return Err(e),
        },
        None => None,
    };
    let provider_id = match updated {
        Some(id) => id,
        None => {
            provider
                .create_draft(from, to, cc, subject, &body, &attachments)
                .await?
        }
    };
    db.mark_draft_pushed(&draft.id, &provider_id, revision)
}

/// Save a composed draft locally and, when the account's provider supports
/// server-side drafts, push it to the Drafts folder (create or update) and
/// store the returned provider draft id. Returns the persisted draft.
///
/// The local save marks the draft dirty in the same write. A push that cannot
/// happen now (no provider) or fails leaves it dirty, and the next
/// [`pull_provider_drafts`] pushes it; a failed push is still returned as an
/// error so the caller can tell the user the provider copy is behind.
pub async fn compose_draft(
    db: &Arc<Database>,
    account: &Account,
    input: ComposeInput,
    provider: Option<&dyn EmailProvider>,
) -> Result<Draft> {
    // An existing draft id must be this account's draft, or the upsert would
    // overwrite another account's draft and push it through this provider.
    if let Some(draft_id) = input.draft_id.as_deref() {
        if db.get_draft(draft_id)?.is_some() {
            crate::services::ownership::draft_in_account(db, &account.id, draft_id)?;
        }
    }
    if let Some(email_id) = input.email_id.as_deref() {
        crate::services::ownership::email_in_account(db, &account.id, email_id)?;
    }
    let plan = plan_compose(&input);
    let saved = db.save_user_draft(&plan.save_req)?;

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
            push_draft(db, account, provider, &saved.id, None).await?;
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
    // Sanitized again here: a draft pulled from the provider's Drafts folder is
    // stored with the provider's raw HTML, which never went through compose.
    let body = match draft.body_html.as_deref() {
        Some(html) => EmailBody::with_html(&draft.body, super::sanitize_outgoing_html(html)),
        None => EmailBody::plain(&draft.body),
    };

    // A draft started from a message is a reply, and has to be sent as one.
    // Routing every draft through `send_new_email_with_provider` dropped
    // In-Reply-To, References and Gmail's `threadId` entirely, so saving a
    // reply as a draft and sending it later silently started a new thread —
    // the same broken-threading symptom as a directly-sent reply, only total.
    //
    // The draft's own subject wins: the user may have edited it, and
    // `drafts.email_id` has a foreign key to `emails(id)`, so it always points
    // at a row that still exists.
    match draft.email_id.as_deref() {
        Some(email_id) => {
            super::send::send_reply_with_provider(
                db,
                email_id,
                &body,
                Some(&account.id),
                Some(draft.to_addresses.clone()),
                Some(draft.cc_addresses.clone()),
                Some(&draft.subject),
                attachments,
                provider,
            )
            .await?;
        }
        None => {
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
        }
    }

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
    // A draft that is already gone deletes as a no-op; one that belongs to
    // another account is refused before its provider id reaches this
    // account's provider.
    if db.get_draft(draft_id)?.is_some() {
        let draft = crate::services::ownership::draft_in_account(db, &account.id, draft_id)?;
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

/// Minimum gap between two **on-demand** draft pulls for one account.
///
/// The sync's own pull is unaffected. This only bounds the UI triggers —
/// opening the Drafts screen and opening a draft from it fire a second apart,
/// and one listing call covers both.
pub const DRAFT_REFRESH_COOLDOWN_SECS: i64 = 10;

/// Pure: may an on-demand pull hit the provider now?
///
/// A clock that moved backwards counts as due — parking an account in a
/// cooldown it can never leave would be worse than one extra listing call.
pub fn draft_refresh_due(last_pull_at: Option<i64>, now: i64) -> bool {
    match last_pull_at {
        Some(last) if now >= last => now - last >= DRAFT_REFRESH_COOLDOWN_SECS,
        _ => true,
    }
}

/// When the last on-demand pull ran, per account. Process-local and
/// intentionally not persisted: a restart may spend one extra listing call.
static LAST_ON_DEMAND_PULL: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// On-demand draft pull for UI triggers (opening the Drafts screen, opening a
/// draft), throttled per account by [`DRAFT_REFRESH_COOLDOWN_SECS`]. Returns
/// the number of drafts whose content was read, or `0` when throttled.
///
/// `now` is passed in rather than read here so the throttle is testable without
/// a global clock.
pub async fn refresh_provider_drafts(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    now: i64,
) -> Result<usize> {
    let account_id = account.id.as_str();
    {
        let mut last = LAST_ON_DEMAND_PULL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !draft_refresh_due(last.get(account_id).copied(), now) {
            return Ok(0);
        }
        // Stamp before the call, not after: two triggers racing must not both
        // get through while the first is still in flight.
        last.insert(account_id.to_string(), now);
    }
    pull_provider_drafts(db, account, provider).await
}

/// Reconcile the local drafts of an account with the provider's Drafts folder,
/// one [`plan_draft_sync`] decision per draft: pull what changed upstream, push
/// what changed here, re-create a dirty draft whose provider copy is gone, and
/// prune clean drafts that were sent or deleted elsewhere. Returns the number
/// of drafts pulled. Best-effort — the caller (sync) logs and continues on
/// error.
///
/// A draft with unpushed local edits is never pruned or overwritten: when both
/// sides changed, the local draft wins and is pushed.
pub async fn pull_provider_drafts(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
) -> Result<usize> {
    let account_id = account.id.as_str();
    // Snapshot the local rows before listing. A draft pushed while the listing
    // is in flight is then simply not part of this pass, instead of looking
    // like a linked draft that is missing upstream.
    let locals = db.draft_sync_states(account_id)?;
    // Hand the provider what we already have so it can skip re-reading drafts
    // that have not changed upstream. At steady state this makes the pass one
    // listing call and zero content reads.
    let known = db.provider_draft_change_tokens(account_id)?;
    let pull = provider.list_drafts(&known).await?;

    let changed: std::collections::HashMap<&str, &crate::models::ProviderDraft> = pull
        .changed
        .iter()
        .map(|pd| (pd.provider_draft_id.as_str(), pd))
        .collect();
    let present: std::collections::HashSet<&str> = pull.present_ids.iter().map(String::as_str).collect();

    let mut pulled = 0usize;
    let mut to_prune = Vec::new();
    let mut linked_locally = std::collections::HashSet::new();
    for local in &locals {
        let provider_id = local.provider_draft_id.as_deref();
        let upstream = match provider_id {
            Some(id) if changed.contains_key(id) => UpstreamDraftState::Changed,
            Some(id) if present.contains(id) => UpstreamDraftState::Unchanged,
            _ => UpstreamDraftState::Absent,
        };
        linked_locally.extend(provider_id);
        let state = LocalDraftState {
            linked: provider_id.is_some(),
            dirty: local.dirty,
        };
        let action = plan_draft_sync(Some(state), upstream);
        match action {
            DraftSyncAction::Push | DraftSyncAction::Recreate => {
                let gone_upstream = if action == DraftSyncAction::Recreate {
                    provider_id
                } else {
                    None
                };
                // One draft that cannot be pushed (an attachment file that was
                // moved, a provider refusal) must not stop the others. It
                // stays dirty, so the next pass tries again.
                if let Err(e) = push_draft(db, account, provider, &local.id, gone_upstream).await {
                    crate::services::logger::log(
                        "warn",
                        "drafts",
                        format!("Could not push draft {} to the provider: {e}", local.id),
                    );
                }
            }
            DraftSyncAction::Pull => {
                if let Some(pd) = provider_id.and_then(|id| changed.get(id)) {
                    db.upsert_provider_draft(account_id, pd)?;
                    pulled += 1;
                }
            }
            DraftSyncAction::Prune => to_prune.push(local.id.clone()),
            DraftSyncAction::Keep => {}
        }
    }

    // Drafts written on another device that this one has never stored.
    for pd in &pull.changed {
        if linked_locally.contains(pd.provider_draft_id.as_str()) {
            continue;
        }
        if plan_draft_sync(None, UpstreamDraftState::Changed) == DraftSyncAction::Pull {
            db.upsert_provider_draft(account_id, pd)?;
            pulled += 1;
        }
    }

    db.prune_provider_drafts(account_id, &to_prune)?;
    Ok(pulled)
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

    const UNSAFE_HTML: &str =
        "<p>Hi</p><script>alert(1)</script><img src=\"https://example.com/a.png\" onerror=\"alert(2)\">";

    fn assert_html_is_sanitized(html: &str) {
        assert!(html.contains("<p>Hi</p>"), "safe markup must survive: {html}");
        assert!(!html.contains("<script"), "script must be stripped: {html}");
        assert!(!html.contains("onerror"), "event handlers must be stripped: {html}");
    }

    #[test]
    fn plan_compose_sanitizes_the_html_body() {
        let mut inp = input("a1", "Hi", "hello");
        inp.body_html = Some(UNSAFE_HTML.to_string());
        let plan = plan_compose(&inp);
        assert_html_is_sanitized(plan.save_req.body_html.as_deref().expect("html kept"));
    }

    #[tokio::test]
    async fn compose_draft_pushes_sanitized_html_to_the_provider() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");

        let mut inp = input("a1", "Hi", "hello");
        inp.body_html = Some(UNSAFE_HTML.to_string());
        let draft = compose_draft(&db, &account, inp, Some(&provider))
            .await
            .expect("compose");

        assert_html_is_sanitized(draft.body_html.as_deref().expect("stored html"));
        let pushed = provider.provider_drafts();
        assert_html_is_sanitized(pushed[0].body_html.as_deref().expect("pushed html"));
    }

    /// A draft pulled from the provider's Drafts folder is stored with the
    /// provider's raw HTML; sending it must not forward that HTML unsanitized.
    #[tokio::test]
    async fn send_draft_sanitizes_stored_html_before_sending() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "imap");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");
        let draft = db
            .save_draft(&SaveDraftRequest {
                id: None,
                email_id: None,
                account_id: "a1".to_string(),
                to_addresses: vec!["dest@example.com".to_string()],
                cc_addresses: Vec::new(),
                subject: "Pulled".to_string(),
                body: "hello".to_string(),
                body_html: Some(UNSAFE_HTML.to_string()),
                provider_draft_id: None,
                attachments: None,
            })
            .expect("save raw draft");

        send_draft(&db, &account, &draft.id, &provider).await.expect("send");

        let sent = provider.sent();
        assert_html_is_sanitized(sent[0].body.html.as_deref().expect("html sent"));
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
    async fn compose_draft_refuses_another_accounts_draft_id() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let owner = seed_account(&db, "a1", "imap");
        let other = seed_account(&db, "a2", "imap");
        let theirs = compose_draft(&db, &other, input("a2", "Theirs", "kept"), None)
            .await
            .expect("compose");

        let mut overwrite = input("a1", "Mine", "replaced");
        overwrite.draft_id = Some(theirs.id.clone());
        let result = compose_draft(&db, &owner, overwrite, None).await;

        assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
        let kept = db.get_draft(&theirs.id).expect("get").expect("still there");
        assert_eq!((kept.account_id.as_str(), kept.body.as_str()), ("a2", "kept"));
    }

    #[tokio::test]
    async fn compose_draft_refuses_a_reply_to_another_accounts_email() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let owner = seed_account(&db, "a1", "imap");
        seed_account(&db, "a2", "imap");
        db.connection()
            .execute(
                "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, recipients_json, snippet, timestamp, created_at) \
                 VALUES ('their-email', 'a2', 't', 's', 'S', 's@example.com', '[]', '', 0, 0)",
                [],
            )
            .expect("seed email");

        let mut reply = input("a1", "Re: s", "body");
        reply.email_id = Some("their-email".to_string());
        let result = compose_draft(&db, &owner, reply, None).await;

        assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
    }

    #[tokio::test]
    async fn delete_draft_leaves_another_accounts_draft_alone() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let owner = seed_account(&db, "a1", "gmail");
        let other = seed_account(&db, "a2", "gmail");
        let owner_provider = FakeEmailProvider::new("a1@example.com", "A One");
        let other_provider = FakeEmailProvider::new("a2@example.com", "A Two");
        let theirs = compose_draft(&db, &other, input("a2", "Theirs", "body"), Some(&other_provider))
            .await
            .expect("compose");

        let result = delete_draft(&db, &owner, &theirs.id, Some(&owner_provider)).await;

        assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
        assert!(db.get_draft(&theirs.id).expect("get").is_some(), "local copy kept");
        assert_eq!(other_provider.provider_drafts().len(), 1, "provider copy kept");
    }

    #[tokio::test]
    async fn pull_provider_drafts_upserts_and_prunes() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Server draft".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
            provider_message_id: Some("msg-1".to_string()),
        });

        let pulled = pull_provider_drafts(&db, &account, &provider).await.expect("pull");
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
        let pulled2 = pull_provider_drafts(&db, &account, &provider).await.expect("pull2");
        assert_eq!(pulled2, 0);
        assert!(db.list_drafts("a1").expect("list").is_empty());
    }

    #[tokio::test]
    async fn re_pulling_unchanged_drafts_reads_no_content_and_keeps_them() {
        // Regression: the pull pass re-downloaded every draft in full on every
        // 60-second sync tick, because it had no way to spot an untouched one.
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Server draft".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
            provider_message_id: Some("msg-1".to_string()),
        });

        assert_eq!(
            pull_provider_drafts(&db, &account, &provider).await.expect("first"),
            1,
            "first pull reads the draft it has never seen"
        );
        assert_eq!(
            pull_provider_drafts(&db, &account, &provider).await.expect("second"),
            0,
            "unchanged draft must not be read again"
        );
        // Skipping the read must not make the draft look absent upstream.
        let drafts = db.list_drafts("a1").expect("list");
        assert_eq!(drafts.len(), 1, "a skipped draft must survive the prune pass");
        assert_eq!(drafts[0].subject, "Server draft");
    }

    #[test]
    fn an_account_never_pulled_on_demand_is_due() {
        assert!(draft_refresh_due(None, 1_000));
    }

    #[test]
    fn a_pull_inside_the_cooldown_is_not_due() {
        // Opening the Drafts screen and then a draft in it are two triggers a
        // second apart; one listing call must cover both.
        assert!(!draft_refresh_due(Some(1_000), 1_000));
        assert!(!draft_refresh_due(Some(1_000), 1_000 + DRAFT_REFRESH_COOLDOWN_SECS - 1));
    }

    #[test]
    fn a_pull_past_the_cooldown_is_due() {
        assert!(draft_refresh_due(Some(1_000), 1_000 + DRAFT_REFRESH_COOLDOWN_SECS));
    }

    #[test]
    fn a_clock_that_moved_backwards_is_due_rather_than_stuck() {
        // A backwards jump (NTP correction, DST-adjacent clock fiddling) must
        // not park the account in a cooldown it can never leave.
        assert!(draft_refresh_due(Some(5_000), 1_000));
    }

    #[tokio::test]
    async fn on_demand_refresh_pulls_once_then_waits_out_the_cooldown() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "cooldown-1", "gmail");
        let provider = FakeEmailProvider::new("cooldown-1@example.com", "A One");
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "First".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
            provider_message_id: Some("msg-1".to_string()),
        });

        assert_eq!(
            refresh_provider_drafts(&db, &account, &provider, 1_000)
                .await
                .expect("first"),
            1
        );

        // Something new upstream, but we are still inside the cooldown.
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-2".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Second".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_100),
            provider_message_id: Some("msg-2".to_string()),
        });
        assert_eq!(
            refresh_provider_drafts(&db, &account, &provider, 1_001)
                .await
                .expect("throttled"),
            0,
            "a second trigger seconds later must not hit the provider"
        );
        assert_eq!(db.list_drafts("cooldown-1").expect("list").len(), 1);

        assert_eq!(
            refresh_provider_drafts(&db, &account, &provider, 1_000 + DRAFT_REFRESH_COOLDOWN_SECS)
                .await
                .expect("after cooldown"),
            1,
            "past the cooldown the provider is consulted again"
        );
        assert_eq!(db.list_drafts("cooldown-1").expect("list").len(), 2);
    }

    #[tokio::test]
    async fn on_demand_refresh_cooldowns_are_per_account() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account_a = seed_account(&db, "cooldown-a", "gmail");
        let account_b = seed_account(&db, "cooldown-b", "gmail");
        let provider = FakeEmailProvider::new("a@example.com", "A");
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Shared".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
            provider_message_id: Some("msg-1".to_string()),
        });

        refresh_provider_drafts(&db, &account_a, &provider, 2_000)
            .await
            .expect("a");
        assert_eq!(
            refresh_provider_drafts(&db, &account_b, &provider, 2_000)
                .await
                .expect("b"),
            1,
            "one account's refresh must not silence another's"
        );
    }

    #[tokio::test]
    async fn a_draft_composed_here_then_edited_upstream_is_updated_in_place() {
        // The reported shape: the draft was created in EmailOps and pushed to
        // Gmail, so its local row has a provider_draft_id but no change token
        // yet (the push never learns the message id). Editing it in Gmail must
        // update that same row rather than leave the local copy stale.
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");

        let draft = compose_draft(&db, &account, input("a1", "Written here", "body one"), Some(&provider))
            .await
            .expect("compose");
        let provider_draft_id = draft.provider_draft_id.clone().expect("pushed to provider");
        assert!(
            db.provider_draft_change_tokens("a1").expect("tokens").is_empty(),
            "a pushed draft starts with no change token"
        );

        // Same draft id, edited upstream.
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: provider_draft_id.clone(),
            to_addresses: vec!["dest@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Edited in Gmail".to_string(),
            body: "body two".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_500),
            provider_message_id: Some("msg-2".to_string()),
        });

        assert_eq!(pull_provider_drafts(&db, &account, &provider).await.expect("pull"), 1);

        let drafts = db.list_drafts("a1").expect("list");
        assert_eq!(drafts.len(), 1, "updated in place, not duplicated");
        assert_eq!(drafts[0].id, draft.id, "same local row");
        assert_eq!(drafts[0].subject, "Edited in Gmail");
        assert_eq!(drafts[0].body, "body two");
    }

    #[tokio::test]
    async fn a_draft_edited_upstream_is_read_again() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let account = seed_account(&db, "a1", "gmail");
        let provider = FakeEmailProvider::new("a1@example.com", "A One");
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "Before".to_string(),
            body: "hi".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_000),
            provider_message_id: Some("msg-1".to_string()),
        });
        pull_provider_drafts(&db, &account, &provider).await.expect("first");

        // Same draft id, new change token — Gmail's behaviour on a re-save.
        provider.add_provider_draft(crate::models::ProviderDraft {
            provider_draft_id: "srv-1".to_string(),
            to_addresses: vec!["x@example.com".to_string()],
            cc_addresses: Vec::new(),
            subject: "After".to_string(),
            body: "hi again".to_string(),
            body_html: None,
            updated_at: Some(1_700_000_500),
            provider_message_id: Some("msg-2".to_string()),
        });

        assert_eq!(
            pull_provider_drafts(&db, &account, &provider).await.expect("second"),
            1,
            "a moved change token must trigger a fresh read"
        );
        let drafts = db.list_drafts("a1").expect("list");
        assert_eq!(drafts[0].subject, "After");
    }
}
