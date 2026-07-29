-- Junk verdicts: one row per scored email.
--
-- Local-flag-only by design. Nothing here is ever pushed to the provider — the
-- message stays exactly where it is on the server and this table only changes
-- how the local UI orders and badges it. See docs/DECISIONS.md.
--
-- Three independent axis scores rather than one number, because a newsletter
-- and a wire-fraud attempt are not the same failure and must not share a
-- threshold or a UI treatment.
--
-- Deliberately NOT stored in `email_tags`: that table's primary key is
-- (email_id, tag_type), so it holds exactly one value per type and could not
-- carry three numeric scores plus a reason list plus the user's override state.
-- A derived `tag_type='junk'` row IS written alongside this one so the existing
-- tag chips and smart-filter aggregation pick junk up for free.

CREATE TABLE IF NOT EXISTS email_junk (
    email_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,

    spam_score REAL NOT NULL,
    phish_score REAL NOT NULL,
    gray_score REAL NOT NULL,

    -- Worst band across the three axes, for cheap list-view filtering.
    band TEXT NOT NULL
        CHECK (band IN ('clean', 'unknown', 'uncertain', 'junk')),
    primary_kind TEXT NOT NULL
        CHECK (primary_kind IN ('legit', 'spam', 'phishing', 'graymail')),

    -- Serialized Vec<Reason>. Reason codes are a closed enum rendered through
    -- i18n; details never contain a subject line or an address.
    reasons_json TEXT NOT NULL,

    method TEXT NOT NULL
        CHECK (method IN ('deterministic', 'statistical', 'llm')),
    -- Which trained model produced this, so a retrain can re-score selectively.
    model_version INTEGER NOT NULL DEFAULT 0,
    scored_at INTEGER NOT NULL,

    -- The user's explicit correction. 'not_junk' is PERMANENT: it survives
    -- retrains and model-version bumps and excludes the message from re-scoring
    -- forever. One re-flagged legitimate email destroys trust in the feature.
    user_override TEXT
        CHECK (user_override IS NULL OR user_override IN ('not_junk', 'junk')),
    overridden_at INTEGER,

    FOREIGN KEY (email_id) REFERENCES emails(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_email_junk_account_band
    ON email_junk(account_id, band, scored_at DESC);

-- Feedback lookup for the statistical layer's training pass.
CREATE INDEX IF NOT EXISTS idx_email_junk_override
    ON email_junk(account_id, user_override)
    WHERE user_override IS NOT NULL;
