-- Blind copies of a draft, saved and restored with it (#270). JSON array of
-- addresses, like to_addresses_json and cc_addresses_json.
ALTER TABLE drafts ADD COLUMN bcc_addresses_json TEXT NOT NULL DEFAULT '[]';
