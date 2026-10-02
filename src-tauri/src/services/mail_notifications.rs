//! Desktop notifications for new mail (and for snoozed conversations coming
//! back).
//!
//! Pure planners decide what to show; thin executors read the inputs from the
//! DB, ask the [`Notifier`] for focus, show the result and persist the
//! rate-limit marker.
//!
//! What notifies (see `plan_new_mail_notifications`): mail the incremental
//! pass of a sync downloaded — never the first sync of an account, a backfill,
//! or anything older than the inbox watermark the sync started from — that is
//! in the inbox, unread, not sent by the user, not a promotion, and not
//! flagged junk (which includes a blocked sender's mail, filed in Spam by the
//! sync hook that runs before this one).
//!
//! Privacy: the body text is never shown; "hidden" mode (or a locked app)
//! shows only the account. Nothing here logs a subject or a sender.

use std::collections::HashMap;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Account, Email};
use crate::services::emails::ThreadRef;
use crate::services::i18n::Language;
use crate::services::logger;
use crate::services::notifier::{DesktopNotification, Notifier};

/// Master switch (`true`/`false`, default on).
pub const PREF_ENABLED: &str = "notifications.new_mail.enabled";
/// Per-account switch prefix: `notifications.new_mail.account:<account_id>`
/// (`true`/`false`, default on).
pub const PREF_ACCOUNT_PREFIX: &str = "notifications.new_mail.account:";
/// `preview` (sender + subject, default) or `hidden` (just the account).
pub const PREF_CONTENT: &str = "notifications.new_mail.content";
/// Only notify while no EmailOps window has focus (`true`/`false`, default on).
pub const PREF_ONLY_UNFOCUSED: &str = "notifications.new_mail.only_unfocused";
/// Backend-owned: when the last summary notification for an account was shown.
const LAST_SUMMARY_PREFIX: &str = "notifications.new_mail.last_summary_at:";

/// Up to this many new messages in one sync notify one by one; more become a
/// single summary.
pub const MAX_INDIVIDUAL: usize = 3;
/// At most one summary per account in this many seconds.
pub const SUMMARY_COOLDOWN_SECS: i64 = 60;
/// Longest subject shown, in characters.
const MAX_SUBJECT_CHARS: usize = 120;

/// What a notification may reveal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    /// Sender as the title, subject as the body.
    Preview,
    /// "New message in <account>", nothing about the message.
    Hidden,
}

impl ContentMode {
    pub fn from_pref(value: Option<&str>) -> Self {
        match value {
            Some("hidden") => ContentMode::Hidden,
            _ => ContentMode::Preview,
        }
    }
}

/// The user's notification preferences, resolved for one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyPrefs {
    pub enabled: bool,
    pub account_enabled: bool,
    pub content: ContentMode,
    pub only_when_unfocused: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            account_enabled: true,
            content: ContentMode::Preview,
            only_when_unfocused: true,
        }
    }
}

/// One message a sync stored, with what the planner needs to judge it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailCandidate {
    pub email_id: String,
    pub thread_id: String,
    pub sender: String,
    pub sender_email: String,
    pub subject: String,
    pub timestamp: i64,
    pub mailbox: String,
    pub category: String,
    pub is_read: bool,
    /// Sent by the user (provider Sent flag, or the account's own address).
    pub from_self: bool,
    /// Listed by the sync's backfill slice rather than its incremental pass.
    pub from_backfill: bool,
    /// The junk detector (or a block / the user) flags it.
    pub junk_flagged: bool,
}

impl MailCandidate {
    /// Build from a freshly stored row. `junk_flagged` is filled in later by
    /// the executor, once scoring and the blocked-sender hook have run.
    pub fn from_email(email: &Email, account_email: &str, from_backfill: bool) -> Self {
        Self {
            email_id: email.id.clone(),
            thread_id: email.thread_id.clone(),
            sender: email.sender.clone(),
            sender_email: email.sender_email.clone(),
            subject: email.subject.clone(),
            timestamp: email.timestamp,
            mailbox: email.mailbox.clone(),
            category: email.category.clone(),
            is_read: email.is_read,
            from_self: email.is_sent || email.sender_email.eq_ignore_ascii_case(account_email),
            from_backfill,
            junk_flagged: false,
        }
    }
}

/// Everything about the account and the moment that shapes the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountNotifyState {
    pub account_id: String,
    /// Shown in summaries and hidden-content notifications.
    pub account_label: String,
    /// Newest inbox timestamp before the sync started; `None` = the account's
    /// first sync, which never notifies.
    pub watermark: Option<i64>,
    pub last_summary_at: Option<i64>,
    pub now: i64,
    pub app_focused: bool,
    pub app_locked: bool,
    pub language: Language,
}

/// Whether one stored message deserves a notification at all.
fn is_notifiable(candidate: &MailCandidate, watermark: i64) -> bool {
    candidate.mailbox == "inbox"
        && !candidate.from_backfill
        && candidate.timestamp >= watermark
        && !candidate.is_read
        && !candidate.from_self
        && !candidate.junk_flagged
        && candidate.category != "promotions"
}

/// Whether the user wants anything shown right now, whatever arrived.
fn notifications_wanted(prefs: &NotifyPrefs, app_focused: bool) -> bool {
    prefs.enabled && prefs.account_enabled && !(prefs.only_when_unfocused && app_focused)
}

/// Decide the notifications for one account's sync batch: one per message for
/// up to [`MAX_INDIVIDUAL`], a single summary above that (at most one per
/// [`SUMMARY_COOLDOWN_SECS`] per account).
pub fn plan_new_mail_notifications(
    batch: &[MailCandidate],
    prefs: &NotifyPrefs,
    state: &AccountNotifyState,
) -> Vec<DesktopNotification> {
    let Some(watermark) = state.watermark else {
        return Vec::new();
    };
    if !notifications_wanted(prefs, state.app_focused) {
        return Vec::new();
    }
    let fresh: Vec<&MailCandidate> = batch.iter().filter(|c| is_notifiable(c, watermark)).collect();
    if fresh.is_empty() {
        return Vec::new();
    }
    let content = if state.app_locked {
        ContentMode::Hidden
    } else {
        prefs.content
    };
    let texts = Texts::for_language(state.language);

    if fresh.len() > MAX_INDIVIDUAL {
        let cooling = state
            .last_summary_at
            .is_some_and(|last| state.now - last < SUMMARY_COOLDOWN_SECS);
        if cooling {
            return Vec::new();
        }
        return vec![DesktopNotification {
            title: texts.app_name.to_string(),
            body: (texts.new_messages_in)(fresh.len(), &state.account_label),
            thread: None,
        }];
    }

    fresh
        .into_iter()
        .map(|c| {
            let (title, body) = match content {
                ContentMode::Preview => (sender_title(c), subject_body(&c.subject, &texts)),
                ContentMode::Hidden => (texts.app_name.to_string(), (texts.new_message_in)(&state.account_label)),
            };
            DesktopNotification {
                title,
                body,
                thread: Some(ThreadRef {
                    account_id: state.account_id.clone(),
                    thread_id: c.thread_id.clone(),
                }),
            }
        })
        .collect()
}

/// A conversation a snooze just brought back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozeReturn {
    pub thread: ThreadRef,
    pub subject: String,
    pub account_label: String,
}

/// Global state for the snooze planner: preferences are per account, the rest
/// is the moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozeNotifyState {
    pub app_focused: bool,
    pub app_locked: bool,
    pub language: Language,
}

/// "Snoozed conversation is back" — one per conversation for up to
/// [`MAX_INDIVIDUAL`], one summary above that. `prefs_for` resolves the
/// preferences of the conversation's account.
pub fn plan_snooze_return_notifications(
    returns: &[SnoozeReturn],
    prefs_for: &dyn Fn(&str) -> NotifyPrefs,
    state: &SnoozeNotifyState,
) -> Vec<DesktopNotification> {
    let texts = Texts::for_language(state.language);
    let wanted: Vec<(&SnoozeReturn, NotifyPrefs)> = returns
        .iter()
        .map(|r| (r, prefs_for(&r.thread.account_id)))
        .filter(|(_, prefs)| notifications_wanted(prefs, state.app_focused))
        .collect();
    if wanted.len() > MAX_INDIVIDUAL {
        return vec![DesktopNotification {
            title: texts.app_name.to_string(),
            body: (texts.snoozed_back_many)(wanted.len()),
            thread: None,
        }];
    }
    wanted
        .into_iter()
        .map(|(r, prefs)| {
            let hidden = state.app_locked || prefs.content == ContentMode::Hidden;
            DesktopNotification {
                title: texts.snoozed_back.to_string(),
                body: if hidden {
                    r.account_label.clone()
                } else {
                    subject_body(&r.subject, &texts)
                },
                thread: Some(r.thread.clone()),
            }
        })
        .collect()
}

fn sender_title(candidate: &MailCandidate) -> String {
    let name = candidate.sender.trim();
    if !name.is_empty() {
        return one_line(name, MAX_SUBJECT_CHARS);
    }
    one_line(candidate.sender_email.trim(), MAX_SUBJECT_CHARS)
}

fn subject_body(subject: &str, texts: &Texts) -> String {
    let line = one_line(subject, MAX_SUBJECT_CHARS);
    if line.is_empty() {
        texts.no_subject.to_string()
    } else {
        line
    }
}

/// Collapse whitespace (subjects can carry folded newlines) and cut to
/// `max_chars` on a character boundary.
fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The fixed strings a notification is built from, per UI language.
struct Texts {
    app_name: &'static str,
    no_subject: &'static str,
    snoozed_back: &'static str,
    new_message_in: fn(&str) -> String,
    new_messages_in: fn(usize, &str) -> String,
    snoozed_back_many: fn(usize) -> String,
}

impl Texts {
    fn for_language(language: Language) -> Self {
        match language {
            Language::En => Texts {
                app_name: "EmailOps",
                no_subject: "(no subject)",
                snoozed_back: "Snoozed conversation is back",
                new_message_in: |account| format!("New message in {account}"),
                new_messages_in: |n, account| format!("{n} new messages in {account}"),
                snoozed_back_many: |n| format!("{n} snoozed conversations are back"),
            },
            Language::Es => Texts {
                app_name: "EmailOps",
                no_subject: "(sin asunto)",
                snoozed_back: "Ha vuelto una conversación pospuesta",
                new_message_in: |account| format!("Nuevo mensaje en {account}"),
                new_messages_in: |n, account| format!("{n} mensajes nuevos en {account}"),
                snoozed_back_many: |n| format!("Han vuelto {n} conversaciones pospuestas"),
            },
            Language::Fr => Texts {
                app_name: "EmailOps",
                no_subject: "(sans objet)",
                snoozed_back: "Une conversation mise en attente est de retour",
                new_message_in: |account| format!("Nouveau message dans {account}"),
                new_messages_in: |n, account| format!("{n} nouveaux messages dans {account}"),
                snoozed_back_many: |n| format!("{n} conversations mises en attente sont de retour"),
            },
            Language::De => Texts {
                app_name: "EmailOps",
                no_subject: "(kein Betreff)",
                snoozed_back: "Zurückgestellte Unterhaltung ist wieder da",
                new_message_in: |account| format!("Neue Nachricht in {account}"),
                new_messages_in: |n, account| format!("{n} neue Nachrichten in {account}"),
                snoozed_back_many: |n| format!("{n} zurückgestellte Unterhaltungen sind wieder da"),
            },
        }
    }
}

// ── Executors ────────────────────────────────────────────────────────────────

fn bool_pref(db: &Database, key: &str) -> Result<bool> {
    Ok(db.get_preference(key)?.as_deref() != Some("false"))
}

/// The user's preferences for one account (defaults: everything on, preview).
pub fn load_prefs(db: &Database, account_id: &str) -> Result<NotifyPrefs> {
    Ok(NotifyPrefs {
        enabled: bool_pref(db, PREF_ENABLED)?,
        account_enabled: bool_pref(db, &format!("{PREF_ACCOUNT_PREFIX}{account_id}"))?,
        content: ContentMode::from_pref(db.get_preference(PREF_CONTENT)?.as_deref()),
        only_when_unfocused: bool_pref(db, PREF_ONLY_UNFOCUSED)?,
    })
}

fn last_summary_key(account_id: &str) -> String {
    format!("{LAST_SUMMARY_PREFIX}{account_id}")
}

fn ui_language(db: &Database) -> Language {
    match crate::services::i18n::resolve_ui_language(db) {
        Ok(language) => language.unwrap_or(Language::En),
        Err(e) => {
            logger::log("warn", "system", format!("Notification language unavailable: {e}"));
            Language::En
        }
    }
}

/// Show `notifications`; log failures without their content. Returns how many
/// were shown.
fn show_all(notifier: &dyn Notifier, notifications: &[DesktopNotification]) -> usize {
    let mut shown = 0;
    for notification in notifications {
        match notifier.show(notification) {
            Ok(()) => shown += 1,
            Err(e) => logger::log("error", "system", format!("Desktop notification failed: {e}")),
        }
    }
    shown
}

/// After a sync's ingest (and its junk scoring and blocked-sender filing):
/// notify about the new mail in `candidates`. Never fails the sync — every
/// error is logged.
pub fn notify_new_mail(
    db: &Database,
    notifier: &dyn Notifier,
    account: &Account,
    watermark: Option<i64>,
    mut candidates: Vec<MailCandidate>,
    now: i64,
) {
    if candidates.is_empty() || watermark.is_none() {
        return;
    }
    if let Err(e) = try_notify_new_mail(db, notifier, account, watermark, &mut candidates, now) {
        logger::log(
            "error",
            "sync",
            format!("[{}] New-mail notifications skipped: {e}", account.email),
        );
    }
}

fn try_notify_new_mail(
    db: &Database,
    notifier: &dyn Notifier,
    account: &Account,
    watermark: Option<i64>,
    candidates: &mut [MailCandidate],
    now: i64,
) -> Result<()> {
    let prefs = load_prefs(db, &account.id)?;
    if !prefs.enabled || !prefs.account_enabled {
        return Ok(());
    }
    let ids: Vec<String> = candidates.iter().map(|c| c.email_id.clone()).collect();
    let verdicts = db.get_junk_verdicts_batch(&ids)?;
    for candidate in candidates.iter_mut() {
        candidate.junk_flagged = verdicts.get(&candidate.email_id).is_some_and(|v| v.is_flagged());
    }
    let state = AccountNotifyState {
        account_id: account.id.clone(),
        account_label: account.email.clone(),
        watermark,
        last_summary_at: db
            .get_preference(&last_summary_key(&account.id))?
            .and_then(|v| v.parse().ok()),
        now,
        app_focused: notifier.app_focused(),
        app_locked: crate::services::password::is_app_locked(db)?,
        language: ui_language(db),
    };
    let plan = plan_new_mail_notifications(candidates, &prefs, &state);
    if plan.is_empty() {
        return Ok(());
    }
    let shown = show_all(notifier, &plan);
    if plan.iter().any(|n| n.thread.is_none()) {
        db.set_preference(&last_summary_key(&account.id), &now.to_string())?;
    }
    logger::log(
        "debug",
        "sync",
        format!("[{}] {shown} new-mail notification(s) shown", account.email),
    );
    Ok(())
}

/// After snoozed conversations returned to the inbox: tell the user, behind
/// the same switches as new mail.
pub fn notify_snooze_returns(db: &Database, notifier: &dyn Notifier, woken: &[ThreadRef]) {
    if woken.is_empty() {
        return;
    }
    if let Err(e) = try_notify_snooze_returns(db, notifier, woken) {
        logger::log("error", "system", format!("Snooze notifications skipped: {e}"));
    }
}

fn try_notify_snooze_returns(db: &Database, notifier: &dyn Notifier, woken: &[ThreadRef]) -> Result<()> {
    let labels: HashMap<String, String> = db.list_accounts()?.into_iter().map(|a| (a.id, a.email)).collect();
    let mut prefs: HashMap<String, NotifyPrefs> = HashMap::new();
    let mut returns = Vec::with_capacity(woken.len());
    for thread in woken {
        if !prefs.contains_key(&thread.account_id) {
            prefs.insert(thread.account_id.clone(), load_prefs(db, &thread.account_id)?);
        }
        let subject = db
            .get_thread(&thread.account_id, &thread.thread_id)?
            .into_iter()
            .max_by_key(|e| e.timestamp)
            .map(|e| e.subject)
            .unwrap_or_default();
        returns.push(SnoozeReturn {
            thread: thread.clone(),
            subject,
            account_label: labels.get(&thread.account_id).cloned().unwrap_or_default(),
        });
    }
    let state = SnoozeNotifyState {
        app_focused: notifier.app_focused(),
        app_locked: crate::services::password::is_app_locked(db)?,
        language: ui_language(db),
    };
    let plan = plan_snooze_return_notifications(
        &returns,
        &|account_id| prefs.get(account_id).copied().unwrap_or_default(),
        &state,
    );
    show_all(notifier, &plan);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::notifier::VecNotifier;

    const NOW: i64 = 1_800_000_000;
    const WATERMARK: i64 = NOW - 600;

    fn mail(id: &str) -> MailCandidate {
        MailCandidate {
            email_id: id.into(),
            thread_id: format!("t-{id}"),
            sender: "Ana Ruiz".into(),
            sender_email: "ana@example.com".into(),
            subject: "Quarterly plans".into(),
            timestamp: NOW - 60,
            mailbox: "inbox".into(),
            category: "primary".into(),
            is_read: false,
            from_self: false,
            from_backfill: false,
            junk_flagged: false,
        }
    }

    fn state() -> AccountNotifyState {
        AccountNotifyState {
            account_id: "acc-1".into(),
            account_label: "me@example.com".into(),
            watermark: Some(WATERMARK),
            last_summary_at: None,
            now: NOW,
            app_focused: false,
            app_locked: false,
            language: Language::En,
        }
    }

    fn plan(batch: &[MailCandidate]) -> Vec<DesktopNotification> {
        plan_new_mail_notifications(batch, &NotifyPrefs::default(), &state())
    }

    #[test]
    fn a_new_unread_inbox_message_notifies_with_sender_and_subject() {
        assert_eq!(
            plan(&[mail("a")]),
            vec![DesktopNotification {
                title: "Ana Ruiz".into(),
                body: "Quarterly plans".into(),
                thread: Some(ThreadRef {
                    account_id: "acc-1".into(),
                    thread_id: "t-a".into()
                }),
            }]
        );
    }

    #[test]
    fn excluded_messages_never_notify() {
        type Tweak = fn(&mut MailCandidate);
        let cases: [(&str, Tweak); 9] = [
            ("already read elsewhere", |c| c.is_read = true),
            ("sent by the user", |c| c.from_self = true),
            ("listed by the backfill", |c| c.from_backfill = true),
            ("older than the watermark", |c| c.timestamp = WATERMARK - 1),
            ("flagged junk / blocked", |c| c.junk_flagged = true),
            ("filed in spam", |c| c.mailbox = "spam".into()),
            ("in sent", |c| c.mailbox = "sent".into()),
            ("a draft folder", |c| c.mailbox = "drafts".into()),
            ("a promotion", |c| c.category = "promotions".into()),
        ];
        for (why, tweak) in cases {
            let mut c = mail("a");
            tweak(&mut c);
            assert!(plan(&[c]).is_empty(), "{why} must not notify");
        }
    }

    #[test]
    fn a_message_at_the_watermark_still_notifies() {
        let mut c = mail("a");
        c.timestamp = WATERMARK;
        assert_eq!(plan(&[c]).len(), 1);
    }

    #[test]
    fn the_first_sync_of_an_account_never_notifies() {
        let s = AccountNotifyState {
            watermark: None,
            ..state()
        };
        assert!(plan_new_mail_notifications(&[mail("a")], &NotifyPrefs::default(), &s).is_empty());
    }

    #[test]
    fn switches_and_focus_silence_everything() {
        let off = |p: NotifyPrefs| plan_new_mail_notifications(&[mail("a")], &p, &state()).is_empty();
        assert!(off(NotifyPrefs {
            enabled: false,
            ..NotifyPrefs::default()
        }));
        assert!(off(NotifyPrefs {
            account_enabled: false,
            ..NotifyPrefs::default()
        }));
        let focused = AccountNotifyState {
            app_focused: true,
            ..state()
        };
        assert!(plan_new_mail_notifications(&[mail("a")], &NotifyPrefs::default(), &focused).is_empty());
        let anytime = NotifyPrefs {
            only_when_unfocused: false,
            ..NotifyPrefs::default()
        };
        assert_eq!(plan_new_mail_notifications(&[mail("a")], &anytime, &focused).len(), 1);
    }

    #[test]
    fn up_to_three_messages_notify_one_by_one_and_more_coalesce() {
        let three: Vec<_> = ["a", "b", "c"].iter().map(|id| mail(id)).collect();
        assert_eq!(plan(&three).len(), 3);
        let four: Vec<_> = ["a", "b", "c", "d"].iter().map(|id| mail(id)).collect();
        assert_eq!(
            plan(&four),
            vec![DesktopNotification {
                title: "EmailOps".into(),
                body: "4 new messages in me@example.com".into(),
                thread: None,
            }]
        );
    }

    #[test]
    fn the_count_is_of_notifiable_messages_only() {
        let mut batch: Vec<_> = ["a", "b", "c", "d"].iter().map(|id| mail(id)).collect();
        batch[3].is_read = true;
        assert_eq!(plan(&batch).len(), 3, "3 notifiable → individual, not a summary");
    }

    #[test]
    fn a_summary_is_rate_limited_per_account() {
        let five: Vec<_> = ["a", "b", "c", "d", "e"].iter().map(|id| mail(id)).collect();
        let at = |last: i64| AccountNotifyState {
            last_summary_at: Some(last),
            ..state()
        };
        let p = NotifyPrefs::default();
        assert!(plan_new_mail_notifications(&five, &p, &at(NOW - 30)).is_empty());
        assert!(plan_new_mail_notifications(&five, &p, &at(NOW - SUMMARY_COOLDOWN_SECS + 1)).is_empty());
        assert_eq!(
            plan_new_mail_notifications(&five, &p, &at(NOW - SUMMARY_COOLDOWN_SECS)).len(),
            1
        );
        // Individual notifications are not throttled by a recent summary.
        assert_eq!(plan_new_mail_notifications(&five[..2], &p, &at(NOW - 5)).len(), 2);
    }

    #[test]
    fn hidden_mode_reveals_only_the_account() {
        let hidden = NotifyPrefs {
            content: ContentMode::Hidden,
            ..NotifyPrefs::default()
        };
        let shown = plan_new_mail_notifications(&[mail("a")], &hidden, &state());
        assert_eq!(shown[0].title, "EmailOps");
        assert_eq!(shown[0].body, "New message in me@example.com");
    }

    #[test]
    fn a_locked_app_hides_content_whatever_the_setting() {
        let locked = AccountNotifyState {
            app_locked: true,
            ..state()
        };
        let shown = plan_new_mail_notifications(&[mail("a")], &NotifyPrefs::default(), &locked);
        assert_eq!(shown[0].title, "EmailOps");
        assert_eq!(shown[0].body, "New message in me@example.com");
        assert!(!shown[0].body.contains("Quarterly"));
    }

    #[test]
    fn texts_follow_the_ui_language() {
        let es = AccountNotifyState {
            language: Language::Es,
            ..state()
        };
        let mut c = mail("a");
        c.subject = "  ".into();
        let shown = plan_new_mail_notifications(&[c], &NotifyPrefs::default(), &es);
        assert_eq!(shown[0].body, "(sin asunto)");
        let four: Vec<_> = ["a", "b", "c", "d"].iter().map(|id| mail(id)).collect();
        let shown = plan_new_mail_notifications(&four, &NotifyPrefs::default(), &es);
        assert_eq!(shown[0].body, "4 mensajes nuevos en me@example.com");
    }

    #[test]
    fn titles_fall_back_to_the_address_and_subjects_fit_one_line() {
        let mut c = mail("a");
        c.sender = " ".into();
        c.subject = format!("Line one\r\n  line two {}", "x".repeat(200));
        let shown = plan(&[c]);
        assert_eq!(shown[0].title, "ana@example.com");
        assert!(shown[0].body.starts_with("Line one line two x"));
        assert_eq!(shown[0].body.chars().count(), MAX_SUBJECT_CHARS);
        assert!(shown[0].body.ends_with('…'));
    }

    fn snooze_return(account: &str, thread: &str) -> SnoozeReturn {
        SnoozeReturn {
            thread: ThreadRef {
                account_id: account.into(),
                thread_id: thread.into(),
            },
            subject: "Contract draft".into(),
            account_label: format!("{account}@example.com"),
        }
    }

    fn snooze_state() -> SnoozeNotifyState {
        SnoozeNotifyState {
            app_focused: false,
            app_locked: false,
            language: Language::En,
        }
    }

    #[test]
    fn snoozed_conversations_coming_back_notify_behind_the_same_switches() {
        let all_on = |_: &str| NotifyPrefs::default();
        let shown = plan_snooze_return_notifications(&[snooze_return("acc-1", "t1")], &all_on, &snooze_state());
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Snoozed conversation is back");
        assert_eq!(shown[0].body, "Contract draft");

        let acc1_off = |id: &str| NotifyPrefs {
            account_enabled: id != "acc-1",
            ..NotifyPrefs::default()
        };
        let shown = plan_snooze_return_notifications(
            &[snooze_return("acc-1", "t1"), snooze_return("acc-2", "t2")],
            &acc1_off,
            &snooze_state(),
        );
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].thread.as_ref().unwrap().account_id, "acc-2");

        let focused = SnoozeNotifyState {
            app_focused: true,
            ..snooze_state()
        };
        assert!(plan_snooze_return_notifications(&[snooze_return("acc-1", "t1")], &all_on, &focused).is_empty());

        let locked = SnoozeNotifyState {
            app_locked: true,
            ..snooze_state()
        };
        let shown = plan_snooze_return_notifications(&[snooze_return("acc-1", "t1")], &all_on, &locked);
        assert_eq!(shown[0].body, "acc-1@example.com");

        let many: Vec<_> = (0..4).map(|i| snooze_return("acc-1", &format!("t{i}"))).collect();
        let shown = plan_snooze_return_notifications(&many, &all_on, &snooze_state());
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].body, "4 snoozed conversations are back");
    }

    // ── Executor ────────────────────────────────────────────────────────────

    fn account() -> Account {
        Account {
            id: "acc-1".into(),
            provider: "gmail".into(),
            email: "acc-1".into(),
            name: "Test".into(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    fn stored(id: &str) -> Email {
        Email {
            id: id.into(),
            account_id: "acc-1".into(),
            thread_id: format!("t-{id}"),
            message_id: None,
            references: None,
            subject: "Quarterly plans".into(),
            sender: "Ana Ruiz".into(),
            sender_email: "ana@example.com".into(),
            recipients: vec![],
            cc: vec![],
            body: String::new(),
            snippet: String::new(),
            timestamp: NOW - 60,
            is_read: false,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    #[test]
    fn candidates_from_stored_rows_know_who_sent_them() {
        let c = MailCandidate::from_email(&stored("a"), "me@example.com", true);
        assert!(!c.from_self);
        assert!(c.from_backfill);
        let c = MailCandidate::from_email(&stored("a"), "ANA@example.com", false);
        assert!(c.from_self, "the account's own address, any case");
        let mut sent = stored("a");
        sent.is_sent = true;
        assert!(MailCandidate::from_email(&sent, "me@example.com", false).from_self);
    }

    fn test_db() -> Database {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db
    }

    #[test]
    fn the_executor_shows_the_plan_and_reads_junk_after_the_hooks() {
        let db = test_db();
        db.insert_emails_batch(&[stored("a"), stored("b")]).unwrap();
        // A junk override (what blocking a sender writes) silences that one.
        db.set_junk_override("b", "acc-1", Some("junk"), NOW).unwrap();
        let notifier = VecNotifier::new();
        notify_new_mail(
            &db,
            &notifier,
            &account(),
            Some(WATERMARK),
            vec![mail("a"), mail("b")],
            NOW,
        );
        let shown = notifier.shown();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].thread.as_ref().unwrap().thread_id, "t-a");
    }

    #[test]
    fn the_executor_respects_stored_prefs_and_focus() {
        let db = test_db();
        let notifier = VecNotifier::new();
        db.set_preference(&format!("{PREF_ACCOUNT_PREFIX}acc-1"), "false")
            .unwrap();
        notify_new_mail(&db, &notifier, &account(), Some(WATERMARK), vec![mail("a")], NOW);
        assert!(notifier.shown().is_empty());

        db.set_preference(&format!("{PREF_ACCOUNT_PREFIX}acc-1"), "true")
            .unwrap();
        notifier.set_focused(true);
        notify_new_mail(&db, &notifier, &account(), Some(WATERMARK), vec![mail("a")], NOW);
        assert!(notifier.shown().is_empty(), "focused app, default only-when-unfocused");

        db.set_preference(PREF_ONLY_UNFOCUSED, "false").unwrap();
        db.set_preference(PREF_CONTENT, "hidden").unwrap();
        notify_new_mail(&db, &notifier, &account(), Some(WATERMARK), vec![mail("a")], NOW);
        assert_eq!(notifier.shown()[0].body, "New message in acc-1");
    }

    #[test]
    fn the_snooze_executor_names_the_conversation_by_its_latest_subject() {
        let db = test_db();
        let mut later = stored("b");
        later.thread_id = "t-a".into();
        later.timestamp = NOW;
        later.subject = "Re: Quarterly plans".into();
        db.insert_emails_batch(&[stored("a"), later]).unwrap();
        let notifier = VecNotifier::new();
        let woken = [ThreadRef {
            account_id: "acc-1".into(),
            thread_id: "t-a".into(),
        }];
        notify_snooze_returns(&db, &notifier, &woken);
        assert_eq!(notifier.shown()[0].body, "Re: Quarterly plans");

        db.set_preference(PREF_ENABLED, "false").unwrap();
        notify_snooze_returns(&db, &notifier, &woken);
        assert_eq!(notifier.shown().len(), 1, "master switch off");
    }

    #[test]
    fn the_executor_persists_the_summary_time_for_the_rate_limit() {
        let db = test_db();
        let notifier = VecNotifier::new();
        let five = || ["a", "b", "c", "d", "e"].iter().map(|id| mail(id)).collect::<Vec<_>>();
        notify_new_mail(&db, &notifier, &account(), Some(WATERMARK), five(), NOW);
        notify_new_mail(&db, &notifier, &account(), Some(WATERMARK), five(), NOW + 30);
        assert_eq!(notifier.shown().len(), 1, "second summary within a minute is dropped");
        notify_new_mail(&db, &notifier, &account(), Some(WATERMARK), five(), NOW + 61);
        assert_eq!(notifier.shown().len(), 2);
    }
}
