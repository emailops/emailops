-- Companion to V002: the chat layer also wants to whitelist draft ids
-- the tools produced this turn, so the UI can render `draft://DRAFT_ID`
-- "Re-open draft" chips that survive validation. Same JSON-array TEXT
-- shape as `referenced_email_ids` — load-once-at-render-time access
-- pattern, no join table needed.
ALTER TABLE chat_messages ADD COLUMN referenced_draft_ids TEXT;
