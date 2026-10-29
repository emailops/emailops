//! Subscribing to ICS feeds: validate the URL, fetch it once to prove it is a
//! calendar, keep the URL in the secrets vault (a Proton share link embeds
//! the calendar's decryption key) and the non-secret row in `calendar_feeds`,
//! then mirror the feed into the calendar tables straight away.

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, CalendarFeed};
use crate::services::calendar::ics::parse_vcalendar;
use crate::services::logger;
use crate::sync::ics_feed_calendar::{feed_host, IcsFeedCalendarProvider, IcsFetcher, CALENDAR_FEED_KEYRING_SERVICE};

/// Longest feed URL accepted. Proton share links are ~200 chars; the cap only
/// stops a pasted blob from reaching the vault.
pub const MAX_FEED_URL_CHARS: usize = 2_048;

/// Pure: an acceptable feed URL — https, with a host, without embedded
/// credentials, within the length cap. The returned URL is normalised.
pub fn validate_feed_url(raw: &str) -> Result<reqwest::Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::InvalidInput("the calendar feed URL is empty".into()));
    }
    if raw.chars().count() > MAX_FEED_URL_CHARS {
        return Err(AppError::InvalidInput("the calendar feed URL is too long".into()));
    }
    let url =
        reqwest::Url::parse(raw).map_err(|_| AppError::InvalidInput("the calendar feed URL is not valid".into()))?;
    if url.scheme() != "https" {
        return Err(AppError::InvalidInput(
            "the calendar feed URL must start with https://".into(),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(AppError::InvalidInput("the calendar feed URL has no host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::InvalidInput(
            "the calendar feed URL must not embed a user name or password".into(),
        ));
    }
    Ok(url)
}

/// Pure: the calendar's display name — what the user typed, else the feed's
/// own `X-WR-CALNAME`, else its host.
pub fn display_label(user_label: &str, calname: Option<&str>, host: &str) -> String {
    let typed = user_label.trim();
    if !typed.is_empty() {
        return typed.to_string();
    }
    match calname.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => name.to_string(),
        None => host.to_string(),
    }
}

pub fn list_feeds(db: &Database, account_id: &str) -> Result<Vec<CalendarFeed>> {
    db.list_calendar_feeds(account_id)
}

/// Subscribe `account` (IMAP only — OAuth providers have a calendar API) to a
/// feed: validate, fetch it once to prove it is an iCalendar document, store
/// the URL in the vault and the row in SQLite, then mirror it into the
/// calendar tables so the user sees events immediately. A mirroring failure
/// is logged, not returned: the feed is subscribed and the scheduler retries.
pub async fn add_feed(
    db: &Database,
    account: &Account,
    label: &str,
    url: &str,
    fetcher: Arc<dyn IcsFetcher>,
    now: i64,
) -> Result<CalendarFeed> {
    if account.provider != "imap" {
        return Err(AppError::InvalidInput(format!(
            "{} accounts sync their own calendar; feeds are for IMAP accounts",
            account.provider
        )));
    }
    let url = validate_feed_url(url)?;
    let url = url.to_string();
    let host = feed_host(&url);

    let body = fetcher.fetch(&url).await?;
    let parsed = parse_vcalendar(&body);
    if !body.to_ascii_uppercase().contains("BEGIN:VCALENDAR") {
        return Err(AppError::InvalidInput(format!(
            "{host} did not return a calendar (no VCALENDAR in the response)"
        )));
    }

    let feed = CalendarFeed {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account.id.clone(),
        label: display_label(label, parsed.name.as_deref(), &host),
        host,
        created_at: now,
    };
    // Secret first: a row without its URL would be an unusable feed, while a
    // secret without its row is cleaned up right below.
    crate::services::secrets_vault::set(CALENDAR_FEED_KEYRING_SERVICE, &feed.id, &url)?;
    if let Err(e) = db.insert_calendar_feed(&feed) {
        if let Err(vault_err) = crate::services::secrets_vault::delete(CALENDAR_FEED_KEYRING_SERVICE, &feed.id) {
            logger::log(
                "error",
                "account",
                format!("calendar feed: could not remove the orphaned URL from the vault: {vault_err}"),
            );
        }
        return Err(e);
    }
    logger::log(
        "success",
        "sync",
        format!(
            "Calendar feed '{}' ({}) added to {}",
            feed.label, feed.host, account.email
        ),
    );

    match IcsFeedCalendarProvider::from_db(db, &account.id, fetcher) {
        Ok(provider) => {
            if let Err(e) =
                crate::services::calendar::sync::sync_account_calendar(db, &account.id, &provider, now).await
            {
                logger::log(
                    "error",
                    "sync",
                    format!("calendar feed '{}': first sync failed: {e}", feed.label),
                );
            }
        }
        Err(e) => logger::log(
            "error",
            "sync",
            format!("calendar feed '{}': first sync failed: {e}", feed.label),
        ),
    }
    Ok(feed)
}

/// Unsubscribe: drop the feed row with its mirrored calendar and events, then
/// its URL from the vault. A vault failure is logged — the row is gone and an
/// `Err` would report a removal that succeeded as failed.
pub fn remove_feed(db: &Database, account_id: &str, feed_id: &str) -> Result<()> {
    db.delete_calendar_feed(account_id, feed_id)?;
    if let Err(e) = crate::services::secrets_vault::delete(CALENDAR_FEED_KEYRING_SERVICE, feed_id) {
        logger::log(
            "error",
            "account",
            format!("Calendar feed removed, but clearing its URL from the vault failed: {e}"),
        );
    }
    logger::log("info", "sync", format!("Calendar feed {feed_id} removed"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::ics_feed_calendar::{FakeIcsFetcher, CALENDAR_FEED_KEYRING_SERVICE};
    use std::sync::MutexGuard;

    const FEED_URL: &str = "https://calendar.proton.me/api/calendar/v1/url/abc/calendar.ics?CacheKey=k&PassphraseKey=p";
    const FEED_BODY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Team\\, shared\r\nBEGIN:VEVENT\r\nUID:e1@x\r\nDTSTART:20260729T070000Z\r\nDTEND:20260729T080000Z\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn account(id: &str, provider: &str) -> Account {
        Account {
            id: id.to_string(),
            provider: provider.to_string(),
            email: format!("{id}@example.com"),
            name: id.to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
            color: crate::models::AccountColor::Blue,
        }
    }

    /// Vault cache + keychain are process globals: lock the seam, reset, and
    /// install an in-memory keychain. Async work runs on a current-thread
    /// runtime under the guard (never held across an `.await`).
    fn vault_setup() -> MutexGuard<'static, ()> {
        let guard = crate::services::events::seam_test_lock();
        crate::services::secrets_vault::reset_for_testing();
        let _ = crate::services::keychain::install_for_testing();
        guard
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(f)
    }

    fn imap_db(id: &str) -> Database {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider(id, "imap");
        db
    }

    // ── pure helpers ─────────────────────────────────────────────────────

    #[test]
    fn validate_feed_url_accepts_https_and_rejects_the_rest() {
        assert!(validate_feed_url(FEED_URL).is_ok());
        assert!(
            validate_feed_url("  https://cal.example.com/x.ics  ").is_ok(),
            "whitespace is trimmed"
        );
        for bad in [
            "http://cal.example.com/x.ics",
            "webcal://cal.example.com/x.ics",
            "https://user:pw@cal.example.com/x.ics",
            "https://",
            "not a url",
            "",
        ] {
            assert!(
                matches!(validate_feed_url(bad), Err(AppError::InvalidInput(_))),
                "{bad}"
            );
        }
        let long = format!("https://cal.example.com/{}", "a".repeat(MAX_FEED_URL_CHARS));
        assert!(matches!(validate_feed_url(&long), Err(AppError::InvalidInput(_))));
    }

    #[test]
    fn display_label_prefers_the_user_label_then_calname_then_host() {
        assert_eq!(display_label("  Work ", Some("Team"), "cal.example.com"), "Work");
        assert_eq!(
            display_label("", Some("Team, shared"), "cal.example.com"),
            "Team, shared"
        );
        assert_eq!(display_label("   ", None, "cal.example.com"), "cal.example.com");
    }

    // ── add_feed ─────────────────────────────────────────────────────────

    #[test]
    fn add_feed_rejects_non_imap_accounts_and_bad_urls_without_fetching() {
        let _g = vault_setup();
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account("g");
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body(FEED_URL, FEED_BODY));

        let err = block_on(add_feed(
            &db,
            &account("g", "gmail"),
            "Work",
            FEED_URL,
            fetcher.clone(),
            1_000,
        ))
        .expect_err("gmail");
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        let db = imap_db("i");
        let err = block_on(add_feed(
            &db,
            &account("i", "imap"),
            "Work",
            "http://cal.example.com/x.ics",
            fetcher.clone(),
            1_000,
        ))
        .expect_err("http");
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        assert_eq!(fetcher.fetch_count(), 0);
        assert_eq!(db.count_calendar_feeds("i").expect("count"), 0);
    }

    #[test]
    fn add_feed_stores_the_url_in_the_vault_not_in_sqlite_and_mirrors_the_calendar() {
        let _g = vault_setup();
        let db = imap_db("i");
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body(FEED_URL, FEED_BODY));

        // 20/07/2026: the fixture event (29/07/2026) sits inside the sync window.
        let now = 1_784_505_600;
        let feed = block_on(add_feed(&db, &account("i", "imap"), "", FEED_URL, fetcher, now)).expect("added");

        assert_eq!(feed.account_id, "i");
        assert_eq!(feed.label, "Team, shared", "blank label falls back to X-WR-CALNAME");
        assert_eq!(feed.host, "calendar.proton.me");
        assert_eq!(feed.created_at, now);
        assert_eq!(db.list_calendar_feeds("i").expect("list"), vec![feed.clone()]);
        assert_eq!(
            crate::services::secrets_vault::get(CALENDAR_FEED_KEYRING_SERVICE, &feed.id)
                .expect("vault")
                .as_deref(),
            Some(FEED_URL)
        );
        // No column of the feeds row carries the URL (or its key material).
        let dumped: String = db
            .reader()
            .query_row(
                "SELECT id || account_id || label || host FROM calendar_feeds WHERE id = ?1",
                [&feed.id],
                |r| r.get(0),
            )
            .expect("row");
        assert!(
            !dumped.contains("PassphraseKey") && !dumped.contains("https"),
            "{dumped}"
        );
        // Mirrored straight away: one reader calendar and its event.
        let calendars = db.list_calendars("i").expect("calendars");
        assert_eq!(calendars.len(), 1);
        assert_eq!(
            (
                calendars[0].provider_calendar_id.as_str(),
                calendars[0].access_role.as_str()
            ),
            (feed.id.as_str(), "reader")
        );
        assert_eq!(calendars[0].name, "Team, shared");
        let events = db.list_calendar_events("i", 0, i64::MAX).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].calendar_id, feed.id);
    }

    #[test]
    fn add_feed_fetch_failure_leaves_no_row_and_no_secret() {
        let _g = vault_setup();
        let db = imap_db("i");
        let fetcher = Arc::new(FakeIcsFetcher::default().failing(FEED_URL));

        let err =
            block_on(add_feed(&db, &account("i", "imap"), "Work", FEED_URL, fetcher, 1_000)).expect_err("fetch failed");
        assert!(matches!(err, AppError::SyncError(_)), "{err:?}");
        assert!(!err.to_string().contains("PassphraseKey"), "URL must not leak: {err}");
        assert_eq!(db.count_calendar_feeds("i").expect("count"), 0);
    }

    #[test]
    fn add_feed_rejects_a_body_that_is_not_a_calendar() {
        let _g = vault_setup();
        let db = imap_db("i");
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body(FEED_URL, "<html>login</html>"));

        let err =
            block_on(add_feed(&db, &account("i", "imap"), "Work", FEED_URL, fetcher, 1_000)).expect_err("not ics");
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        assert_eq!(db.count_calendar_feeds("i").expect("count"), 0);
    }

    // ── remove_feed ──────────────────────────────────────────────────────

    #[test]
    fn remove_feed_deletes_the_rows_and_the_secret() {
        let _g = vault_setup();
        let db = imap_db("i");
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body(FEED_URL, FEED_BODY));
        let feed = block_on(add_feed(&db, &account("i", "imap"), "Work", FEED_URL, fetcher, 1_000)).expect("added");

        remove_feed(&db, "i", &feed.id).expect("removed");

        assert_eq!(db.count_calendar_feeds("i").expect("count"), 0);
        assert!(db.list_calendars("i").expect("calendars").is_empty());
        assert_eq!(
            crate::services::secrets_vault::get(CALENDAR_FEED_KEYRING_SERVICE, &feed.id).expect("vault"),
            None
        );
    }

    #[test]
    fn remove_feed_of_another_account_is_not_found_and_keeps_the_secret() {
        let _g = vault_setup();
        let db = imap_db("i");
        db.seed_test_account_with_provider("other", "imap");
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body(FEED_URL, FEED_BODY));
        let feed = block_on(add_feed(&db, &account("i", "imap"), "Work", FEED_URL, fetcher, 1_000)).expect("added");

        let err = remove_feed(&db, "other", &feed.id).expect_err("wrong account");
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
        assert!(
            crate::services::secrets_vault::get(CALENDAR_FEED_KEYRING_SERVICE, &feed.id)
                .expect("vault")
                .is_some()
        );
    }
}
