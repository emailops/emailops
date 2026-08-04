-- V022: multi-calendar support.
--
-- Until now sync hard-coded the account's primary calendar (Google
-- `calendars/primary`, Graph `/me/calendarView`). Accounts can own several
-- calendars and subscribe to calendars shared with them, each carrying its own
-- colour in the provider's UI. This migration adds the calendar registry and
-- re-keys events so the same event id can live in more than one calendar.

-- One row per calendar the account can see (owned, shared-with-me, subscribed).
-- Refreshed on every sync from Google `calendarList.list` / Graph
-- `/me/calendars`; `is_visible` is the user's local show/hide toggle and is
-- deliberately NOT overwritten by sync.
CREATE TABLE IF NOT EXISTS calendars (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    -- Provider calendar id: an email-like address on Google
    -- ("…@group.calendar.google.com", or the literal "primary" alias) and an
    -- opaque base64-ish id on Graph.
    provider_calendar_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    -- Provider colour as "#rrggbb"; empty when the provider reported none, in
    -- which case the UI falls back to a deterministic palette slot.
    color TEXT NOT NULL DEFAULT '',
    is_primary INTEGER NOT NULL DEFAULT 0,
    -- Google calendarList accessRole, with Graph mapped onto the same set
    -- (canEdit → writer, otherwise reader). Drives whether writes are offered.
    access_role TEXT NOT NULL DEFAULT 'reader'
        CHECK (access_role IN ('owner', 'writer', 'reader', 'freeBusyReader')),
    -- Local show/hide toggle (Settings → Calendar). Sync preserves it.
    is_visible INTEGER NOT NULL DEFAULT 1,
    -- Provider list order, so the UI lists calendars the way the provider does
    -- (primary first).
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
    UNIQUE (account_id, provider_calendar_id)
);

CREATE INDEX IF NOT EXISTS idx_calendars_account
    ON calendars(account_id, sort_order);

-- Re-key calendar_events from (account_id, provider_event_id) to
-- (account_id, calendar_id, provider_event_id).
--
-- Why the table rebuild: Google hands out the SAME event id for every copy of
-- a meeting you are an attendee on, so one event visible in both your primary
-- calendar and a calendar shared with you collides under the old unique key —
-- the second copy silently overwrote the first. SQLite cannot drop a UNIQUE
-- constraint in place, hence the copy/drop/rename.
--
-- Row ids gain the calendar segment for the same reason. Ids are opaque
-- (never parsed apart), so rewriting them is safe; existing rows are all
-- 'primary' because that is all the old sync ever wrote.
CREATE TABLE calendar_events_v022 (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    provider_event_id TEXT NOT NULL,
    calendar_id TEXT NOT NULL DEFAULT 'primary',
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    location TEXT NOT NULL DEFAULT '',
    start_time INTEGER NOT NULL,
    end_time INTEGER NOT NULL,
    is_all_day INTEGER NOT NULL DEFAULT 0,
    timezone TEXT NOT NULL DEFAULT '',
    organizer TEXT NOT NULL DEFAULT '',
    attendees_json TEXT NOT NULL DEFAULT '[]',
    meeting_link TEXT,
    meeting_platform TEXT,
    status TEXT NOT NULL DEFAULT 'confirmed'
        CHECK (status IN ('confirmed', 'tentative', 'cancelled')),
    html_link TEXT,
    notified_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    recurring_event_id TEXT,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
    UNIQUE (account_id, calendar_id, provider_event_id)
);

INSERT INTO calendar_events_v022 (
    id, account_id, provider_event_id, calendar_id, title, description, location,
    start_time, end_time, is_all_day, timezone, organizer, attendees_json,
    meeting_link, meeting_platform, status, html_link, notified_at,
    created_at, updated_at, recurring_event_id
)
SELECT
    account_id || ':' || calendar_id || ':' || provider_event_id,
    account_id, provider_event_id, calendar_id, title, description, location,
    start_time, end_time, is_all_day, timezone, organizer, attendees_json,
    meeting_link, meeting_platform, status, html_link, notified_at,
    created_at, updated_at, recurring_event_id
FROM calendar_events;

DROP TABLE calendar_events;

ALTER TABLE calendar_events_v022 RENAME TO calendar_events;

-- Recreate the indexes the rebuild dropped (V011 range index, V012 series index).
CREATE INDEX IF NOT EXISTS idx_calendar_events_account_start
    ON calendar_events(account_id, start_time);

CREATE INDEX IF NOT EXISTS idx_calendar_events_account_master
    ON calendar_events(account_id, recurring_event_id)
    WHERE recurring_event_id IS NOT NULL;
