-- V036: what the user did with a reply draft the email agent wrote — sent it
-- or discarded it from the Agent view. NULL while it waits for review.
ALTER TABLE agent_actions ADD COLUMN review_outcome TEXT CHECK (review_outcome IN ('sent', 'discarded'));
ALTER TABLE agent_actions ADD COLUMN reviewed_at INTEGER;
