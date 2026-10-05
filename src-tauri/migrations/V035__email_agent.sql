-- V035: the email agent.
--
-- `agent_rules`: the user's rules. When `match_prompt` describes a new email
-- (trigger 'email') or an upcoming calendar event (trigger 'event'), the
-- agent follows `action_prompt`. `account_id` NULL = every account.
CREATE TABLE IF NOT EXISTS agent_rules (
    id              TEXT    PRIMARY KEY,
    name            TEXT    NOT NULL CHECK (trim(name) <> ''),
    trigger_kind    TEXT    NOT NULL CHECK (trigger_kind IN ('email', 'event')),
    account_id      TEXT    REFERENCES accounts(id) ON DELETE CASCADE,
    match_prompt    TEXT    NOT NULL,
    action_prompt   TEXT    NOT NULL,
    always_approve  INTEGER NOT NULL DEFAULT 0 CHECK (always_approve IN (0, 1)),
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- `agent_runs`: the agent's work on one email or event. One row per trigger,
-- whatever the outcome, so nothing is ever evaluated twice. Rows that matched
-- no rule are kept for that but never shown.
CREATE TABLE IF NOT EXISTS agent_runs (
    id            TEXT    PRIMARY KEY,
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    trigger_kind  TEXT    NOT NULL CHECK (trigger_kind IN ('email', 'event')),
    trigger_ref   TEXT    NOT NULL,
    title         TEXT    NOT NULL DEFAULT '',
    sender        TEXT    NOT NULL DEFAULT '',
    status        TEXT    NOT NULL CHECK (status IN ('matched', 'no_match', 'failed')),
    summary       TEXT    NOT NULL DEFAULT '',
    error         TEXT,
    created_at    INTEGER NOT NULL,
    UNIQUE (trigger_kind, trigger_ref)
);
CREATE INDEX IF NOT EXISTS idx_agent_runs_feed ON agent_runs(status, created_at DESC);

-- `agent_actions`: what a run did ('done' / 'failed') or proposes ('pending'
-- until the user approves or rejects it).
CREATE TABLE IF NOT EXISTS agent_actions (
    id                 TEXT    PRIMARY KEY,
    run_id             TEXT    NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    rule_id            TEXT    REFERENCES agent_rules(id) ON DELETE SET NULL,
    rule_name          TEXT    NOT NULL,
    kind               TEXT    NOT NULL
        CHECK (kind IN ('draft_reply', 'create_task', 'run_skill', 'mark_read', 'archive', 'star')),
    detail             TEXT    NOT NULL DEFAULT '',
    status             TEXT    NOT NULL CHECK (status IN ('pending', 'done', 'failed', 'rejected')),
    requires_approval  INTEGER NOT NULL CHECK (requires_approval IN (0, 1)),
    result             TEXT,
    error              TEXT,
    created_at         INTEGER NOT NULL,
    decided_at         INTEGER
);
CREATE INDEX IF NOT EXISTS idx_agent_actions_run ON agent_actions(run_id);
CREATE INDEX IF NOT EXISTS idx_agent_actions_pending ON agent_actions(status) WHERE status = 'pending';

-- `agent_panels`: counters the user defined with a prompt; `agent_panel_hits`
-- holds the emails each one matched, counted over the panel's window.
CREATE TABLE IF NOT EXISTS agent_panels (
    id          TEXT    PRIMARY KEY,
    title       TEXT    NOT NULL CHECK (trim(title) <> ''),
    prompt      TEXT    NOT NULL CHECK (trim(prompt) <> ''),
    time_window TEXT    NOT NULL CHECK (time_window IN ('today', 'last7_days', 'last30_days')),
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_panel_hits (
    panel_id         TEXT    NOT NULL REFERENCES agent_panels(id) ON DELETE CASCADE,
    email_id         TEXT    NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    email_timestamp  INTEGER NOT NULL,
    PRIMARY KEY (panel_id, email_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_agent_panel_hits_window ON agent_panel_hits(panel_id, email_timestamp);
CREATE INDEX IF NOT EXISTS idx_agent_panel_hits_email ON agent_panel_hits(email_id);
