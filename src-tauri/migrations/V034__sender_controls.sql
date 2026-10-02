-- V034: senders the user blocked, and lists the user unsubscribed from.
--
-- `blocked_senders`: mail from `address` (normalized: trimmed, lowercase) that
-- arrives in this account's inbox is marked junk and filed in the provider's
-- Spam/Junk folder on ingest. Per account, like Gmail's block list: the same
-- address may be wanted in one mailbox and not in another.
CREATE TABLE IF NOT EXISTS blocked_senders (
    account_id  TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    address     TEXT    NOT NULL CHECK (address = lower(trim(address)) AND address <> ''),
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, address)
) WITHOUT ROWID;

-- `sender_unsubscribes`: the user asked to leave the list behind `address`
-- (the sender of the message they unsubscribed from) through `method`. Kept
-- so the reading pane can say "You unsubscribed" afterwards. A later request
-- replaces the row.
CREATE TABLE IF NOT EXISTS sender_unsubscribes (
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    address       TEXT    NOT NULL CHECK (address = lower(trim(address)) AND address <> ''),
    method        TEXT    NOT NULL CHECK (method IN ('one_click', 'mailto', 'link')),
    requested_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, address)
) WITHOUT ROWID;
