//! Materializing `JunkSignals` from the database.
//!
//! The thin executor half of the pure-planner split: everything that touches
//! SQLite lives here, so `verdict::judge` stays a function of its arguments and
//! can be exhaustively table-tested.

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::services::junk::auth::expected_authserv;
use crate::services::junk::model::{self, ModelAxis, NaiveBayes};
use crate::services::junk::tokens;
use crate::services::junk::verdict::JunkSignals;

/// Cap on the contact reference set.
///
/// Lookalike detection is O(candidates × references); the reference list is the
/// user's real correspondents, which is small, but a shared or very old mailbox
/// can make it large enough to matter on a full backfill.
const MAX_KNOWN_CONTACTS: usize = 2_000;

/// Per-account context that is identical for every message in a scoring batch.
///
/// Hoisted out of the per-email path so a 500-message batch runs the contact
/// queries once instead of five hundred times.
pub struct AccountContext {
    pub account_id: String,
    pub known_contact_domains: Vec<String>,
    pub known_contact_names: Vec<String>,
    pub trusted_authserv: Option<String>,
    /// Trained models, when they exist and clear the sample floor. `None` means
    /// the deterministic layer runs alone.
    pub spam_model: Option<NaiveBayes>,
    pub graymail_model: Option<NaiveBayes>,
    /// Base rates. **Configuration, not learnt** — see `model::score`.
    pub spam_prior: f32,
    pub graymail_prior: f32,
}

/// Default base rates.
///
/// Measured against a real mailbox rather than assumed: roughly 1% of inbox mail
/// is spam the server let through, and roughly a quarter is unengaged bulk.
/// These are deliberately NOT derived from the training counts — the free labels
/// come from the provider's spam folder, which over-represents spam by more than
/// an order of magnitude.
const DEFAULT_SPAM_PRIOR: f32 = 0.01;
const DEFAULT_GRAYMAIL_PRIOR: f32 = 0.25;

fn prior_pref(db: &Arc<Database>, key: &str, default: f32) -> f32 {
    db.get_preference(key)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (0.0001..=0.9999).contains(v))
        .unwrap_or(default)
}

impl AccountContext {
    pub fn load(db: &Arc<Database>, account_id: &str) -> Result<Self> {
        let (domains, names) = db.get_known_contacts(account_id, MAX_KNOWN_CONTACTS)?;
        // Loaded once per batch: the blob is a couple of hundred kilobytes and
        // re-reading it per message would dominate the scoring cost.
        let spam_model = db
            .load_junk_model(account_id, ModelAxis::Spam)?
            .map(|(m, _)| m)
            .filter(NaiveBayes::is_usable);
        let graymail_model = db
            .load_junk_model(account_id, ModelAxis::Graymail)?
            .map(|(m, _)| m)
            .filter(NaiveBayes::is_usable);
        let provider = db
            .get_account(account_id)
            .ok()
            .flatten()
            .map(|a| a.provider)
            .unwrap_or_default();
        Ok(Self {
            account_id: account_id.to_string(),
            known_contact_domains: domains,
            known_contact_names: names,
            spam_model,
            graymail_model,
            spam_prior: prior_pref(db, "junk_spam_prior", DEFAULT_SPAM_PRIOR),
            graymail_prior: prior_pref(db, "junk_graymail_prior", DEFAULT_GRAYMAIL_PRIOR),
            // `None` for self-hosted IMAP: we cannot identify the MTA whose
            // Authentication-Results we should believe, so we believe none of
            // them. That yields Unknown rather than a false confident verdict.
            trusted_authserv: expected_authserv(&provider).map(str::to_string),
        })
    }
}

/// Build the signal set for one email.
pub fn materialize(db: &Arc<Database>, ctx: &AccountContext, email_id: &str) -> Result<Option<JunkSignals>> {
    let Some(email) = db.get_email_by_id(email_id)? else {
        return Ok(None);
    };

    // `row_to_email` deliberately leaves `body` empty — bodies live in their own
    // table so list queries never drag them along. The content layer (links,
    // pressure wording, hidden text) is worthless without it, so it has to be
    // fetched explicitly here. Forgetting this made every content signal
    // evaluate against an empty string on real mail while the eval corpus, which
    // supplies bodies directly, showed nothing wrong.
    let body = db.get_email_body(&email.id).unwrap_or_default();

    let headers = db
        .get_email_headers_batch(std::slice::from_ref(&email.id))?
        .remove(&email.id);

    let attachment_names = db
        .get_email_attachment_metas(&email.id)
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.filename)
        .collect();

    let user_marked_not_junk = db
        .get_junk_verdicts_batch(std::slice::from_ref(&email.id))?
        .get(&email.id)
        .and_then(|v| v.user_override.clone())
        .as_deref()
        == Some("not_junk");

    let x_mailer = headers.as_ref().and_then(|h| h.x_mailer.clone());
    // Same projection the models were trained on — subject + snippet + sender.
    // Scoring against the full body would mean scoring against features
    // training never saw.
    let feature_set = tokens::features(&tokens::FeatureInput {
        subject: &email.subject,
        snippet: &email.snippet,
        sender_email: &email.sender_email,
        x_mailer: x_mailer.as_deref(),
    });

    Ok(Some(JunkSignals {
        statistical_spam: ctx
            .spam_model
            .as_ref()
            .map(|m| model::score(m, &feature_set, ctx.spam_prior)),
        statistical_graymail: ctx
            .graymail_model
            .as_ref()
            .map(|m| model::score(m, &feature_set, ctx.graymail_prior)),
        headers,
        subject: email.subject.clone(),
        body,
        sender_display_name: email.sender.clone(),
        sender_engaged: db.is_sender_engaged(&ctx.account_id, &email.sender_email)?,
        sender_trusted: db
            .is_sender_trusted(&ctx.account_id, &email.sender_email)
            .unwrap_or(false),
        own_thread: db.thread_has_own_message(&ctx.account_id, &email.thread_id)?,
        sender_message_count: db
            .count_messages_from_sender(&ctx.account_id, &email.sender_email)
            .unwrap_or(0),
        provider_category: Some(email.category.clone()),
        attachment_names,
        user_marked_not_junk,
        known_contact_domains: ctx.known_contact_domains.clone(),
        known_contact_names: ctx.known_contact_names.clone(),
        trusted_authserv: ctx.trusted_authserv.clone(),
        sender_email: email.sender_email,
    }))
}
