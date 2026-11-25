-- An attachment rule can keep only the newest copy of each file name in a
-- thread: a document sent back and forth in a conversation is held once, at
-- its latest version. On by default, for the rules that already exist too.
ALTER TABLE attachment_rules ADD COLUMN keep_latest_per_thread INTEGER NOT NULL DEFAULT 1;
