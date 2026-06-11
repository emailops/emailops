-- The exact user-message text as it was sent to the model (memory header +
-- sources block + question). Replaying history with these bytes keeps each
-- turn's prompt a pure extension of the previous one, which is what lets the
-- llama.cpp KV prefix cache survive across turns. NULL for assistant/system
-- rows and for user rows written before this migration (replay falls back to
-- `content`).
ALTER TABLE chat_messages ADD COLUMN prompt_content TEXT;
