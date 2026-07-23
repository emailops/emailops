-- V012: link expanded calendar-event instances to their recurring series
-- master (Google `recurringEventId` / Graph `seriesMasterId`). NULL for
-- non-recurring events. Enables scoped deletes: this instance / this and
-- following / the whole series.
ALTER TABLE calendar_events ADD COLUMN recurring_event_id TEXT;

CREATE INDEX IF NOT EXISTS idx_calendar_events_account_master
    ON calendar_events(account_id, recurring_event_id)
    WHERE recurring_event_id IS NOT NULL;
