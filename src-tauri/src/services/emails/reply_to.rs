//! Where a reply to a message goes: its `Reply-To`, when it names a valid
//! address, instead of its sender (#270). Read from the headers captured at
//! sync; mail synced before they were captured has none and is answered at its
//! `From`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::services::list::is_full_address;
use crate::util::email_addr::{split_address_list, split_name_addr};

/// The valid addresses of a `Reply-To` value, lowercased, in order, without
/// repeats. Empty when it names none, so the reply falls back to `From`.
pub(crate) fn reply_to_addresses(raw: &str) -> Vec<String> {
    let mut addresses: Vec<String> = Vec::new();
    for entry in split_address_list(raw) {
        let (_, address) = split_name_addr(&entry);
        if is_full_address(&address) && !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    addresses
}

/// Email id → the `Reply-To` addresses of every message of the thread that has
/// any.
pub fn thread_reply_to(db: &Arc<Database>, account_id: &str, thread_id: &str) -> Result<HashMap<String, Vec<String>>> {
    let ids: Vec<String> = db
        .get_thread(account_id, thread_id)?
        .into_iter()
        .map(|e| e.id)
        .collect();
    Ok(db
        .get_email_headers_batch(&ids)?
        .into_iter()
        .filter_map(|(id, headers)| {
            let addresses = reply_to_addresses(headers.reply_to.as_deref()?);
            (!addresses.is_empty()).then_some((id, addresses))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_addresses_of_a_reply_to_header() {
        assert_eq!(
            reply_to_addresses(r#""Sales, EU" <Sales@Shop.example>, help@shop.example"#),
            vec!["sales@shop.example", "help@shop.example"]
        );
    }

    #[test]
    fn drops_what_is_not_an_address_and_repeats() {
        assert_eq!(
            reply_to_addresses("undisclosed-recipients:;, a@shop.example, A@shop.example, not an address"),
            vec!["a@shop.example"]
        );
        assert!(reply_to_addresses("").is_empty());
        assert!(reply_to_addresses("nobody").is_empty());
    }

    #[test]
    fn a_thread_answers_each_message_at_its_reply_to() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        let conn = db.connection();
        for (id, ts) in [("e-list", 100), ("e-plain", 200)] {
            conn.execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                 VALUES (?1, 'acc', 't1', 's', 'List', 'list@shop.example', 'shop.example',
                         '[]', '[]', '', ?2, 0, 'primary', 'inbox', 0)",
                rusqlite::params![id, ts],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO email_headers (email_id, account_id, reply_to, captured_at)
             VALUES ('e-list', 'acc', 'Team <team@shop.example>', 0),
                    ('e-plain', 'acc', NULL, 0)",
            [],
        )
        .unwrap();
        drop(conn);

        let got = thread_reply_to(&db, "acc", "t1").unwrap();

        assert_eq!(got.get("e-list"), Some(&vec!["team@shop.example".to_string()]));
        assert_eq!(got.get("e-plain"), None, "no Reply-To: the reply goes to From");
    }
}
