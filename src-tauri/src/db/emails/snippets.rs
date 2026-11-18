//! Repair of the snippets already stored.

use crate::db::Database;
use crate::models::error::Result;
use crate::util::html::decode_html_entities;
use rusqlite::params;

/// Set once the stored snippets are decoded. Decoding is not idempotent: the
/// first run turns `&amp;lt;` into the literal text `&lt;`, which a second
/// run would turn into `<`.
const SNIPPETS_DECODED_KEY: &str = "snippets_entities_decoded";

impl Database {
    /// Snippets synced before the sync decoded them keep the provider's HTML
    /// escaping (Gmail's `I&#39;m`, an HTML-only IMAP body's `&iacute;`), and
    /// the email list shows it. Decodes them, once per database, in one
    /// transaction with the marker. Returns how many snippets changed.
    pub fn decode_stored_snippets_once(&self) -> Result<usize> {
        if self.get_preference(SNIPPETS_DECODED_KEY)?.is_some() {
            return Ok(0);
        }
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        let rows: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id, snippet FROM emails WHERE snippet LIKE '%&%;%'")?;
            let collected = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            collected
        };
        let mut changed = 0usize;
        for (id, snippet) in rows {
            let decoded = decode_html_entities(&snippet);
            if decoded != snippet {
                tx.execute("UPDATE emails SET snippet = ?1 WHERE id = ?2", params![decoded, id])?;
                changed += 1;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?1, '1')",
            params![SNIPPETS_DECODED_KEY],
        )?;
        tx.commit()?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{email_fixture, insert_account};
    use crate::db::Database;

    fn db_with_snippet(snippet: &str) -> Database {
        let db = Database::new_for_testing().expect("db");
        insert_account(&db, "acct-1", "me@example.test");
        let mut email = email_fixture("e1", "acct-1", "<p>body</p>");
        email.snippet = snippet.to_string();
        db.insert_email(&email).expect("insert");
        db
    }

    fn snippet(db: &Database) -> String {
        db.get_email("e1").expect("read").expect("email").snippet
    }

    #[test]
    fn decodes_the_entities_of_stored_snippets() {
        let db = db_with_snippet("I&#39;m &quot;in&quot;");

        assert_eq!(db.decode_stored_snippets_once().expect("decode"), 1);

        assert_eq!(snippet(&db), "I'm \"in\"");
    }

    #[test]
    fn decodes_only_once() {
        // The first run turns Gmail's `&amp;lt;` into the literal text `&lt;`,
        // which a second run must leave alone.
        let db = db_with_snippet("type &amp;lt;b&amp;gt;");
        db.decode_stored_snippets_once().expect("first run");

        assert_eq!(db.decode_stored_snippets_once().expect("second run"), 0);

        assert_eq!(snippet(&db), "type &lt;b&gt;");
    }
}
