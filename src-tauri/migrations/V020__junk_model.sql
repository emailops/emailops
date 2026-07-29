-- Per-account Naive Bayes counters for the junk detector's statistical layer.
--
-- One row per (account, axis). Only two axes are ever stored: `spam` and
-- `graymail`. Phishing is deliberately excluded — a mailbox yields a handful of
-- phishing examples at best, and a model trained on that emits noise rather
-- than signal, so that axis stays purely deterministic.
--
-- `counts_blob` holds two little-endian u32 arrays (positive counts, then
-- negative), one entry per hash bucket. The bucket count lives in
-- `services::junk::tokens::BUCKETS`; a change there makes existing blobs
-- meaningless rather than merely stale, which is why the loader rejects a blob
-- whose length no longer matches instead of reinterpreting it.
--
-- Note what is NOT stored here: the prior. The base rate is configuration, not
-- something learnt from these counts. The free training labels come from the
-- provider's spam folder, which is not a random sample of the inbox — deriving
-- the prior from `n_pos / n_neg` would inflate it by more than an order of
-- magnitude and make the classifier accuse on far weaker evidence than it
-- should. See `services::junk::model::score`.

CREATE TABLE IF NOT EXISTS junk_model (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    axis TEXT NOT NULL CHECK (axis IN ('spam', 'graymail')),

    -- Bumped on every retrain. `email_junk.model_version` records which model
    -- produced a verdict, so a re-score can be targeted rather than total.
    version INTEGER NOT NULL DEFAULT 1,

    n_pos INTEGER NOT NULL DEFAULT 0,
    n_neg INTEGER NOT NULL DEFAULT 0,
    counts_blob BLOB NOT NULL,
    trained_at INTEGER NOT NULL,

    PRIMARY KEY (account_id, axis)
);
