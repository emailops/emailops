-- V039: ICS-feed calendars for IMAP accounts.
--
-- Providers without a calendar API (Proton Calendar behind Bridge, any
-- CalDAV-less host) can still publish a read-only iCalendar feed ("share via
-- link"). One row per subscribed feed. The feed URL is NOT stored here: a
-- Proton share link embeds the calendar's decryption key, so the URL lives in
-- the secrets vault keyed by the feed id. `host` is kept for display and for
-- provider-specific affordances (e.g. a Proton feed offers "create in Proton
-- Calendar"); it is not sensitive.
--
-- Each feed is mirrored into `calendars` with `provider_calendar_id = id` and
-- `access_role = 'reader'`, and its expanded instances into `calendar_events`
-- with `calendar_id = id`, so the rest of the calendar stack needs no feed
-- special-casing.
CREATE TABLE IF NOT EXISTS calendar_feeds (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_calendar_feeds_account
    ON calendar_feeds(account_id, created_at);
