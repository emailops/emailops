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
pub mod planner;

use std::sync::Arc;

use base64::Engine;
use serde::Serialize;

use crate::db::shared_docs::NewSharedDoc;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::shared_docs::{DocKind, DocStatus, SharedDoc};
use crate::models::{Account, Email};
use crate::services::emails::{ProviderAccess, ThreadAction, ThreadActionReport, ThreadRef};
use crate::services::logger;
use crate::services::outbox::OutboxProviders;
use crate::services::ownership::shared_doc_in_account;
use crate::sync::provider::{
    provider_supports_mailbox_writes, AttachmentInfo, EmailAttachment, EmailBody, EmailProvider,
};
use envelope::Envelope;
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
    db.set_shared_doc_state(doc_id, &crdt::merge(&state, &update)?, true, now)
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

// ── Mail transport ───────────────────────────────────────────────────────────

fn subject(title: &str) -> String {
    format!("{title} (EmailOps shared document)")
}

fn invitation_body(account: &Account, doc: &SharedDoc, snapshot_html: Option<&str>) -> EmailBody {
    let sharer = crate::services::accounts::sender_display_name(account).unwrap_or(&account.email);
    let text = format!(
        "{sharer} shared \"{}\" with you in EmailOps.\n\n\
         Open EmailOps to edit it together: changes travel between the people \
         sharing it as email, with no server in between. Without EmailOps you \
         can read the copy below, but not edit it.",
        doc.title
    );
    let html = snapshot_html.map(|snapshot| {
        format!(
            "<p>{}</p><hr>{}",
            ammonia::clean_text(&text),
            crate::services::emails::sanitize_outgoing_html(snapshot)
        )
    });
    let body = match html {
        Some(html) => EmailBody::with_html(text, html),
        None => EmailBody::plain(text),
    };
    body.without_footer()
}

fn update_body(doc: &SharedDoc) -> EmailBody {
    EmailBody::plain(format!(
        "Changes to the shared document \"{}\", sent by EmailOps to the people \
         editing it. EmailOps applies them automatically; you can ignore this message.",
        doc.title
    ))
    .without_footer()
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
) -> Result<Vec<String>> {
    let to = planner::recipients(&doc.participants, &account.email);
    if to.is_empty() {
        return Ok(to);
    }
    let bytes = envelope::encode(&Envelope {
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
        &subject(&doc.title),
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
    let body = invitation_body(account, &doc, snapshot_html);
    let to = mail_envelope(db, account, provider, &doc, state, ours.state_vector.clone(), body).await?;
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
            update_body(&doc),
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
    // Ask for a catch-up when something is missing here, or send one when the
    // sender lacks what this install has: either way, mail our state vector.
    let ours = crdt::integrate(&state)?;
    if ours.has_missing || (sender != me(account) && crdt::lacks(&ours.state_vector, &env.state_vector)?) {
        db.mark_shared_doc_dirty(&env.doc_id, now)?;
    }
    Ok(Applied::Changed)
}

/// Sync hook: apply the document messages of a freshly stored batch, then
/// mark them read and archive them so they do not clutter the inbox. An
/// invitation stays in the inbox so the user sees it. Never fails the sync:
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
            let was_known = db.get_shared_doc(&env.doc_id)?.is_some();
            let applied = apply_arrival(db, account, &email.sender_email, &env, now)?;
            db.record_shared_doc_message(&account.id, &email.id, &env.doc_id, now)?;
            Ok::<_, AppError>(Some((env.doc_id, was_known, applied)))
        };
        match outcome.await {
            Ok(Some((doc_id, was_known, Applied::Changed))) => {
                if !changed.contains(&doc_id) {
                    changed.push(doc_id);
                }
                if was_known && email.mailbox == "inbox" && !email.is_sent {
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
