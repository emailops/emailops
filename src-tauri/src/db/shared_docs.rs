//! Shared documents (V036). Storage only — the CRDT, the envelope and the
//! mail transport live in `services::shared_docs`.

use rusqlite::{params, OptionalExtension, Row};

use crate::db::Database;
use crate::models::error::Result;
use crate::models::shared_docs::{DocKind, DocStatus, SharedDoc};

/// A new document row.
pub struct NewSharedDoc<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub kind: DocKind,
    pub title: &'a str,
    pub state: &'a [u8],
    pub status: DocStatus,
    pub participants: &'a [String],
    pub now: i64,
}

/// A participant and what it is known to have seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub address: String,
    pub state_vector: Option<Vec<u8>>,
}

const DOC_COLUMNS: &str = "id, account_id, kind, title, status, consented_at, dirty_since, created_at, updated_at";

fn doc_from_row(r: &Row<'_>) -> rusqlite::Result<SharedDoc> {
    let parse_err = |i: usize, e: crate::models::error::AppError| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(SharedDoc {
        id: r.get(0)?,
        account_id: r.get(1)?,
        kind: DocKind::parse(&r.get::<_, String>(2)?).map_err(|e| parse_err(2, e))?,
        title: r.get(3)?,
        status: DocStatus::parse(&r.get::<_, String>(4)?).map_err(|e| parse_err(4, e))?,
        participants: Vec::new(),
        consented_at: r.get(5)?,
        dirty_since: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

impl Database {
    /// Store a new document with its participants, in one transaction.
    pub fn insert_shared_doc(&self, doc: &NewSharedDoc<'_>) -> Result<()> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO shared_docs (id, account_id, kind, title, state, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                doc.id,
                doc.account_id,
                doc.kind.as_str(),
                doc.title,
                doc.state,
                doc.status.as_str(),
                doc.now
            ],
        )?;
        for (position, address) in doc.participants.iter().enumerate() {
            tx.execute(
                "INSERT INTO shared_doc_participants (doc_id, address, position) VALUES (?1, ?2, ?3)",
                params![doc.id, address, position as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One document with its participants, whichever account holds it.
    pub fn get_shared_doc(&self, doc_id: &str) -> Result<Option<SharedDoc>> {
        let doc = self
            .reader()
            .query_row(
                &format!("SELECT {DOC_COLUMNS} FROM shared_docs WHERE id = ?1"),
                params![doc_id],
                doc_from_row,
            )
            .optional()?;
        let Some(mut doc) = doc else {
            return Ok(None);
        };
        doc.participants = self
            .shared_doc_participants(doc_id)?
            .into_iter()
            .map(|p| p.address)
            .collect();
        Ok(Some(doc))
    }

    /// The documents of one account (or of every account), newest change
    /// first, with their participants.
    pub fn list_shared_docs(&self, account_id: Option<&str>) -> Result<Vec<SharedDoc>> {
        let mut docs = {
            let conn = self.reader();
            let mut stmt = conn.prepare(&format!(
                "SELECT {DOC_COLUMNS} FROM shared_docs
                 WHERE ?1 IS NULL OR account_id = ?1
                 ORDER BY updated_at DESC, id"
            ))?;
            let rows = stmt.query_map(params![account_id], doc_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for doc in &mut docs {
            doc.participants = self
                .shared_doc_participants(&doc.id)?
                .into_iter()
                .map(|p| p.address)
                .collect();
        }
        Ok(docs)
    }

    pub fn shared_doc_state(&self, doc_id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .reader()
            .query_row("SELECT state FROM shared_docs WHERE id = ?1", params![doc_id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Replace a document's state. `dirty` marks it as having changes to mail
    /// (keeping the earliest such time); a change merged from a peer passes
    /// `false` and leaves the mark as it was.
    pub fn set_shared_doc_state(&self, doc_id: &str, state: &[u8], dirty: bool, now: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE shared_docs SET state = ?2, updated_at = ?3,
                 dirty_since = CASE WHEN ?4 THEN COALESCE(dirty_since, ?3) ELSE dirty_since END
             WHERE id = ?1",
            params![doc_id, state, now, dirty],
        )?;
        Ok(())
    }

    /// Mark a document as having changes to mail, without touching its state
    /// (a peer turned out to lack something we have).
    pub fn mark_shared_doc_dirty(&self, doc_id: &str, now: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE shared_docs SET dirty_since = COALESCE(dirty_since, ?2) WHERE id = ?1",
            params![doc_id, now],
        )?;
        Ok(())
    }

    /// Take a document's pending changes for mailing: clears `dirty_since` if
    /// it is still `expected`. Returns whether this caller took them — a
    /// second flush pass, or an edit that re-marked it, makes this false.
    pub fn claim_shared_doc_flush(&self, doc_id: &str, expected: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE shared_docs SET dirty_since = NULL WHERE id = ?1 AND dirty_since = ?2",
            params![doc_id, expected],
        )?;
        Ok(n == 1)
    }

    pub fn set_shared_doc_status(
        &self,
        doc_id: &str,
        status: DocStatus,
        consented_at: Option<i64>,
        now: i64,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE shared_docs SET status = ?2, consented_at = COALESCE(?3, consented_at), updated_at = ?4
             WHERE id = ?1",
            params![doc_id, status.as_str(), consented_at, now],
        )?;
        Ok(())
    }

    /// A document's participants, in the order they were added.
    pub fn shared_doc_participants(&self, doc_id: &str) -> Result<Vec<Participant>> {
        let conn = self.reader();
        let mut stmt = conn
            .prepare("SELECT address, state_vector FROM shared_doc_participants WHERE doc_id = ?1 ORDER BY position")?;
        let rows = stmt.query_map(params![doc_id], |r| {
            Ok(Participant {
                address: r.get(0)?,
                state_vector: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Add the participants not stored yet, after the existing ones.
    pub fn add_shared_doc_participants(&self, doc_id: &str, addresses: &[String]) -> Result<()> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        for address in addresses {
            tx.execute(
                "INSERT INTO shared_doc_participants (doc_id, address, position)
                 VALUES (?1, ?2, (SELECT COALESCE(MAX(position), -1) + 1 FROM shared_doc_participants WHERE doc_id = ?1))
                 ON CONFLICT(doc_id, address) DO NOTHING",
                params![doc_id, address],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record what `addresses` are known to have seen.
    pub fn set_participant_state_vectors(&self, doc_id: &str, addresses: &[String], state_vector: &[u8]) -> Result<()> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        for address in addresses {
            tx.execute(
                "UPDATE shared_doc_participants SET state_vector = ?3 WHERE doc_id = ?1 AND address = ?2",
                params![doc_id, address, state_vector],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether this account already processed `message_id`.
    pub fn shared_doc_message_seen(&self, account_id: &str, message_id: &str) -> Result<bool> {
        Ok(self
            .reader()
            .query_row(
                "SELECT 1 FROM shared_doc_messages WHERE account_id = ?1 AND message_id = ?2",
                params![account_id, message_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn record_shared_doc_message(&self, account_id: &str, message_id: &str, doc_id: &str, now: i64) -> Result<()> {
        self.connection().execute(
            "INSERT INTO shared_doc_messages (account_id, message_id, doc_id, seen_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, message_id) DO NOTHING",
            params![account_id, message_id, doc_id, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Database {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-2");
        db
    }

    fn insert(db: &Database, id: &str, account: &str, now: i64) {
        db.insert_shared_doc(&NewSharedDoc {
            id,
            account_id: account,
            kind: DocKind::Doc,
            title: "Notes",
            state: b"s0",
            status: DocStatus::Active,
            participants: &["me@example.com".to_string(), "you@example.org".to_string()],
            now,
        })
        .unwrap();
    }

    #[test]
    fn a_document_is_stored_with_its_participants_in_order() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        let doc = db.get_shared_doc("d1").unwrap().unwrap();
        assert_eq!(doc.participants, vec!["me@example.com", "you@example.org"]);
        assert_eq!(
            (doc.status, doc.consented_at, doc.dirty_since),
            (DocStatus::Active, None, None)
        );
        assert_eq!(db.shared_doc_state("d1").unwrap().unwrap(), b"s0");
        assert!(db.get_shared_doc("nope").unwrap().is_none());
    }

    #[test]
    fn listing_filters_by_account_newest_change_first() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        insert(&db, "d2", "acc-1", 20);
        insert(&db, "d3", "acc-2", 30);
        let ids: Vec<_> = db
            .list_shared_docs(Some("acc-1"))
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec!["d2", "d1"]);
        assert_eq!(db.list_shared_docs(None).unwrap().len(), 3);
    }

    #[test]
    fn a_local_change_keeps_the_first_dirty_time_and_a_merge_leaves_it() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        db.set_shared_doc_state("d1", b"s1", false, 11).unwrap();
        assert_eq!(db.get_shared_doc("d1").unwrap().unwrap().dirty_since, None);
        db.set_shared_doc_state("d1", b"s2", true, 12).unwrap();
        db.set_shared_doc_state("d1", b"s3", true, 13).unwrap();
        let doc = db.get_shared_doc("d1").unwrap().unwrap();
        assert_eq!((doc.dirty_since, doc.updated_at), (Some(12), 13));
    }

    #[test]
    fn only_one_flush_claims_the_pending_changes() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        db.mark_shared_doc_dirty("d1", 15).unwrap();
        assert!(!db.claim_shared_doc_flush("d1", 14).unwrap(), "stale expectation");
        assert!(db.claim_shared_doc_flush("d1", 15).unwrap());
        assert!(!db.claim_shared_doc_flush("d1", 15).unwrap(), "already taken");
        assert_eq!(db.get_shared_doc("d1").unwrap().unwrap().dirty_since, None);
    }

    #[test]
    fn participants_are_added_once_and_their_vectors_recorded() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        db.add_shared_doc_participants("d1", &["you@example.org".into(), "new@example.net".into()])
            .unwrap();
        db.set_participant_state_vectors("d1", &["new@example.net".into()], b"sv")
            .unwrap();
        let participants = db.shared_doc_participants("d1").unwrap();
        assert_eq!(
            participants,
            vec![
                Participant {
                    address: "me@example.com".into(),
                    state_vector: None
                },
                Participant {
                    address: "you@example.org".into(),
                    state_vector: None
                },
                Participant {
                    address: "new@example.net".into(),
                    state_vector: Some(b"sv".to_vec())
                },
            ]
        );
    }

    #[test]
    fn status_changes_keep_the_first_consent() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        db.set_shared_doc_status("d1", DocStatus::Active, Some(20), 20).unwrap();
        db.set_shared_doc_status("d1", DocStatus::Left, None, 30).unwrap();
        let doc = db.get_shared_doc("d1").unwrap().unwrap();
        assert_eq!((doc.status, doc.consented_at), (DocStatus::Left, Some(20)));
    }

    #[test]
    fn a_processed_message_is_remembered_per_account() {
        let db = db();
        assert!(!db.shared_doc_message_seen("acc-1", "m1").unwrap());
        db.record_shared_doc_message("acc-1", "m1", "d1", 5).unwrap();
        db.record_shared_doc_message("acc-1", "m1", "d1", 6).unwrap();
        assert!(db.shared_doc_message_seen("acc-1", "m1").unwrap());
        assert!(!db.shared_doc_message_seen("acc-2", "m1").unwrap());
    }

    #[test]
    fn deleting_the_account_removes_its_documents() {
        let db = db();
        insert(&db, "d1", "acc-1", 10);
        db.connection()
            .execute("DELETE FROM accounts WHERE id = 'acc-1'", [])
            .unwrap();
        assert!(db.get_shared_doc("d1").unwrap().is_none());
        assert!(db.shared_doc_participants("d1").unwrap().is_empty());
    }
}
