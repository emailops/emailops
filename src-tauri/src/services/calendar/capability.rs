//! Which calendar an account has, if any: a provider API (Gmail, Outlook)
//! or subscribed ICS feeds (IMAP with at least one feed). Replaces the old
//! provider-string check so every caller — sync scheduler, chat tools,
//! commands, CLI — answers the question the same way.

use crate::db::Database;
use crate::models::error::Result;
use crate::models::Account;
use crate::sync::calendar_provider::{calendar_kind, CalendarKind};

/// The calendar backend of `account`, or `None` when it has no calendar
/// (IMAP without feeds, unknown provider).
pub fn calendar_capability(db: &Database, account: &Account) -> Result<Option<CalendarKind>> {
    let feeds = if account.provider == "imap" {
        db.count_calendar_feeds(&account.id)?
    } else {
        0
    };
    Ok(calendar_kind(&account.provider, feeds))
}

/// Whether calendar surfaces (sync, reminders, chat tool) should run for the
/// account: it has a calendar AND the user has not switched the integration
/// off in Settings → Calendar. Read failures are logged and count as "no" —
/// never sync or notify on an account the user may have opted out of.
pub fn calendar_available(db: &Database, account: &Account) -> bool {
    let has_calendar = match calendar_capability(db, account) {
        Ok(kind) => kind.is_some(),
        Err(e) => {
            crate::services::logger::log(
                "error",
                "sync",
                format!("calendar: failed to read the calendar feeds of {}: {e}", account.id),
            );
            false
        }
    };
    has_calendar
        && match db.calendar_enabled(&account.id) {
            Ok(enabled) => enabled,
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "sync",
                    format!("calendar: failed to read calendar.enabled pref for {}: {e}", account.id),
                );
                false
            }
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::calendar::calendar_enabled_pref_key;
    use crate::models::CalendarFeed;
    use crate::sync::calendar_provider::CalendarKind;

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

    fn feed(id: &str, account_id: &str) -> CalendarFeed {
        CalendarFeed {
            id: id.to_string(),
            account_id: account_id.to_string(),
            label: "Feed".to_string(),
            host: "cal.example.com".to_string(),
            created_at: 1,
        }
    }

    #[test]
    fn oauth_providers_have_a_calendar_regardless_of_feeds() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account("g");
        assert_eq!(
            calendar_capability(&db, &account("g", "gmail")).expect("ok"),
            Some(CalendarKind::Google)
        );
        assert_eq!(
            calendar_capability(&db, &account("o", "outlook")).expect("ok"),
            Some(CalendarKind::Outlook)
        );
    }

    #[test]
    fn imap_account_gains_a_calendar_when_a_feed_exists() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("i", "imap");
        assert_eq!(calendar_capability(&db, &account("i", "imap")).expect("ok"), None);
        db.insert_calendar_feed(&feed("f1", "i")).expect("insert");
        assert_eq!(
            calendar_capability(&db, &account("i", "imap")).expect("ok"),
            Some(CalendarKind::IcsFeeds)
        );
    }

    #[test]
    fn calendar_available_requires_capability_and_the_opt_in_pref() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("i", "imap");
        assert!(
            !calendar_available(&db, &account("i", "imap")),
            "no feed → nothing to show"
        );
        db.insert_calendar_feed(&feed("f1", "i")).expect("insert");
        assert!(
            calendar_available(&db, &account("i", "imap")),
            "pref defaults to enabled"
        );
        db.set_preference(&calendar_enabled_pref_key("i"), "false")
            .expect("pref");
        assert!(
            !calendar_available(&db, &account("i", "imap")),
            "switched off in Settings"
        );
    }
}
