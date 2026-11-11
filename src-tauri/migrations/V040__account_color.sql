-- V040: a colour per account.
--
-- One colour identifies an account everywhere the app mixes accounts: the
-- sidebar, the unified inbox rows and the "All accounts" calendar. It is a
-- palette key (the frontend maps it to its own classes / hex), assigned when
-- the account is added and editable in the account's settings.
--
-- Existing accounts are backfilled in sidebar order, one palette slot each,
-- so the first eight accounts never share a colour.
ALTER TABLE accounts ADD COLUMN color TEXT NOT NULL DEFAULT 'blue'
    CHECK (color IN ('blue', 'emerald', 'amber', 'purple', 'rose', 'cyan', 'indigo', 'orange'));

UPDATE accounts
SET color = (
    SELECT CASE (ranked.position - 1) % 8
        WHEN 0 THEN 'blue'
        WHEN 1 THEN 'emerald'
        WHEN 2 THEN 'amber'
        WHEN 3 THEN 'purple'
        WHEN 4 THEN 'rose'
        WHEN 5 THEN 'cyan'
        WHEN 6 THEN 'indigo'
        ELSE 'orange'
    END
    FROM (
        SELECT id, ROW_NUMBER() OVER (ORDER BY sort_order ASC, created_at DESC) AS position
        FROM accounts
    ) AS ranked
    WHERE ranked.id = accounts.id
);
