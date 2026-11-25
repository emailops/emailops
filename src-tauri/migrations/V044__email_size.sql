-- V044: each message's size, for the mail list's Size column and sort.
--
-- The whole message in bytes, attachments included, as the provider reports
-- it during sync: Gmail `sizeEstimate`, IMAP the length of the `BODY.PEEK[]`
-- download (its RFC822.SIZE). NULL means unknown: Microsoft Graph reports no
-- size, and mail stored before this column keeps NULL, because sync never
-- downloads a stored message again just to measure it.
ALTER TABLE emails ADD COLUMN size_bytes INTEGER;
