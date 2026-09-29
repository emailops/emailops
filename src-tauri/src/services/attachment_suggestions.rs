//! Candidate attachment rules mined from recurring document attachments.
//!
//! A sender that keeps mailing the same kind of document (a monthly invoice,
//! a payslip, a bank statement) is exactly what an attachment rule is for, so
//! the app proposes one and lets the user confirm or dismiss it. Mining is a
//! deterministic heuristic over `email_attachment_meta` — no AI involved.
//!
//! [`plan_suggestions`] is the pure planner; the executor that reads the
//! observations and persists the candidates lives below it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
#[cfg(feature = "desktop")]
use tauri::Emitter;

use crate::models::error::{AppError, Result};
use crate::models::{AttachmentRule, AttachmentRuleSuggestion, AttachmentRuleSuggestionStatus};

/// One document attachment on one received email — the planner's input row.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentObservation {
    pub email_id: String,
    pub sender_email: String,
    /// Display name from the From header; may be empty.
    pub sender_name: String,
    pub subject: String,
    pub timestamp: i64,
    pub filename: String,
    pub mime_type: String,
}

/// A proposed attachment rule, before it is persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionCandidate {
    /// Stable identity (`sender identity|filename pattern`, lowercased): the
    /// sender part is the pooled identity (`*@acme.com`, or the address of a
    /// person), not the proposed pattern, so a second sender address joining
    /// the group keeps the same key — and the same pending row id.
    pub key: String,
    pub name: String,
    pub sender_email_pattern: String,
    pub filename_pattern: Option<String>,
    pub tags: Vec<String>,
    pub email_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub sample_filenames: Vec<String>,
}

/// A suggestion the user already accepted or dismissed — the planner never
/// proposes what it covers again.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSuggestion {
    pub sender_email_pattern: String,
    pub filename_pattern: Option<String>,
}

/// Thresholds for what counts as "recurring".
#[derive(Debug, Clone, Copy)]
pub struct SuggestionParams {
    /// Distinct emails carrying the document.
    pub min_emails: usize,
    /// Distinct calendar months those emails span — a burst of attachments
    /// on one day is a conversation, not a recurring document.
    pub min_distinct_months: usize,
    /// Seconds between the first and the last email: two emails on Jan 31
    /// and Feb 1 span two months but are one conversation.
    pub min_spread_secs: i64,
    pub max_suggestions: usize,
}

impl Default for SuggestionParams {
    fn default() -> Self {
        Self {
            // Many providers send one invoice a month: two months of it is
            // already a pattern worth proposing.
            min_emails: 2,
            min_distinct_months: 2,
            min_spread_secs: 20 * 86_400,
            max_suggestions: 20,
        }
    }
}

const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "docm", "xls", "xlsx", "xlsm", "ppt", "pptx", "odt", "ods", "odp", "rtf", "csv", "xml",
    // Spanish / Italian e-invoices (Facturae, FatturaPA) often arrive zipped.
    "zip",
];

const DOCUMENT_MIME_PREFIXES: &[&str] = &[
    "application/pdf",
    "application/msword",
    "application/vnd.ms-",
    "application/vnd.openxmlformats-officedocument.",
    "application/vnd.oasis.opendocument.",
    "application/rtf",
    "application/xml",
    "text/xml",
    "text/csv",
    "application/zip",
];

/// Keyword → tag. A keyword counts when it appears in the filename or the
/// subject of at least half of a candidate's documents.
const KIND_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "invoice",
        &[
            "invoice",
            "invoices",
            "factura",
            "facturas",
            "facture",
            "factures",
            "rechnung",
            "rechnungen",
            "fattura",
            "fatture",
        ],
    ),
    (
        "receipt",
        &["receipt", "receipts", "recibo", "recibos", "quittung", "reçu"],
    ),
    (
        "payroll",
        &[
            "payslip",
            "payroll",
            "payslips",
            "nomina",
            "nómina",
            "nominas",
            "nóminas",
            "lohnabrechnung",
            "fiche de paie",
            "bulletin de paie",
        ],
    ),
    (
        "statement",
        &[
            "statement",
            "statements",
            "extracto",
            "extractos",
            "kontoauszug",
            "relevé",
        ],
    ),
    (
        "contract",
        &[
            "contract",
            "contracts",
            "contrato",
            "contratos",
            "vertrag",
            "contrat",
            "contrats",
        ],
    ),
];

const MAX_SAMPLE_FILENAMES: usize = 3;

fn extension(filename: &str) -> Option<String> {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
}

/// PDFs, office documents and e-invoice formats. Images (logos, inline
/// signatures), calendar invites and S/MIME signatures are excluded — they
/// recur on every email from a sender without being documents worth keeping.
pub fn is_document_attachment(mime_type: &str, filename: &str) -> bool {
    let mime = mime_type.trim().to_ascii_lowercase();
    if DOCUMENT_MIME_PREFIXES.iter().any(|p| mime.starts_with(p)) {
        return true;
    }
    extension(filename).is_some_and(|ext| DOCUMENT_EXTENSIONS.contains(&ext.as_str()))
}

/// Month names (en/es/fr/de, full and abbreviated) as they appear in
/// periodic document names — `invoice-jan.pdf`, `Factura_Enero_2026.pdf`.
const MONTH_NAMES: &str = "january|february|march|april|may|june|july|august|september|october|november|december\
    |jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec\
    |enero|febrero|marzo|abril|mayo|junio|julio|agosto|septiembre|setiembre|octubre|noviembre|diciembre\
    |ene|abr|ago|dic\
    |janvier|février|fevrier|mars|avril|mai|juin|juillet|août|aout|octobre|novembre|décembre|decembre\
    |janv|févr|fevr|juil|déc\
    |januar|februar|märz|maerz|juni|juli|oktober|dezember|okt|dez";

/// Generalise a filename into the glob its siblings share: month names and
/// every run of digits (with the separators between them — dates, ISO
/// timestamps, invoice numbers) become one `*`, and the extension is
/// lowercased.
pub fn filename_family(filename: &str) -> String {
    use std::sync::LazyLock;

    // A month only counts as a whole token, so `summary` or `mayor` survive.
    #[allow(clippy::unwrap_used)] // infallible by construction: literal pattern
    static MONTH: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(&format!(r"(?i)(^|[^\p{{L}}])(?:{MONTH_NAMES})([^\p{{L}}]|$)")).unwrap());
    #[allow(clippy::unwrap_used)] // infallible by construction: literal pattern
    static NUMBER_RUN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\d(?:[\d\-_./: ]*\d|T\d)*Z?").unwrap());
    // `Factura_*_*` (month then year) is one variable part, not two.
    #[allow(clippy::unwrap_used)] // infallible by construction: literal pattern
    static WILDCARD_RUN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\*(?:[\-_./ ]*\*)+").unwrap());

    let (stem, ext) = match filename.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem, Some(ext.to_ascii_lowercase())),
        _ => (filename, None),
    };
    let stem = MONTH.replace_all(stem, "${1}*${2}");
    let stem = NUMBER_RUN.replace_all(&stem, "*");
    let stem = WILDCARD_RUN.replace_all(&stem, "*");
    match ext {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem.into_owned(),
    }
}

/// Fewer fixed letters/digits than this and a family pattern (`*F*.pdf`)
/// says nothing a plain `*.pdf` does not.
const MIN_FIXED_CHARS: usize = 3;

/// The pattern a document is grouped and matched by: its [`filename_family`],
/// or just `*.<ext>` when the family keeps too little fixed text.
fn family_pattern(filename: &str) -> String {
    let family = filename_family(filename);
    let stem = family.rsplit_once('.').map_or(family.as_str(), |(stem, _)| stem);
    if stem.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_FIXED_CHARS {
        return family;
    }
    match extension(filename) {
        Some(ext) => format!("*.{ext}"),
        None => family,
    }
}

/// Second-level labels that are part of a public suffix (`acme.co.uk`,
/// `acme.com.es`) rather than the organisation's name.
const SECOND_LEVEL_SUFFIXES: &[&str] = &["co", "com", "org", "net", "gob", "gov", "edu", "ac"];

/// The organisation label of a domain: `mail.acme.com` → `acme`,
/// `shop.acme.co.uk` → `acme`. Mail senders sit on subdomains
/// (`mail.`, `email.`, `em.`) that are not the company name.
fn organisation_label(domain: &str) -> String {
    let labels: Vec<&str> = domain.trim_matches('.').split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    let idx = match n {
        0 => return String::new(),
        1 => 0,
        _ if n >= 3 && labels[n - 1].len() == 2 && SECOND_LEVEL_SUFFIXES.contains(&labels[n - 2]) => n - 3,
        _ => n - 2,
    };
    labels[idx].to_ascii_lowercase()
}

fn month_index(timestamp: i64) -> i64 {
    use chrono::Datelike;
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|d| i64::from(d.year()) * 12 + i64::from(d.month0()))
        .unwrap_or(0)
}

/// Who a group of documents comes from. Senders of one corporate domain are
/// pooled (invoices often rotate between `billing@` and `noreply@`); senders
/// of a personal provider (gmail.com…) never are.
fn sender_identity(sender_email: &str) -> String {
    match crate::util::email_addr::extract_domain(sender_email) {
        Some(domain) if !crate::util::email_addr::is_personal_email_domain(&domain) => format!("*@{domain}"),
        _ => sender_email.trim().to_ascii_lowercase(),
    }
}

fn is_recurring(group: &[&AttachmentObservation], params: &SuggestionParams) -> bool {
    let emails: HashSet<&str> = group.iter().map(|o| o.email_id.as_str()).collect();
    let months: HashSet<i64> = group.iter().map(|o| month_index(o.timestamp)).collect();
    let first = group.iter().map(|o| o.timestamp).min().unwrap_or_default();
    let last = group.iter().map(|o| o.timestamp).max().unwrap_or_default();
    emails.len() >= params.min_emails
        && months.len() >= params.min_distinct_months
        && last - first >= params.min_spread_secs
}

/// Mail the user sends themselves (a scanner, a self-forward) or gets from
/// colleagues at their own organisation is not a vendor's recurring document.
/// A personal-provider account (gmail.com…) only excludes its own address.
fn is_own_mail(sender_email: &str, account_email: &str) -> bool {
    let sender = sender_email.trim();
    let account = account_email.trim();
    if account.is_empty() {
        return false;
    }
    if sender.eq_ignore_ascii_case(account) {
        return true;
    }
    let (Some(sender_domain), Some(own_domain)) = (
        crate::util::email_addr::extract_domain(sender),
        crate::util::email_addr::extract_domain(account),
    ) else {
        return false;
    };
    !crate::util::email_addr::is_personal_email_domain(&own_domain)
        && (sender_domain == own_domain || sender_domain.ends_with(&format!(".{own_domain}")))
}

/// A suggestion the user already resolved covers a candidate when it names
/// the same sender identity and its filename pattern (none = every document)
/// matches most of the candidate's documents. Keys alone are not enough: the
/// proposed patterns drift as mail arrives (`billing@acme.com` → `*@acme.com`,
/// `*.pdf` → `Invoice_*.pdf`) and a dismissal must survive that.
fn covered_by_resolved(identity: &str, group: &[&AttachmentObservation], resolved: &[ResolvedSuggestion]) -> bool {
    resolved
        .iter()
        .filter(|r| sender_identity(&r.sender_email_pattern) == identity)
        .any(|r| match r.filename_pattern.as_deref().filter(|p| !p.is_empty()) {
            None => true,
            Some(pattern) => {
                let covered = group
                    .iter()
                    .filter(|o| super::attachments::matches_glob(pattern, &o.filename))
                    .count();
                covered * 2 > group.len()
            }
        })
}

fn covered_by_existing_rule(group: &[&AttachmentObservation], rules: &[AttachmentRule]) -> bool {
    let covered = group
        .iter()
        .filter(|o| {
            rules.iter().any(|r| {
                super::attachments::matches_rule(r, &o.sender_email, &o.subject)
                    && super::attachments::matches_filename(r, &o.filename)
            })
        })
        .count();
    covered * 2 > group.len()
}

/// A rule the user already made for this sender: one of its sender patterns
/// matches a sender of the group, or names the same organisation (a rule for
/// `invoicing@email.acme.com` covers what `billing@acme.com` sends). A new
/// suggestion next to it would only be noise — the user extends their rule.
fn similar_rule_exists(group: &[&AttachmentObservation], rules: &[AttachmentRule]) -> bool {
    use crate::util::email_addr::is_personal_email_domain;

    let group_orgs: HashSet<String> = group
        .iter()
        .filter_map(|o| crate::util::email_addr::extract_domain(&o.sender_email))
        .filter(|d| !is_personal_email_domain(d))
        .map(|d| organisation_label(&d))
        .collect();

    rules
        .iter()
        .filter_map(|r| r.sender_email_pattern.as_deref())
        .flat_map(|p| p.split(','))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .any(|pattern| {
            if group
                .iter()
                .any(|o| super::attachments::matches_glob(pattern, &o.sender_email))
            {
                return true;
            }
            let domain = pattern
                .rsplit('@')
                .next()
                .unwrap_or(pattern)
                .trim_matches(|c| c == '*' || c == '.');
            !domain.is_empty()
                && !domain.contains('*')
                && !is_personal_email_domain(domain)
                && group_orgs.contains(&organisation_label(domain))
        })
}

/// `word` occurs in `haystack` with no letter on either side: `contract`
/// is in `Contract_2026.pdf` but not in `contractor.pdf`.
fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + word.len()..].chars().next();
        !before.is_some_and(char::is_alphabetic) && !after.is_some_and(char::is_alphabetic)
    })
}

fn kind_tags(group: &[&AttachmentObservation]) -> Vec<String> {
    let haystacks: Vec<String> = group
        .iter()
        .map(|o| format!("{} {}", o.filename, o.subject).to_lowercase())
        .collect();
    KIND_KEYWORDS
        .iter()
        .filter(|(_, words)| {
            let hits = haystacks
                .iter()
                .filter(|h| words.iter().any(|w| contains_word(h, w)))
                .count();
            hits * 2 >= haystacks.len()
        })
        .map(|(tag, _)| (*tag).to_string())
        .collect()
}

fn build_candidate(
    identity: &str,
    group: &[&AttachmentObservation],
    filename_pattern: Option<String>,
) -> SuggestionCandidate {
    let mut by_recency: Vec<&AttachmentObservation> = group.to_vec();
    by_recency.sort_by_key(|o| std::cmp::Reverse(o.timestamp));

    let senders: HashSet<String> = group
        .iter()
        .map(|o| o.sender_email.trim().to_ascii_lowercase())
        .collect();
    let newest_sender = by_recency[0].sender_email.trim().to_ascii_lowercase();
    let sender_email_pattern = if senders.len() == 1 {
        newest_sender.clone()
    } else {
        sender_identity(&newest_sender)
    };

    let domain = crate::util::email_addr::extract_domain(&newest_sender).unwrap_or_default();
    let corporate = !domain.is_empty() && !crate::util::email_addr::is_personal_email_domain(&domain);
    let label = if corporate {
        organisation_label(&domain)
    } else {
        crate::util::email_addr::company_label_for(&domain, Some(&newest_sender))
    };

    let mut tags = kind_tags(group);
    let display_label = if corporate {
        tags.push(label.clone());
        let mut chars = label.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    } else {
        // A person on gmail.com & co: their display name reads better than
        // the address (the address still goes in the sender pattern).
        by_recency
            .iter()
            .map(|o| o.sender_name.trim())
            .find(|n| !n.is_empty() && !n.eq_ignore_ascii_case(&newest_sender))
            .map(str::to_string)
            .unwrap_or(label)
    };
    let name = match tags.first().filter(|_| tags.len() > usize::from(corporate)) {
        Some(kind) => format!("{display_label} · {kind}"),
        None => display_label,
    };

    let mut sample_filenames: Vec<String> = Vec::new();
    for o in &by_recency {
        if sample_filenames.len() == MAX_SAMPLE_FILENAMES {
            break;
        }
        if !sample_filenames.contains(&o.filename) {
            sample_filenames.push(o.filename.clone());
        }
    }

    let email_count = group.iter().map(|o| o.email_id.as_str()).collect::<HashSet<_>>().len() as i64;
    let key = format!("{identity}|{}", filename_pattern.as_deref().unwrap_or("")).to_lowercase();

    SuggestionCandidate {
        key,
        name,
        sender_email_pattern,
        filename_pattern,
        tags,
        email_count,
        first_seen: by_recency.last().map(|o| o.timestamp).unwrap_or_default(),
        last_seen: by_recency[0].timestamp,
        sample_filenames,
    }
}

/// Pure planner: turn document-attachment observations into ranked candidate
/// rules. Per sender identity, each recurring filename family becomes one
/// candidate; an identity with recurring documents but no shared filename
/// shape falls back to one extension-wide candidate (`*.pdf`).
pub fn plan_suggestions(
    observations: &[AttachmentObservation],
    existing_rules: &[AttachmentRule],
    resolved: &[ResolvedSuggestion],
    account_email: &str,
    params: SuggestionParams,
) -> Vec<SuggestionCandidate> {
    use std::collections::BTreeMap;

    let mut by_identity: BTreeMap<String, Vec<&AttachmentObservation>> = BTreeMap::new();
    for o in observations.iter().filter(|o| {
        is_document_attachment(&o.mime_type, &o.filename)
            && crate::util::email_addr::extract_domain(&o.sender_email).is_some()
            && !is_own_mail(&o.sender_email, account_email)
    }) {
        by_identity.entry(sender_identity(&o.sender_email)).or_default().push(o);
    }

    let mut candidates = Vec::new();
    for (identity, group) in &by_identity {
        let mut by_family: BTreeMap<String, Vec<&AttachmentObservation>> = BTreeMap::new();
        for o in group {
            by_family
                .entry(family_pattern(&o.filename).to_lowercase())
                .or_default()
                .push(o);
        }

        let mut recurring: Vec<(Vec<&AttachmentObservation>, Option<String>)> = by_family
            .values()
            .filter(|family| is_recurring(family, &params))
            .map(|family| {
                let newest = family.iter().max_by_key(|o| o.timestamp).map(|o| o.filename.as_str());
                (family.clone(), newest.map(family_pattern))
            })
            .collect();

        // No shared filename shape: fall back to the extension that recurs
        // most. A pattern-less rule would collect every attachment the sender
        // mails (logos, invites), so an identity whose documents never share
        // an extension proposes nothing.
        if recurring.is_empty() {
            let mut by_ext: BTreeMap<String, Vec<&AttachmentObservation>> = BTreeMap::new();
            for o in group {
                if let Some(ext) = extension(&o.filename) {
                    by_ext.entry(ext).or_default().push(o);
                }
            }
            let email_count =
                |members: &[&AttachmentObservation]| members.iter().map(|o| &o.email_id).collect::<HashSet<_>>().len();
            if let Some((ext, members)) = by_ext
                .into_iter()
                .filter(|(_, members)| is_recurring(members, &params))
                .max_by_key(|(_, members)| email_count(members))
            {
                recurring.push((members, Some(format!("*.{ext}"))));
            }
        }

        for (members, pattern) in recurring {
            if covered_by_existing_rule(&members, existing_rules)
                || similar_rule_exists(&members, existing_rules)
                || covered_by_resolved(identity, &members, resolved)
            {
                continue;
            }
            candidates.push(build_candidate(identity, &members, pattern));
        }
    }

    candidates.sort_by(|a, b| {
        b.email_count
            .cmp(&a.email_count)
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.key.cmp(&b.key))
    });
    candidates.truncate(params.max_suggestions);
    candidates
}

// --- Executor ---

/// How far back mining looks: long enough to see a quarterly document
/// recur, short enough that a sender who stopped mailing drops out.
const LOOKBACK_SECS: i64 = 548 * 86_400;

/// Re-mine the account and persist the result; returns the pending list.
pub fn refresh_suggestions(db: &crate::db::Database, account_id: &str) -> Result<Vec<AttachmentRuleSuggestion>> {
    refresh_suggestions_at(db, account_id, super::clock::now_secs())
}

/// Event the frontend listens to (payload: account id) to refresh the
/// suggestion badge after a background refresh.
pub const SUGGESTIONS_UPDATED_EVENT: &str = "attachment-rule-suggestions-updated";

/// Post-sync hook: re-mine and notify the frontend. Returns the number of
/// pending suggestions.
pub fn refresh_after_sync(
    db: &crate::db::Database,
    app: Option<&super::app_handle::AppHandle>,
    account_id: &str,
) -> Result<usize> {
    let pending = refresh_suggestions(db, account_id)?.len();
    if let Some(app) = app {
        if let Err(e) = app.emit(SUGGESTIONS_UPDATED_EVENT, account_id) {
            eprintln!("[attachment-suggestions] could not emit {SUGGESTIONS_UPDATED_EVENT}: {e}");
        }
    }
    Ok(pending)
}

pub fn refresh_suggestions_at(
    db: &crate::db::Database,
    account_id: &str,
    now: i64,
) -> Result<Vec<AttachmentRuleSuggestion>> {
    let candidates = preview_suggestions_at(db, account_id, now)?;
    db.replace_pending_attachment_rule_suggestions(account_id, &candidates, now)?;
    list_suggestions(db, account_id)
}

/// Mine the account without persisting anything — what a refresh at `now`
/// would propose. Backs the read-only `emailops-cli attachment-suggestions`.
pub fn preview_suggestions_at(
    db: &crate::db::Database,
    account_id: &str,
    now: i64,
) -> Result<Vec<SuggestionCandidate>> {
    let observations = db.get_attachment_observations(account_id, now - LOOKBACK_SECS)?;
    let rules = db.get_all_attachment_rules(account_id)?;
    let resolved = db.get_resolved_attachment_rule_suggestions(account_id)?;
    let account_email = db.get_account(account_id)?.map(|a| a.email).unwrap_or_default();
    Ok(plan_suggestions(
        &observations,
        &rules,
        &resolved,
        &account_email,
        SuggestionParams::default(),
    ))
}

pub fn list_suggestions(db: &crate::db::Database, account_id: &str) -> Result<Vec<AttachmentRuleSuggestion>> {
    db.get_pending_attachment_rule_suggestions(account_id)
}

pub fn set_suggestion_status(
    db: &crate::db::Database,
    account_id: &str,
    suggestion_id: &str,
    status: AttachmentRuleSuggestionStatus,
) -> Result<()> {
    if db.set_attachment_rule_suggestion_status(account_id, suggestion_id, status, super::clock::now_secs())? {
        Ok(())
    } else {
        Err(AppError::NotFound(format!(
            "Attachment rule suggestion {suggestion_id} not found"
        )))
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::db::Database;
    use crate::models::Email;

    const DAY: i64 = 86_400;
    /// 2026-06-15T00:00:00Z
    const NOW: i64 = 1_781_481_600;

    fn setup() -> Database {
        let db = Database::new_for_testing().expect("test db");
        for (id, email) in [("acc1", "me@example.com"), ("acc2", "other@example.com")] {
            db.connection()
                .execute(
                    "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled, sync_from_timestamp) \
                     VALUES (?1, 'gmail', ?2, 'Test', 0, 0, 1, NULL)",
                    rusqlite::params![id, email],
                )
                .expect("insert account");
        }
        db
    }

    fn add_email_with_pdf(db: &Database, id: &str, sender: &str, ts: i64, filename: &str, mailbox: &str) {
        add_email_with_pdf_for(db, "acc1", id, sender, ts, filename, mailbox);
    }

    fn add_email_with_pdf_for(
        db: &Database,
        account: &str,
        id: &str,
        sender: &str,
        ts: i64,
        filename: &str,
        mailbox: &str,
    ) {
        let email = Email {
            id: id.into(),
            account_id: account.into(),
            thread_id: id.into(),
            message_id: None,
            references: None,
            subject: "Monthly document".into(),
            sender: "Sender".into(),
            sender_email: sender.into(),
            recipients: vec!["me@example.com".into()],
            cc: vec![],
            body: "body".into(),
            snippet: "snippet".into(),
            timestamp: ts,
            is_read: false,
            triage_status: None,
            category: "primary".into(),
            mailbox: mailbox.into(),
            is_sent: mailbox == "sent",
            headers: None,
        };
        db.insert_email(&email).expect("insert email");
        db.insert_email_attachment_metas_batch(&[(
            id.into(),
            account.into(),
            format!("att-{id}"),
            filename.into(),
            "application/pdf".into(),
            1000,
            None,
        )])
        .expect("insert meta");
    }

    /// Three monthly invoices from billing@acme.com, the latest a month ago.
    fn add_monthly_invoices(db: &Database, mailbox: &str) {
        for i in 0..3 {
            add_email_with_pdf(
                db,
                &format!("{mailbox}-{i}"),
                "billing@acme.com",
                NOW - (i + 1) * 31 * DAY,
                &format!("Invoice_{i}0{i}.pdf"),
                mailbox,
            );
        }
    }

    #[test]
    fn refresh_persists_recurring_documents_as_pending_suggestions() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");

        let out = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh");

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sender_email_pattern, "billing@acme.com");
        assert_eq!(out[0].filename_pattern.as_deref(), Some("Invoice_*.pdf"));
        assert_eq!(out[0].status, AttachmentRuleSuggestionStatus::Pending);
        assert_eq!(list_suggestions(&db, "acc1").expect("list").len(), 1);
    }

    #[test]
    fn sent_spam_and_trash_mail_is_not_mined() {
        let db = setup();
        for mailbox in ["sent", "spam", "trash"] {
            add_monthly_invoices(&db, mailbox);
        }

        assert!(refresh_suggestions_at(&db, "acc1", NOW).expect("refresh").is_empty());
    }

    #[test]
    fn documents_older_than_the_lookback_window_are_ignored() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");

        let three_years_later = NOW + 3 * 365 * DAY;
        assert!(refresh_suggestions_at(&db, "acc1", three_years_later)
            .expect("refresh")
            .is_empty());
    }

    #[test]
    fn preview_reports_candidates_without_persisting_them() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");

        let preview = preview_suggestions_at(&db, "acc1", NOW).expect("preview");

        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].key, "*@acme.com|invoice_*.pdf");
        assert!(list_suggestions(&db, "acc1").expect("list").is_empty());
    }

    #[test]
    fn dismissed_suggestion_stays_hidden_after_refresh() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        let id = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh")[0].id.clone();

        set_suggestion_status(&db, "acc1", &id, AttachmentRuleSuggestionStatus::Dismissed).expect("dismiss");

        assert!(refresh_suggestions_at(&db, "acc1", NOW).expect("refresh").is_empty());
    }

    #[test]
    fn accepted_suggestion_leaves_the_pending_list() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        let id = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh")[0].id.clone();

        set_suggestion_status(&db, "acc1", &id, AttachmentRuleSuggestionStatus::Accepted).expect("accept");

        assert!(list_suggestions(&db, "acc1").expect("list").is_empty());
    }

    #[test]
    fn refresh_keeps_the_id_of_a_still_valid_suggestion() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        let first = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh")[0].id.clone();

        add_email_with_pdf(&db, "new", "billing@acme.com", NOW, "Invoice_999.pdf", "inbox");
        let again = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh");

        assert_eq!(again[0].id, first);
        assert_eq!(again[0].email_count, 4);
    }

    #[test]
    fn refresh_drops_a_pending_suggestion_once_a_rule_covers_it() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        refresh_suggestions_at(&db, "acc1", NOW).expect("refresh");

        db.insert_attachment_rule(&AttachmentRule {
            id: "rule-1".into(),
            account_id: "acc1".into(),
            name: "Acme".into(),
            sender_email_pattern: Some("*@acme.com".into()),
            subject_pattern: None,
            filename_pattern: None,
            tags: vec![],
            enabled: true,
            created_at: NOW,
            updated_at: NOW,
        })
        .expect("insert rule");

        assert!(refresh_suggestions_at(&db, "acc1", NOW).expect("refresh").is_empty());
    }

    #[test]
    fn documents_filed_in_custom_folders_are_mined() {
        let db = setup();
        add_monthly_invoices(&db, "folder:Facturas");

        assert_eq!(refresh_suggestions_at(&db, "acc1", NOW).expect("refresh").len(), 1);
    }

    #[test]
    fn mining_one_account_ignores_another_accounts_documents() {
        let db = setup();
        for i in 0..3 {
            add_email_with_pdf_for(
                &db,
                "acc2",
                &format!("other-{i}"),
                "billing@acme.com",
                NOW - (i + 1) * 31 * DAY,
                &format!("Invoice_{i}.pdf"),
                "inbox",
            );
        }

        assert!(refresh_suggestions_at(&db, "acc1", NOW).expect("refresh").is_empty());
        assert_eq!(refresh_suggestions_at(&db, "acc2", NOW).expect("refresh").len(), 1);
    }

    #[test]
    fn a_dismissal_survives_a_new_sender_address_of_the_company() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        let id = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh")[0].id.clone();
        set_suggestion_status(&db, "acc1", &id, AttachmentRuleSuggestionStatus::Dismissed).expect("dismiss");

        add_email_with_pdf(&db, "new", "noreply@acme.com", NOW, "Invoice_999.pdf", "inbox");

        assert!(refresh_suggestions_at(&db, "acc1", NOW).expect("refresh").is_empty());
    }

    #[test]
    fn a_corrupt_suggestion_row_is_reported_not_silently_emptied() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        refresh_suggestions_at(&db, "acc1", NOW).expect("refresh");
        db.connection()
            .execute("UPDATE attachment_rule_suggestions SET tags_json = 'not json'", [])
            .expect("corrupt");

        assert!(list_suggestions(&db, "acc1").is_err());
    }

    #[test]
    fn status_change_on_another_accounts_suggestion_is_not_found() {
        let db = setup();
        add_monthly_invoices(&db, "inbox");
        let id = refresh_suggestions_at(&db, "acc1", NOW).expect("refresh")[0].id.clone();

        let err = set_suggestion_status(&db, "acc2", &id, AttachmentRuleSuggestionStatus::Dismissed)
            .expect_err("foreign account");

        assert!(matches!(err, AppError::NotFound(_)));
        assert_eq!(list_suggestions(&db, "acc1").expect("list").len(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    /// 2026-01-15T00:00:00Z
    const JAN_15: i64 = 1_768_435_200;
    /// The mailbox being mined — unrelated to every sender below.
    const ACCOUNT: &str = "me@example.org";

    fn resolved(sender: &str, filename: Option<&str>) -> ResolvedSuggestion {
        ResolvedSuggestion {
            sender_email_pattern: sender.into(),
            filename_pattern: filename.map(Into::into),
        }
    }

    fn obs(email_id: &str, sender: &str, ts: i64, filename: &str) -> AttachmentObservation {
        AttachmentObservation {
            email_id: email_id.to_string(),
            sender_email: sender.to_string(),
            sender_name: String::new(),
            subject: "Your document".to_string(),
            timestamp: ts,
            filename: filename.to_string(),
            mime_type: "application/pdf".to_string(),
        }
    }

    fn monthly(sender: &str, prefix: &str, n: usize) -> Vec<AttachmentObservation> {
        (0..n)
            .map(|i| {
                obs(
                    &format!("{sender}-{i}"),
                    sender,
                    JAN_15 + (i as i64) * 31 * DAY,
                    &format!("{prefix}_2026-0{}_00{}.pdf", i + 1, 10 + i),
                )
            })
            .collect()
    }

    fn rule(sender: &str, filename: Option<&str>) -> AttachmentRule {
        AttachmentRule {
            id: "r1".into(),
            account_id: "acc".into(),
            name: "existing".into(),
            sender_email_pattern: Some(sender.into()),
            subject_pattern: None,
            filename_pattern: filename.map(Into::into),
            tags: vec![],
            enabled: true,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn plan(observations: &[AttachmentObservation]) -> Vec<SuggestionCandidate> {
        plan_suggestions(observations, &[], &[], ACCOUNT, SuggestionParams::default())
    }

    #[test]
    fn document_detection_accepts_office_pdf_and_einvoice_formats() {
        for (mime, name) in [
            ("application/pdf", "a.pdf"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "a.docx",
            ),
            ("application/vnd.ms-excel", "a.xls"),
            ("application/vnd.oasis.opendocument.text", "a.odt"),
            ("application/xml", "factura.xml"),
            ("application/zip", "facturae.zip"),
            ("application/octet-stream", "Statement.PDF"),
        ] {
            assert!(is_document_attachment(mime, name), "{mime} {name}");
        }
    }

    #[test]
    fn document_detection_rejects_images_calendar_and_signatures() {
        for (mime, name) in [
            ("image/png", "logo.png"),
            ("image/jpeg", "photo.jpg"),
            ("text/calendar", "invite.ics"),
            ("application/pkcs7-signature", "smime.p7s"),
            ("application/octet-stream", "blob.bin"),
        ] {
            assert!(!is_document_attachment(mime, name), "{mime} {name}");
        }
    }

    #[test]
    fn filename_family_collapses_numbers_and_dates_into_one_wildcard() {
        assert_eq!(filename_family("Factura_2026-03_0012.pdf"), "Factura_*.pdf");
        assert_eq!(filename_family("invoice-123.PDF"), "invoice-*.pdf");
        assert_eq!(filename_family("Statement 03.2026.pdf"), "Statement *.pdf");
        assert_eq!(filename_family("contract.pdf"), "contract.pdf");
    }

    #[test]
    fn filename_family_treats_month_names_as_variable() {
        assert_eq!(filename_family("borgbase-invoice-jan.pdf"), "borgbase-invoice-*.pdf");
        assert_eq!(filename_family("Factura_Enero_2026.pdf"), "Factura_*.pdf");
        assert_eq!(filename_family("Relevé Mars 2026.pdf"), "Relevé *.pdf");
        assert_eq!(filename_family("Rechnung-März-2026.pdf"), "Rechnung-*.pdf");
    }

    #[test]
    fn filename_family_keeps_month_letters_inside_words() {
        assert_eq!(filename_family("summary.pdf"), "summary.pdf");
        assert_eq!(filename_family("mayor-report.pdf"), "mayor-report.pdf");
    }

    #[test]
    fn filename_family_collapses_an_iso_timestamp_into_one_wildcard() {
        assert_eq!(
            filename_family("cursor_analytics_2025-10-09T09:14:00Z.csv"),
            "cursor_analytics_*.csv"
        );
        assert_eq!(filename_family("EMI 1T.pdf"), "EMI *T.pdf");
    }

    #[test]
    fn a_family_with_almost_no_fixed_text_falls_back_to_the_extension() {
        let o = vec![
            obs("e1", "office@acme.com", JAN_15, "12F34.pdf"),
            obs("e2", "office@acme.com", JAN_15 + 40 * DAY, "56F78.pdf"),
        ];

        assert_eq!(plan(&o)[0].filename_pattern.as_deref(), Some("*.pdf"));
    }

    #[test]
    fn candidates_are_named_after_the_registrable_domain_not_the_mail_subdomain() {
        let out = plan(&monthly("invoice@mail.acme.com", "Invoice", 2));

        assert_eq!(out[0].name, "Acme · invoice");
        assert_eq!(out[0].tags, vec!["invoice".to_string(), "acme".to_string()]);
    }

    #[test]
    fn a_two_level_public_suffix_is_skipped_when_naming() {
        let out = plan(&monthly("billing@shop.acme.co.uk", "Invoice", 2));

        assert_eq!(out[0].name, "Acme · invoice");
    }

    #[test]
    fn recurring_monthly_invoices_from_one_sender_become_a_candidate() {
        let out = plan(&monthly("billing@acme.com", "Factura", 3));

        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.sender_email_pattern, "billing@acme.com");
        assert_eq!(c.filename_pattern.as_deref(), Some("Factura_*.pdf"));
        assert_eq!(c.email_count, 3);
        assert_eq!(c.first_seen, JAN_15);
        assert_eq!(c.key, "*@acme.com|factura_*.pdf");
    }

    #[test]
    fn candidate_tags_carry_document_kind_and_company() {
        let out = plan(&monthly("billing@acme.com", "Factura", 3));

        assert_eq!(out[0].tags, vec!["invoice".to_string(), "acme".to_string()]);
        assert_eq!(out[0].name, "Acme · invoice");
    }

    #[test]
    fn a_single_email_is_not_recurring() {
        assert!(plan(&monthly("billing@acme.com", "Factura", 1)).is_empty());
    }

    #[test]
    fn two_monthly_invoices_are_already_recurring() {
        let out = plan(&monthly("billing@acme.com", "Factura", 2));

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename_pattern.as_deref(), Some("Factura_*.pdf"));
    }

    #[test]
    fn two_unrelated_documents_in_two_months_fall_back_to_an_extension_pattern() {
        let o = vec![
            obs("e1", "office@acme.com", JAN_15, "report.pdf"),
            obs("e2", "office@acme.com", JAN_15 + 40 * DAY, "minutes.pdf"),
        ];

        let out = plan(&o);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename_pattern.as_deref(), Some("*.pdf"));
    }

    #[test]
    fn a_personal_sender_candidate_is_named_after_their_display_name() {
        let mut o = monthly("ana@gmail.com", "Documento", 2);
        o[0].sender_name = "Ana Gestoría".into();
        o[1].sender_name = "Ana Gestoría".into();

        assert_eq!(plan(&o)[0].name, "Ana Gestoría");
    }

    #[test]
    fn a_personal_sender_without_display_name_is_named_after_the_address() {
        let o = monthly("ana@gmail.com", "Documento", 2);

        assert_eq!(plan(&o)[0].name, "ana@gmail.com");
    }

    #[test]
    fn emails_within_a_single_month_are_not_recurring() {
        let o: Vec<_> = (0..4)
            .map(|i| obs(&format!("e{i}"), "a@acme.com", JAN_15 + i * DAY, &format!("doc{i}.pdf")))
            .collect();
        assert!(plan(&o).is_empty());
    }

    #[test]
    fn several_attachments_on_one_email_count_once() {
        let o: Vec<_> = (0..3)
            .map(|i| obs("same", "a@acme.com", JAN_15 + i * 40 * DAY, &format!("inv{i}.pdf")))
            .collect();
        assert!(plan(&o).is_empty());
    }

    #[test]
    fn non_document_attachments_are_ignored() {
        let mut o = monthly("billing@acme.com", "Factura", 3);
        for x in &mut o {
            x.mime_type = "image/png".into();
            x.filename = x.filename.replace(".pdf", ".png");
        }
        assert!(plan(&o).is_empty());
    }

    #[test]
    fn senders_of_the_same_company_domain_merge_into_a_domain_pattern() {
        let mut o = monthly("billing@acme.com", "Invoice", 2);
        let mut other = monthly("noreply@acme.com", "Invoice", 4);
        other.drain(..2);
        o.extend(other);

        let out = plan(&o);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sender_email_pattern, "*@acme.com");
        assert_eq!(out[0].email_count, 4);
    }

    #[test]
    fn personal_domain_senders_are_never_merged() {
        let mut o = monthly("ana@gmail.com", "Invoice", 1);
        let mut other = monthly("luis@gmail.com", "Invoice", 2);
        other.drain(..1);
        o.extend(other);

        assert!(plan(&o).is_empty());
    }

    #[test]
    fn unrelated_filenames_fall_back_to_an_extension_pattern() {
        let names = ["report.pdf", "summary.pdf", "minutes.pdf"];
        let o: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, n)| obs(&format!("e{i}"), "office@acme.com", JAN_15 + (i as i64) * 40 * DAY, n))
            .collect();

        let out = plan(&o);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename_pattern.as_deref(), Some("*.pdf"));
    }

    #[test]
    fn a_recurring_family_suppresses_the_extension_fallback_for_that_sender() {
        let mut o = monthly("billing@acme.com", "Factura", 3);
        o.push(obs("x1", "billing@acme.com", JAN_15 + 100 * DAY, "terms.pdf"));

        let out = plan(&o);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename_pattern.as_deref(), Some("Factura_*.pdf"));
    }

    #[test]
    fn a_sender_with_two_recurring_families_yields_two_candidates() {
        let mut o = monthly("hr@acme.com", "Payslip", 3);
        o.extend(monthly("hr@acme.com", "Pension", 3).into_iter().map(|mut x| {
            x.email_id = format!("p-{}", x.email_id);
            x
        }));

        assert_eq!(plan(&o).len(), 2);
    }

    #[test]
    fn existing_rule_covering_the_documents_suppresses_the_candidate() {
        let o = monthly("billing@acme.com", "Factura", 3);
        for r in [rule("billing@acme.com", None), rule("*@acme.com", Some("*.pdf"))] {
            let out = plan_suggestions(&o, &[r], &[], ACCOUNT, SuggestionParams::default());
            assert!(out.is_empty());
        }
    }

    #[test]
    fn any_rule_for_the_same_sender_suppresses_the_candidate() {
        let o = monthly("billing@acme.com", "Factura", 3);
        let out = plan_suggestions(
            &o,
            &[rule("billing@acme.com", Some("contract*.pdf"))],
            &[],
            ACCOUNT,
            SuggestionParams::default(),
        );
        assert!(out.is_empty(), "the user already handles this sender");
    }

    #[test]
    fn a_rule_for_another_address_of_the_same_organisation_suppresses_the_candidate() {
        let o = monthly("billing@acme.com", "Factura", 3);
        let out = plan_suggestions(
            &o,
            &[rule("invoicing@email.acme.com", None)],
            &[],
            ACCOUNT,
            SuggestionParams::default(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_rule_for_another_person_on_a_personal_provider_does_not_suppress() {
        let o = monthly("ana@gmail.com", "Documento", 2);
        let out = plan_suggestions(
            &o,
            &[rule("luis@gmail.com", None)],
            &[],
            ACCOUNT,
            SuggestionParams::default(),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_rule_for_an_unrelated_sender_does_not_suppress() {
        let o = monthly("billing@acme.com", "Factura", 3);
        let out = plan_suggestions(
            &o,
            &[rule("*@globex.com", None)],
            &[],
            ACCOUNT,
            SuggestionParams::default(),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn excluded_keys_are_not_proposed_again() {
        let o = monthly("billing@acme.com", "Factura", 3);
        let resolved = [resolved("billing@acme.com", Some("Factura_*.pdf"))];
        let out = plan_suggestions(&o, &[], &resolved, ACCOUNT, SuggestionParams::default());
        assert!(out.is_empty());
    }

    // --- Dismissals stick while the group drifts ---

    #[test]
    fn the_key_stays_the_same_when_a_second_sender_of_the_domain_joins() {
        let alone = plan(&monthly("billing@acme.com", "Factura", 2));
        let mut o = monthly("billing@acme.com", "Factura", 2);
        o.push(obs(
            "n1",
            "noreply@acme.com",
            JAN_15 + 70 * DAY,
            "Factura_2026-03_0099.pdf",
        ));

        assert_eq!(plan(&o)[0].key, alone[0].key);
    }

    #[test]
    fn a_dismissal_survives_a_second_sender_of_the_domain_joining() {
        let mut o = monthly("billing@acme.com", "Factura", 2);
        o.push(obs(
            "n1",
            "noreply@acme.com",
            JAN_15 + 70 * DAY,
            "Factura_2026-03_0099.pdf",
        ));

        let dismissed = [resolved("billing@acme.com", Some("Factura_*.pdf"))];
        let out = plan_suggestions(&o, &[], &dismissed, ACCOUNT, SuggestionParams::default());

        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_dismissed_extension_fallback_hides_a_family_that_recurs_later() {
        let o = monthly("office@acme.com", "Invoice", 3);

        let dismissed = [resolved("office@acme.com", Some("*.pdf"))];
        let out = plan_suggestions(&o, &[], &dismissed, ACCOUNT, SuggestionParams::default());

        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_dismissal_does_not_hide_another_document_family_of_the_sender() {
        let o = monthly("billing@acme.com", "Factura", 3);

        let dismissed = [resolved("billing@acme.com", Some("Contract_*.pdf"))];
        let out = plan_suggestions(&o, &[], &dismissed, ACCOUNT, SuggestionParams::default());

        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_dismissal_does_not_hide_another_sender() {
        let o = monthly("billing@globex.com", "Factura", 3);

        let dismissed = [resolved("billing@acme.com", Some("Factura_*.pdf"))];
        let out = plan_suggestions(&o, &[], &dismissed, ACCOUNT, SuggestionParams::default());

        assert_eq!(out.len(), 1);
    }

    // --- Mixed extensions ---

    #[test]
    fn mixed_extensions_fall_back_to_the_recurring_extension_only() {
        let o = vec![
            obs("e1", "office@acme.com", JAN_15, "report.pdf"),
            obs("e2", "office@acme.com", JAN_15 + 30 * DAY, "notes.docx"),
            obs("e3", "office@acme.com", JAN_15 + 60 * DAY, "minutes.pdf"),
        ];

        let out = plan(&o);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filename_pattern.as_deref(), Some("*.pdf"));
        assert_eq!(out[0].email_count, 2);
    }

    #[test]
    fn mixed_extensions_without_a_recurring_one_propose_nothing() {
        let o = vec![
            obs("e1", "office@acme.com", JAN_15, "report.pdf"),
            obs("e2", "office@acme.com", JAN_15 + 40 * DAY, "notes.docx"),
        ];

        assert!(
            plan(&o).is_empty(),
            "a rule without a filename pattern would collect every attachment"
        );
    }

    // --- The user's own mail ---

    #[test]
    fn the_users_own_address_is_never_a_candidate() {
        let o = monthly("Me@Example.org", "Scan", 3);

        assert!(plan(&o).is_empty());
    }

    #[test]
    fn colleagues_at_the_users_own_corporate_domain_are_not_candidates() {
        let mut o = monthly("ana@example.org", "Report", 2);
        o.extend(monthly("luis@eu.example.org", "Budget", 2));

        assert!(plan(&o).is_empty(), "{:?}", plan(&o));
    }

    #[test]
    fn a_personal_provider_account_still_gets_other_peoples_documents() {
        let o = monthly("ana@gmail.com", "Documento", 2);
        let out = plan_suggestions(&o, &[], &[], "me@gmail.com", SuggestionParams::default());

        assert_eq!(out.len(), 1);
    }

    // --- Recurrence ---

    #[test]
    fn a_burst_across_a_month_boundary_is_not_recurring() {
        // 2026-01-31 and 2026-02-01: two calendar months, one conversation.
        let jan_31 = JAN_15 + 16 * DAY;
        let o = vec![
            obs("e1", "billing@acme.com", jan_31, "Factura_0001.pdf"),
            obs("e2", "billing@acme.com", jan_31 + DAY, "Factura_0002.pdf"),
        ];

        assert!(plan(&o).is_empty());
    }

    // --- Kind tags ---

    #[test]
    fn kind_keywords_only_match_whole_words() {
        let out = plan(&monthly("x@acme.com", "Contractor", 2));
        assert_eq!(out[0].tags, vec!["acme".to_string()]);

        let out = plan(&monthly("x@acme.com", "Denominacion", 2));
        assert_eq!(out[0].tags, vec!["acme".to_string()]);
    }

    #[test]
    fn kind_keywords_match_next_to_separators_and_digits() {
        for prefix in ["Invoice", "invoice2026", "Mi-Nomina"] {
            let out = plan(&monthly("x@acme.com", prefix, 2));
            assert_ne!(out[0].tags, vec!["acme".to_string()], "{prefix}");
        }
    }

    // --- Input hygiene ---

    #[test]
    fn senders_without_a_domain_are_ignored() {
        for sender in ["", "undisclosed-recipients"] {
            assert!(plan(&monthly(sender, "Factura", 3)).is_empty(), "{sender:?}");
        }
    }

    #[test]
    fn german_and_french_personal_providers_are_not_pooled() {
        for domain in [
            "web.de",
            "t-online.de",
            "orange.fr",
            "free.fr",
            "laposte.net",
            "libero.it",
        ] {
            let mut o = monthly(&format!("ana@{domain}"), "Scan", 1);
            let mut other = monthly(&format!("luis@{domain}"), "Scan", 2);
            other.drain(..1);
            o.extend(other);

            assert!(plan(&o).is_empty(), "{domain} pooled two people");
        }
    }

    #[test]
    fn candidates_are_ranked_by_email_count_and_capped() {
        let mut o = monthly("a@alpha.com", "Invoice", 3);
        o.extend(monthly("b@beta.com", "Invoice", 5));
        o.extend(monthly("c@gamma.com", "Invoice", 4));

        let params = SuggestionParams {
            max_suggestions: 2,
            ..SuggestionParams::default()
        };
        let out = plan_suggestions(&o, &[], &[], ACCOUNT, params);

        let senders: Vec<_> = out.iter().map(|c| c.sender_email_pattern.as_str()).collect();
        assert_eq!(senders, vec!["b@beta.com", "c@gamma.com"]);
    }

    #[test]
    fn sample_filenames_are_most_recent_first_and_deduplicated() {
        let out = plan(&monthly("billing@acme.com", "Factura", 5));
        assert_eq!(out[0].sample_filenames.len(), 3);
        assert_eq!(out[0].sample_filenames[0], "Factura_2026-05_0014.pdf");
    }
}
