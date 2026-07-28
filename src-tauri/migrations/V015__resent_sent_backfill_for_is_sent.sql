-- Re-open the Sent backfill so existing databases learn which stored messages
-- the provider filed under Sent.
--
-- V014 could only backfill `is_sent` from the local `mailbox` column, which
-- misses the case it was added for: Gmail labels mail you send to yourself
-- INBOX *and* SENT, so the inbox pass stored it as 'inbox'. The Sent pass now
-- repairs such rows when it lists them (see `ingest_mailbox_refs`), but on an
-- existing account that pass has already marked its history walk complete and
-- only looks forward from its watermark.
--
-- Clearing the completion flag and cursor makes the next syncs walk Sent
-- history once more. This is cheap: every message is already stored, so the
-- pass issues id-listing calls and in-place flag updates without re-downloading
-- any bodies. The forward watermark is deliberately left alone so incremental
-- sync does not re-fetch recent mail.
DELETE FROM user_preferences WHERE key LIKE 'extra_mailbox_backfill:%:sent';
DELETE FROM user_preferences WHERE key LIKE 'extra_mailbox_backfill_cursor:%:sent';
