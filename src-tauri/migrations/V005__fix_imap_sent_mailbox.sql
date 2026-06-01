-- Repair IMAP messages mis-filed as 'inbox'.
--
-- IMAP message IDs embed a folder sub-prefix (SENT:: / SPAM:: / TRASH::) so the
-- lookup path knows which mailbox a UID belongs to. A prior bug left
-- ImapClient::get_message returning the default mailbox='inbox' for these
-- messages (it only flipped is_read), so a sent copy ingested by the primary
-- merged INBOX+Sent pass was stored as 'inbox' and surfaced in BOTH the Inbox
-- and Sent views. The forward fix derives the mailbox from the source folder;
-- this migration re-derives it for rows already persisted.
--
-- The dedup path skips re-downloading IDs that already exist, so a re-sync
-- alone would never correct these rows — they must be repaired in place.
--
-- Gmail / Outlook message IDs never contain these "FOLDER::" markers, so the
-- LIKE predicates only match IMAP rows.
UPDATE emails SET mailbox = 'sent'  WHERE mailbox <> 'sent'  AND id LIKE '%SENT::%';
UPDATE emails SET mailbox = 'spam'  WHERE mailbox <> 'spam'  AND id LIKE '%SPAM::%';
UPDATE emails SET mailbox = 'trash' WHERE mailbox <> 'trash' AND id LIKE '%TRASH::%';
