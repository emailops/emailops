//! Shared documents and sheets, kept in sync between EmailOps installs with
//! email as the only transport — no server, no cloud. See `MODULE.md`.
//!
//! Content is a Yjs CRDT ([`crdt`]): every install merges what it receives in
//! any order and converges. Changes travel as an `.eodoc` attachment
//! ([`envelope`]) on an ordinary message to the other participants, mailed in
//! the background once the user has shared or accepted the document — the
//! consent that makes these automatic sends acceptable (DECISIONS 2026-10-04
//! "Shared documents sync over email").

pub mod crdt;
pub mod envelope;
mod mail_text;
pub mod planner;

use std::sync::Arc;

use base64::Engine;
use serde::Serialize;

use crate::db::shared_docs::NewSharedDoc;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::shared_docs::{DocFolder, DocKind, DocStatus, DocVersion, SharedDoc};
use crate::models::{Account, Email};
use crate::services::emails::{ProviderAccess, ThreadAction, ThreadActionReport, ThreadRef};
use crate::services::logger;
use crate::services::outbox::OutboxProviders;
use crate::services::ownership::{doc_folder_in_account, shared_doc_in_account};
use crate::sync::provider::{
    provider_supports_mailbox_writes, AttachmentInfo, EmailAttachment, EmailBody, EmailProvider,
};
use envelope::{Envelope, Purpose};
use planner::{Arrival, FlushCandidate, KnownDoc};

/// The preference that turns the feature on. Experimental: off until the
/// user enables it, and while off nothing is ingested or mailed.
pub const SHARED_DOCS_ENABLED_PREF: &str = "shared_docs_enabled";

/// Emitted when documents changed outside the editor (a peer's changes
/// arrived, an invitation came in), so an open editor pulls the diff.
pub const SHARED_DOCS_CHANGED_EVENT: &str = "shared-docs-changed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedDocsChanged<'a> {
    doc_ids: &'a [String],
}

pub fn is_enabled(db: &Database) -> bool {
    db.get_preference(SHARED_DOCS_ENABLED_PREF)
        .ok()
        .flatten()
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn me(account: &Account) -> String {
    account.email.trim().to_lowercase()
}

fn notify_changed(doc_ids: &[String]) {
    if !doc_ids.is_empty() {
        crate::services::events::emit(SHARED_DOCS_CHANGED_EVENT, SharedDocsChanged { doc_ids });
    }
}

// ── Local operations ─────────────────────────────────────────────────────────

/// Create an empty document owned by `account`. Nothing is mailed until it is
/// shared.
pub fn create(db: &Database, account: &Account, kind: DocKind, title: &str, now: i64) -> Result<SharedDoc> {
    let id = uuid::Uuid::new_v4().to_string();
    db.insert_shared_doc(&NewSharedDoc {
        id: &id,
        account_id: &account.id,
        kind,
        title: &envelope::normalize_title(title)?,
        state: &crdt::empty_state(),
        status: DocStatus::Active,
        participants: &[me(account)],
        now,
    })?;
    logger::log(
        "info",
        "sync",
        format!("[{}] Created a shared {}", account.email, kind.as_str()),
    );
    reindex(db, &id)?;
    db.get_shared_doc(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {id} not found")))
}

/// The whole document, as one Yjs v1 update, base64.
pub fn state(db: &Database, account_id: &str, doc_id: &str) -> Result<String> {
    shared_doc_in_account(db, account_id, doc_id)?;
    let state = db
        .shared_doc_state(doc_id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
    Ok(b64().encode(state))
}

/// What an editor at `state_vector` (base64) has not seen yet, base64.
pub fn diff_since(db: &Database, account_id: &str, doc_id: &str, state_vector: &str) -> Result<String> {
    shared_doc_in_account(db, account_id, doc_id)?;
    let sv = b64()
        .decode(state_vector)
        .map_err(|_| AppError::InvalidInput("Not a valid shared document state vector".into()))?;
    let state = db
        .shared_doc_state(doc_id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
    Ok(b64().encode(crdt::diff(&state, &sv)?))
}

/// Merge an edit made in this install's editor. Only an active document can
/// be edited; the change waits for the next flush to be mailed.
pub fn apply_local_update(db: &Database, account_id: &str, doc_id: &str, update: &str, now: i64) -> Result<()> {
    let doc = shared_doc_in_account(db, account_id, doc_id)?;
    if doc.status != DocStatus::Active {
        return Err(AppError::InvalidInput("This document is read-only here".into()));
    }
    let update = b64()
        .decode(update)
        .map_err(|_| AppError::InvalidInput("Not a valid shared document update".into()))?;
    crdt::validate_update(&update)?;
    let state = db
        .shared_doc_state(doc_id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
    let merged = crdt::merge(&state, &update)?;
    db.set_shared_doc_state(doc_id, &merged, true, now)?;
    let author = db.get_account(account_id)?.map(|a| me(&a)).unwrap_or_default();
    db.record_doc_version(doc_id, &author, "local", &merged, now)?;
    reindex(db, doc_id)
}

/// Accept an invitation: the document becomes editable and its changes are
/// mailed to its participants from now on.
pub fn accept(db: &Database, account_id: &str, doc_id: &str, now: i64) -> Result<SharedDoc> {
    let doc = shared_doc_in_account(db, account_id, doc_id)?;
    if doc.status != DocStatus::Invited {
        return Err(AppError::InvalidInput("Only an invitation can be accepted".into()));
    }
    db.set_shared_doc_status(doc_id, DocStatus::Active, Some(now), now)?;
    shared_doc_in_account(db, account_id, doc_id)
}

/// Decline an invitation or leave a document: kept read-only here, nothing
/// mailed, later messages about it ignored.
pub fn leave(db: &Database, account_id: &str, doc_id: &str, now: i64) -> Result<SharedDoc> {
    shared_doc_in_account(db, account_id, doc_id)?;
    db.set_shared_doc_status(doc_id, DocStatus::Left, None, now)?;
    shared_doc_in_account(db, account_id, doc_id)
}

// ── Folders, history and search ──────────────────────────────────────────────

/// Refresh a document's entry in the search index from its current content.
fn reindex(db: &Database, doc_id: &str) -> Result<()> {
    let doc = db
        .get_shared_doc(doc_id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
    let state = db
        .shared_doc_state(doc_id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
    db.index_shared_doc(doc_id, &doc.title, &crdt::plain_text(&state, doc.kind)?)
}

/// Documents of one account whose title or text matches `query` (every word,
/// as a prefix), best match first. A blank query matches nothing.
pub fn search(db: &Database, account_id: &str, query: &str) -> Result<Vec<SharedDoc>> {
    let fts = crate::db::emails::sanitize_fts_query(query);
    if fts.is_empty() {
        return Ok(Vec::new());
    }
    // Documents from before the index existed are indexed on first search.
    for doc_id in db.unindexed_shared_doc_ids(account_id)? {
        reindex(db, &doc_id)?;
    }
    db.search_shared_docs(account_id, &fts, 50)
}

/// Pure: a folder name as stored — trimmed, one line, at most 100 characters.
pub fn normalize_folder_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(AppError::InvalidInput("A folder needs a one-line name".into()));
    }
    Ok(name.chars().take(100).collect())
}

pub fn list_folders(db: &Database, account_id: &str) -> Result<Vec<DocFolder>> {
    db.list_doc_folders(account_id)
}

pub fn create_folder(
    db: &Database,
    account_id: &str,
    name: &str,
    parent_id: Option<&str>,
    now: i64,
) -> Result<DocFolder> {
    if let Some(parent) = parent_id {
        doc_folder_in_account(db, account_id, parent)?;
    }
    let folder = DocFolder {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        parent_id: parent_id.map(str::to_string),
        name: normalize_folder_name(name)?,
        created_at: now,
    };
    db.insert_doc_folder(&folder)?;
    Ok(folder)
}

pub fn rename_folder(db: &Database, account_id: &str, folder_id: &str, name: &str) -> Result<DocFolder> {
    doc_folder_in_account(db, account_id, folder_id)?;
    db.rename_doc_folder(folder_id, &normalize_folder_name(name)?)?;
    doc_folder_in_account(db, account_id, folder_id)
}

/// Delete a folder; what it held moves up one level.
pub fn delete_folder(db: &Database, account_id: &str, folder_id: &str) -> Result<()> {
    doc_folder_in_account(db, account_id, folder_id)?;
    db.delete_doc_folder(folder_id)
}

/// Put a document in a folder of the same account, or at the top level.
pub fn move_doc(db: &Database, account_id: &str, doc_id: &str, folder_id: Option<&str>) -> Result<SharedDoc> {
    shared_doc_in_account(db, account_id, doc_id)?;
    if let Some(folder) = folder_id {
        doc_folder_in_account(db, account_id, folder)?;
    }
    db.set_shared_doc_folder(doc_id, folder_id)?;
    shared_doc_in_account(db, account_id, doc_id)
}

/// A document's history, newest first.
pub fn versions(db: &Database, account_id: &str, doc_id: &str) -> Result<Vec<DocVersion>> {
    shared_doc_in_account(db, account_id, doc_id)?;
    db.list_doc_versions(doc_id)
}

/// The whole document as it was at one version, base64 Yjs v1.
pub fn version_state(db: &Database, account_id: &str, doc_id: &str, version_id: i64) -> Result<String> {
    shared_doc_in_account(db, account_id, doc_id)?;
    let state = db
        .doc_version_state(doc_id, version_id)?
        .ok_or_else(|| AppError::NotFound(format!("Version {version_id} not found")))?;
    Ok(b64().encode(state))
}

// ── Mail transport ───────────────────────────────────────────────────────────

/// The sender's UI language, which the human-readable text is written in.
fn mail_language(db: &Database) -> Result<crate::services::i18n::Language> {
    Ok(crate::services::i18n::resolve_ui_language(db)?.unwrap_or_default())
}

fn invitation_body(
    db: &Database,
    account: &Account,
    doc: &SharedDoc,
    snapshot_html: Option<&str>,
) -> Result<EmailBody> {
    let sharer = crate::services::accounts::sender_display_name(account).unwrap_or(&account.email);
    let text = mail_text::invitation(mail_language(db)?, sharer, &doc.title);
    let html = snapshot_html.map(|snapshot| {
        format!(
            "<p>{}</p><hr>{}",
            text.split("\n\n")
                .map(ammonia::clean_text)
                .collect::<Vec<_>>()
                .join("</p><p>"),
            crate::services::emails::sanitize_outgoing_html(snapshot)
        )
    });
    let body = match html {
        Some(html) => EmailBody::with_html(text, html),
        None => EmailBody::plain(text),
    };
    Ok(body.without_footer())
}

fn update_body(db: &Database, doc: &SharedDoc) -> Result<EmailBody> {
    Ok(EmailBody::plain(mail_text::update(mail_language(db)?, &doc.title)).without_footer())
}

/// Mail `update` (with our state vector) to every participant but us.
async fn mail_envelope(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    doc: &SharedDoc,
    update: Vec<u8>,
    state_vector: Vec<u8>,
    body: EmailBody,
    purpose: Purpose,
) -> Result<Vec<String>> {
    let to = planner::recipients(&doc.participants, &account.email);
    if to.is_empty() {
        return Ok(to);
    }
    let bytes = envelope::encode(&Envelope {
        purpose,
        doc_id: doc.id.clone(),
        kind: doc.kind,
        title: doc.title.clone(),
        participants: doc.participants.clone(),
        state_vector,
        update,
    })?;
    let attachment = EmailAttachment {
        filename: envelope::file_name(&doc.id),
        mime_type: envelope::ENVELOPE_MIME.to_string(),
        data: b64().encode(bytes),
        content_id: None,
        is_inline: false,
    };
    crate::services::emails::send_new_email_with_provider(
        db,
        &account.id,
        to.clone(),
        Vec::new(),
        &mail_text::subject(mail_language(db)?, &doc.title),
        &body,
        vec![attachment],
        provider,
    )
    .await?;
    Ok(to)
}

/// Share a document with `recipients` and mail them an invitation holding
/// the whole document. Calling it is the user's consent to mailing this
/// document's later changes to its participants automatically.
pub async fn share(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    doc_id: &str,
    recipients: &[String],
    snapshot_html: Option<&str>,
    now: i64,
) -> Result<SharedDoc> {
    let doc = shared_doc_in_account(db, &account.id, doc_id)?;
    if doc.status != DocStatus::Active {
        return Err(AppError::InvalidInput("Only an active document can be shared".into()));
    }
    let added = envelope::normalize_participants(recipients)?;
    let participants = planner::merged_participants(&doc.participants, &added);
    if planner::recipients(&participants, &account.email).is_empty() {
        return Err(AppError::InvalidInput(
            "Share the document with someone other than yourself".into(),
        ));
    }
    envelope::normalize_participants(&participants)?;
    db.add_shared_doc_participants(doc_id, &participants)?;
    db.set_shared_doc_status(doc_id, DocStatus::Active, Some(now), now)?;
    let doc = shared_doc_in_account(db, &account.id, doc_id)?;

    let state = db
        .shared_doc_state(doc_id)?
        .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
    let ours = crdt::integrate(&state)?;
    // Until it goes out, the document counts as having changes to mail: a
    // failed invitation is retried by the flush with the whole document.
    db.mark_shared_doc_dirty(doc_id, now)?;
    let dirty_since = db.get_shared_doc(doc_id)?.and_then(|d| d.dirty_since);
    let body = invitation_body(db, account, &doc, snapshot_html)?;
    let to = mail_envelope(
        db,
        account,
        provider,
        &doc,
        state,
        ours.state_vector.clone(),
        body,
        Purpose::Invitation,
    )
    .await?;
    if let Some(at) = dirty_since {
        db.claim_shared_doc_flush(doc_id, at)?;
    }
    db.set_participant_state_vectors(doc_id, &to, &ours.state_vector)?;
    logger::log(
        "success",
        "sync",
        format!("[{}] Shared \"{}\" with {}", account.email, doc.title, to.join(", ")),
    );
    shared_doc_in_account(db, &account.id, doc_id)
}

/// Mail a document's pending changes now, if it has any and the user agreed
/// to mailing them. Returns whether a message went out.
///
/// The update sent is what the least up-to-date recipient lacks, so a lost
/// message is made good by the next one. After it goes out every recipient is
/// assumed to have it; a recipient whose later message says otherwise is
/// caught up again ([`ingest_arrivals`] marks the document dirty).
pub async fn flush(db: &Arc<Database>, account: &Account, provider: &dyn EmailProvider, doc_id: &str) -> Result<bool> {
    let doc = shared_doc_in_account(db, &account.id, doc_id)?;
    let (Some(dirty_since), Some(_)) = (doc.dirty_since, doc.consented_at) else {
        return Ok(false);
    };
    if doc.status != DocStatus::Active || !db.claim_shared_doc_flush(doc_id, dirty_since)? {
        return Ok(false);
    }
    let attempt = async {
        let state = db
            .shared_doc_state(doc_id)?
            .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
        let ours = crdt::integrate(&state)?;
        let to = planner::recipients(&doc.participants, &account.email);
        let known: Vec<Vec<u8>> = db
            .shared_doc_participants(doc_id)?
            .into_iter()
            .filter(|p| to.contains(&p.address))
            .map(|p| p.state_vector.unwrap_or_else(crdt::empty_state_vector))
            .collect();
        let update = crdt::diff(&state, &crdt::min_state_vector(&known)?)?;
        let sent_to = mail_envelope(
            db,
            account,
            provider,
            &doc,
            update,
            ours.state_vector.clone(),
            update_body(db, &doc)?,
            Purpose::Update,
        )
        .await?;
        db.set_participant_state_vectors(doc_id, &sent_to, &ours.state_vector)?;
        Ok::<bool, AppError>(!sent_to.is_empty())
    };
    match attempt.await {
        Ok(sent) => Ok(sent),
        Err(e) => {
            // The changes were not mailed: keep them pending for the next pass.
            db.mark_shared_doc_dirty(doc_id, dirty_since)?;
            Err(e)
        }
    }
}

/// Flush every document whose changes are due ([`planner::due_flushes`]).
/// Never fails as a whole: each failure is logged and retried next pass.
pub async fn flush_due(db: &Arc<Database>, now: i64, providers: &dyn OutboxProviders) -> usize {
    if !is_enabled(db) {
        return 0;
    }
    let docs = match db.list_shared_docs(None) {
        Ok(docs) => docs,
        Err(e) => {
            logger::log("error", "sync", format!("Shared documents could not be read: {e}"));
            return 0;
        }
    };
    let candidates: Vec<FlushCandidate> = docs
        .iter()
        .map(|d| FlushCandidate {
            id: d.id.clone(),
            status: d.status,
            consented: d.consented_at.is_some(),
            dirty_since: d.dirty_since,
            updated_at: d.updated_at,
        })
        .collect();
    let mut sent = 0;
    for doc_id in planner::due_flushes(&candidates, now) {
        let Some(doc) = docs.iter().find(|d| d.id == doc_id) else {
            continue;
        };
        let outcome = async {
            let account = db
                .get_account(&doc.account_id)?
                .ok_or_else(|| AppError::NotFound(format!("Account {} not found", doc.account_id)))?;
            let provider = providers.provider_for(&account).await?;
            flush(db, &account, provider.as_ref(), &doc.id).await
        };
        match outcome.await {
            Ok(true) => sent += 1,
            Ok(false) => {}
            Err(e) => logger::log(
                "error",
                "sync",
                format!("Changes to \"{}\" could not be mailed, will retry: {e}", doc.title),
            ),
        }
    }
    sent
}

// ── Attached to an email ─────────────────────────────────────────────────────

/// MIME type of the placeholder a composer puts in an email's attachments to
/// attach an EO Doc. Its data is the document id (base64). It never reaches a
/// provider: [`resolve_doc_refs`] swaps it for the document's envelope when
/// the email actually goes out (so an undone or cancelled send shares nothing).
pub const DOC_REF_MIME: &str = "application/vnd.emailops.doc-ref";

/// The placeholder attachment for a document.
pub fn doc_ref_attachment(doc_id: &str) -> EmailAttachment {
    EmailAttachment {
        filename: envelope::file_name(doc_id),
        mime_type: DOC_REF_MIME.to_string(),
        data: b64().encode(doc_id),
        content_id: None,
        is_inline: false,
    }
}

/// A document an outgoing email shares, to record once the email is sent.
pub struct SharedByMail {
    doc_id: String,
    recipients: Vec<String>,
    state_vector: Vec<u8>,
}

/// Pure: a file name for the attachment from the document title.
fn attachment_name(title: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("{}{}", safe.trim(), envelope::ENVELOPE_EXTENSION)
}

/// Swap each EO Doc placeholder in `attachments` for the document's
/// envelope, shared with `recipients` (the email's To and Cc). Attaching a
/// document is the user's consent to sharing it with them, given in the
/// composer. Fails — and the email is not sent — for a document that is not
/// this account's or not editable here.
pub fn resolve_doc_refs(
    db: &Database,
    account: &Account,
    attachments: Vec<EmailAttachment>,
    recipients: &[String],
) -> Result<(Vec<EmailAttachment>, Vec<SharedByMail>)> {
    if !attachments.iter().any(|a| a.mime_type == DOC_REF_MIME) {
        return Ok((attachments, Vec::new()));
    }
    if !is_enabled(db) {
        return Err(AppError::InvalidInput("EO Docs is turned off in Settings".into()));
    }
    let added = envelope::normalize_participants(recipients)?;
    let mut out = Vec::with_capacity(attachments.len());
    let mut shared = Vec::new();
    for attachment in attachments {
        if attachment.mime_type != DOC_REF_MIME {
            out.push(attachment);
            continue;
        }
        let doc_id = b64()
            .decode(&attachment.data)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| AppError::InvalidInput("Not a valid EO Docs attachment".into()))?;
        let doc = shared_doc_in_account(db, &account.id, &doc_id)?;
        if doc.status != DocStatus::Active {
            return Err(AppError::InvalidInput(format!("\"{}\" is read-only here", doc.title)));
        }
        let state = db
            .shared_doc_state(&doc_id)?
            .ok_or_else(|| AppError::NotFound(format!("Shared document {doc_id} not found")))?;
        let ours = crdt::integrate(&state)?;
        let participants = planner::merged_participants(&doc.participants, &added);
        envelope::normalize_participants(&participants)?;
        let bytes = envelope::encode(&Envelope {
            purpose: Purpose::Message,
            doc_id: doc.id.clone(),
            kind: doc.kind,
            title: doc.title.clone(),
            participants,
            state_vector: ours.state_vector.clone(),
            update: state,
        })?;
        out.push(EmailAttachment {
            filename: attachment_name(&doc.title),
            mime_type: envelope::ENVELOPE_MIME.to_string(),
            data: b64().encode(bytes),
            content_id: None,
            is_inline: false,
        });
        shared.push(SharedByMail {
            doc_id,
            recipients: planner::recipients(&added, &account.email),
            state_vector: ours.state_vector,
        });
    }
    Ok((out, shared))
}

/// After the email went out: the documents it carried are shared with its
/// recipients, who now hold everything up to the state that was sent. The
/// email is already sent, so a failure here is logged, not returned.
pub fn record_shared_by_mail(db: &Database, account: &Account, shared: &[SharedByMail], now: i64) {
    for item in shared {
        let outcome = (|| {
            db.add_shared_doc_participants(&item.doc_id, &item.recipients)?;
            db.set_shared_doc_status(&item.doc_id, DocStatus::Active, Some(now), now)?;
            db.set_participant_state_vectors(&item.doc_id, &item.recipients, &item.state_vector)
        })();
        match outcome {
            Ok(()) => logger::log(
                "success",
                "sync",
                format!(
                    "[{}] Shared an EO Doc with {}",
                    account.email,
                    item.recipients.join(", ")
                ),
            ),
            Err(e) => logger::log(
                "error",
                "sync",
                format!(
                    "[{}] An EO Doc went out but its sharing was not recorded: {e}",
                    account.email
                ),
            ),
        }
    }
    notify_changed(&shared.iter().map(|s| s.doc_id.clone()).collect::<Vec<_>>());
}

// ── Ingest ───────────────────────────────────────────────────────────────────

/// The envelope attachment of a message, if it has one.
fn envelope_attachment(attachments: &[AttachmentInfo]) -> Option<&AttachmentInfo> {
    attachments.iter().find(|a| envelope::is_envelope_file(&a.filename))
}

async fn envelope_bytes(provider: &dyn EmailProvider, email: &Email, info: &AttachmentInfo) -> Result<Vec<u8>> {
    if info.size > envelope::MAX_ENVELOPE_BYTES as i64 {
        return Err(AppError::InvalidInput("Shared document message too large".into()));
    }
    match &info.inline_data {
        Some(data) => crate::services::attachments::decode_inline_base64(data),
        None => provider.fetch_attachment_bytes(&email.id, &info.attachment_id).await,
    }
}

/// What one arriving envelope did.
enum Applied {
    Changed,
    Ignored,
}

/// Merge one envelope into this install, per [`planner::plan_arrival`].
fn apply_arrival(db: &Database, account: &Account, sender: &str, env: &Envelope, now: i64) -> Result<Applied> {
    let known = db.get_shared_doc(&env.doc_id)?;
    // A document id is global: one held by another local account is not
    // this account's to change.
    if known.as_ref().is_some_and(|d| d.account_id != account.id) {
        return Ok(Applied::Ignored);
    }
    let known_doc = known.as_ref().map(|d| KnownDoc {
        status: d.status,
        participants: &d.participants,
    });
    let sender = sender.trim().to_lowercase();
    let state = match planner::plan_arrival(env, &sender, &account.email, known_doc.as_ref()) {
        Arrival::Ignore(reason) => {
            logger::log(
                "debug",
                "sync",
                format!("[{}] Shared document message ignored: {reason}", account.email),
            );
            return Ok(Applied::Ignored);
        }
        Arrival::Invitation => {
            let state = crdt::merge(&crdt::empty_state(), &env.update)?;
            // The user's own message, sent from another install of theirs
            // (its Sent copy): already shared and consented to over there.
            let own = sender == me(account);
            db.insert_shared_doc(&NewSharedDoc {
                id: &env.doc_id,
                account_id: &account.id,
                kind: env.kind,
                title: &env.title,
                state: &state,
                status: if own { DocStatus::Active } else { DocStatus::Invited },
                participants: &env.participants,
                now,
            })?;
            if own {
                db.set_shared_doc_status(&env.doc_id, DocStatus::Active, Some(now), now)?;
            } else {
                logger::log(
                    "info",
                    "sync",
                    format!("[{}] {sender} shared \"{}\" with you", account.email, env.title),
                );
            }
            state
        }
        Arrival::Apply => {
            let stored = db
                .shared_doc_state(&env.doc_id)?
                .ok_or_else(|| AppError::NotFound(format!("Shared document {} not found", env.doc_id)))?;
            let state = crdt::merge(&stored, &env.update)?;
            db.set_shared_doc_state(&env.doc_id, &state, false, now)?;
            db.add_shared_doc_participants(&env.doc_id, &env.participants)?;
            state
        }
    };
    if sender != me(account) {
        db.set_participant_state_vectors(&env.doc_id, std::slice::from_ref(&sender), &env.state_vector)?;
    }
    db.record_doc_version(&env.doc_id, &sender, "remote", &state, now)?;
    reindex(db, &env.doc_id)?;
    // Ask for a catch-up when something is missing here, or send one when the
    // sender lacks what this install has: either way, mail our state vector.
    let ours = crdt::integrate(&state)?;
    if ours.has_missing || (sender != me(account) && crdt::lacks(&ours.state_vector, &env.state_vector)?) {
        db.mark_shared_doc_dirty(&env.doc_id, now)?;
    }
    Ok(Applied::Changed)
}

/// Sync hook: apply the document messages of a freshly stored batch, then
/// mark the background change messages read and archive them so they do not
/// clutter the inbox. An invitation, or an email with a document attached,
/// stays in the inbox. Never fails the sync:
/// each failure is logged. Returns how many documents changed.
pub async fn ingest_arrivals(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    batch: &[(Email, Vec<AttachmentInfo>)],
    now: i64,
) -> usize {
    if !is_enabled(db) {
        return 0;
    }
    let mut changed: Vec<String> = Vec::new();
    let mut tidy: Vec<ThreadRef> = Vec::new();
    // Oldest first: providers list newest first, and the first message of a
    // document is the invitation the user should see.
    let mut ordered: Vec<&(Email, Vec<AttachmentInfo>)> = batch.iter().collect();
    ordered.sort_by_key(|(email, _)| (email.timestamp, email.id.clone()));
    for (email, attachments) in ordered {
        let Some(info) = envelope_attachment(attachments) else {
            continue;
        };
        let outcome = async {
            if db.shared_doc_message_seen(&account.id, &email.id)? {
                return Ok(None);
            }
            let env = envelope::decode(&envelope_bytes(provider, email, info).await?)?;
            let applied = apply_arrival(db, account, &email.sender_email, &env, now)?;
            db.record_shared_doc_message(&account.id, &email.id, &env.doc_id, now)?;
            Ok::<_, AppError>(Some((env.doc_id, env.purpose, applied)))
        };
        match outcome.await {
            Ok(Some((doc_id, purpose, Applied::Changed))) => {
                if !changed.contains(&doc_id) {
                    changed.push(doc_id);
                }
                // Only background change messages are tidied away; an
                // invitation, or an email a person wrote with a document
                // attached, is mail the user should see.
                if purpose == Purpose::Update && email.mailbox == "inbox" && !email.is_sent {
                    tidy.push(ThreadRef {
                        account_id: account.id.clone(),
                        thread_id: email.thread_id.clone(),
                    });
                }
            }
            Ok(_) => {}
            Err(e) => logger::log(
                "error",
                "sync",
                format!(
                    "[{}] A shared document message could not be applied: {e}",
                    account.email
                ),
            ),
        }
    }
    tidy.dedup();
    if !tidy.is_empty() {
        let access = if provider_supports_mailbox_writes(&account.provider) {
            ProviderAccess::Ready(provider)
        } else {
            ProviderAccess::LocalOnly
        };
        let threads: Vec<&ThreadRef> = tidy.iter().collect();
        let mut report = ThreadActionReport::default();
        for action in [ThreadAction::MarkRead, ThreadAction::Archive] {
            let access = match &access {
                ProviderAccess::Ready(p) => ProviderAccess::Ready(*p),
                _ => ProviderAccess::LocalOnly,
            };
            crate::services::emails::apply_to_account(db, account, &threads, action, access, &mut report).await;
        }
    }
    if !changed.is_empty() {
        logger::log(
            "info",
            "sync",
            format!(
                "[{}] {} shared document(s) updated by email",
                account.email,
                changed.len()
            ),
        );
    }
    notify_changed(&changed);
    changed.len()
}

#[cfg(test)]
mod tests;
