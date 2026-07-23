-- V011: calendar events synced from Google Calendar / Microsoft Graph.
-- Per-account only (no cross-account calendar surface — see docs/DECISIONS.md).
-- Rows are expanded recurrence *instances* fetched over a rolling window
-- (Google events.list singleEvents=true, Graph calendarView), so no RRULE
-- expansion happens locally.
CREATE TABLE IF NOT EXISTS calendar_events (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    provider_event_id TEXT NOT NULL,
    calendar_id TEXT NOT NULL DEFAULT 'primary',
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    location TEXT NOT NULL DEFAULT '',
    -- UTC epoch seconds. All-day events carry the day's midnight bounds and
    -- is_all_day = 1; `timezone` preserves the provider's original IANA zone.
    start_time INTEGER NOT NULL,
    end_time INTEGER NOT NULL,
    is_all_day INTEGER NOT NULL DEFAULT 0,
    timezone TEXT NOT NULL DEFAULT '',
    organizer TEXT NOT NULL DEFAULT '',
    attendees_json TEXT NOT NULL DEFAULT '[]',
    -- Join URL extracted at sync time (structured conference data first,
    -- regex over location/description as fallback). https-only by contract.
    meeting_link TEXT,
    meeting_platform TEXT,
    status TEXT NOT NULL DEFAULT 'confirmed'
        CHECK (status IN ('confirmed', 'tentative', 'cancelled')),
    html_link TEXT,
    -- Set when the upcoming-meeting notification for this event has fired,
    -- so restarts / rescans never notify twice.
    notified_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
    UNIQUE (account_id, provider_event_id)
);

-- Range queries ("events overlapping [start, end)") are the only read pattern.
CREATE INDEX IF NOT EXISTS idx_calendar_events_account_start
    ON calendar_events(account_id, start_time);

-- Incremental-sync cursor per account (Google syncToken / Graph deltaLink).
CREATE TABLE IF NOT EXISTS calendar_sync_state (
    account_id TEXT PRIMARY KEY,
    sync_token TEXT,
    last_sync_at INTEGER,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
