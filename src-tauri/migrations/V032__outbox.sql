-- V032: the local outbox behind undo send and scheduled send.
--
-- A message the user sent with an undo window, or scheduled for later, waits
-- here until `send_at`; the dispatcher in `services::outbox` then sends it
-- through the same send path as an immediate send. Nothing here reaches a
-- provider before `send_at`, so undo and cancel need no provider support, and
-- the app must be running for a message to go out.
--
-- Lifecycle (`status`):
--   scheduled → sending → sent
--                       ↘ failed     (provider refused it; or the app stopped
--                                     mid-send: `failure_kind = 'interrupted'`)
--   scheduled | failed  → cancelled  (undo, edit, delete)
--   failed              → scheduled  (retry / send now)
-- The dispatcher flips `scheduled → sending` in one guarded UPDATE before it
-- calls the provider, so two ticks cannot both send a row, and a row found in
-- `sending` at start-up is never resent automatically.
--
-- `payload` is the composed message as JSON (recipients, subject, bodies,
-- inline images and attachment bytes as base64), enough to repeat the exact
-- send call or reopen the composer. It is emptied once the row is sent or
-- cancelled, so attachment bytes do not outlive the message. The summary
-- columns are what the Scheduled view lists without reading the payload.
CREATE TABLE IF NOT EXISTS outbox (
    id                TEXT    PRIMARY KEY,
    account_id        TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind              TEXT    NOT NULL CHECK (kind IN ('new', 'reply')),
    -- The message a reply answers. No foreign key: a parent deleted meanwhile
    -- must fail the send visibly, not turn the reply into a new thread.
    reply_to_email_id TEXT,
    origin            TEXT    NOT NULL CHECK (origin IN ('undo', 'scheduled')),
    to_addresses      TEXT    NOT NULL DEFAULT '[]',
    cc_addresses      TEXT    NOT NULL DEFAULT '[]',
    subject           TEXT    NOT NULL DEFAULT '',
    attachment_count  INTEGER NOT NULL DEFAULT 0 CHECK (attachment_count >= 0),
    payload           TEXT    NOT NULL,
    send_at           INTEGER NOT NULL CHECK (send_at > 0),
    status            TEXT    NOT NULL DEFAULT 'scheduled'
                      CHECK (status IN ('scheduled', 'sending', 'sent', 'failed', 'cancelled')),
    attempts          INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error        TEXT,
    failure_kind      TEXT    CHECK (failure_kind IS NULL OR failure_kind IN ('error', 'interrupted')),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    CHECK ((kind = 'reply') = (reply_to_email_id IS NOT NULL))
);

-- The dispatcher reads the scheduled rows on every tick.
CREATE INDEX IF NOT EXISTS idx_outbox_status_send_at ON outbox(status, send_at);
CREATE INDEX IF NOT EXISTS idx_outbox_account ON outbox(account_id);
