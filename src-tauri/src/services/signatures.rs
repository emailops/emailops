//! Per-account email signatures: one rich-HTML signature per account, plus
//! where the composer inserts it (new messages, replies and forwards).
//!
//! The HTML is sanitized on save with the allowlist the send path applies
//! (`sanitize_outgoing_html`), so what is stored is already what can go out.
//! Inserting it into a composer is the frontend's job (`src/lib/signature.ts`).

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, AccountSignature, SignatureInput};
use crate::services::emails::sanitize_outgoing_html;
use crate::sync::provider::EmailProvider;

/// Largest signature accepted, in bytes of HTML. Room for a pasted logo as a
/// data-URL image (the send path turns it into an inline `cid:` part), not
/// for a photo album travelling with every message.
pub const MAX_SIGNATURE_BYTES: usize = 512 * 1024;

/// Pure: the HTML to store for what the editor produced. Sanitized with the
/// outgoing allowlist; a signature with no text and no image is no signature.
pub fn clean_signature_html(html: &str) -> String {
    let sanitized = sanitize_outgoing_html(html.trim());
    let text = ammonia::Builder::empty().clean(&sanitized).to_string();
    let has_text = !text.replace("&nbsp;", " ").trim().is_empty();
    if has_text || sanitized.contains("<img") {
        sanitized
    } else {
        String::new()
    }
}

fn require_account(db: &Database, account_id: &str) -> Result<Account> {
    db.get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))
}

/// The signature of an account; the defaults (none, used everywhere) when it
/// never saved one. `NotFound` for an unknown account.
pub fn get_signature(db: &Database, account_id: &str) -> Result<AccountSignature> {
    require_account(db, account_id)?;
    Ok(db
        .get_account_signature(account_id)?
        .unwrap_or_else(|| AccountSignature {
            account_id: account_id.to_string(),
            html: String::new(),
            use_for_new: true,
            use_for_replies: true,
            updated_at: None,
        }))
}

/// Save an account's signature, sanitized. Returns what was stored.
pub fn save_signature(db: &Database, account_id: &str, input: SignatureInput, now: i64) -> Result<AccountSignature> {
    require_account(db, account_id)?;
    if input.html.len() > MAX_SIGNATURE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "The signature is too large ({} KB, at most {} KB). Use a smaller image.",
            input.html.len() / 1024,
            MAX_SIGNATURE_BYTES / 1024
        )));
    }
    let html = clean_signature_html(&input.html);
    db.upsert_account_signature(account_id, &html, input.use_for_new, input.use_for_replies, now)?;
    Ok(AccountSignature {
        account_id: account_id.to_string(),
        html,
        use_for_new: input.use_for_new,
        use_for_replies: input.use_for_replies,
        updated_at: Some(now),
    })
}

/// The signature the provider's own client uses for this account (Gmail's
/// "Send mail as" signature), sanitized like a saved one. Not stored: the
/// editor shows it and the user saves. `None` when the provider has none.
pub async fn import_provider_signature(
    db: &Database,
    account_id: &str,
    provider: &dyn EmailProvider,
) -> Result<Option<String>> {
    let account = require_account(db, account_id)?;
    let html = provider.get_signature(&account.email).await?;
    Ok(html.map(|h| clean_signature_html(&h)).filter(|h| !h.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn db_with_account(id: &str) -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', 'ana@example.com', 'Ana', 0)",
                rusqlite::params![id],
            )
            .expect("insert account");
        db
    }

    fn input(html: &str) -> SignatureInput {
        SignatureInput {
            html: html.to_string(),
            use_for_new: true,
            use_for_replies: false,
        }
    }

    #[test]
    fn an_account_without_a_signature_gets_the_defaults() {
        let db = db_with_account("acc1");
        assert_eq!(
            get_signature(&db, "acc1").unwrap(),
            AccountSignature {
                account_id: "acc1".into(),
                html: String::new(),
                use_for_new: true,
                use_for_replies: true,
                updated_at: None,
            }
        );
    }

    #[test]
    fn a_saved_signature_is_read_back_with_its_options() {
        let db = db_with_account("acc1");
        let saved = save_signature(&db, "acc1", input("<p>Ana <strong>Lopez</strong></p>"), NOW).unwrap();
        assert_eq!(saved.html, "<p>Ana <strong>Lopez</strong></p>");
        assert_eq!(get_signature(&db, "acc1").unwrap(), saved);
        assert!(saved.use_for_new);
        assert!(!saved.use_for_replies);
        assert_eq!(saved.updated_at, Some(NOW));
    }

    #[test]
    fn saving_again_replaces_the_signature() {
        let db = db_with_account("acc1");
        save_signature(&db, "acc1", input("<p>One</p>"), NOW).unwrap();
        save_signature(&db, "acc1", input("<p>Two</p>"), NOW + 1).unwrap();
        assert_eq!(get_signature(&db, "acc1").unwrap().html, "<p>Two</p>");
    }

    #[test]
    fn scripts_and_event_handlers_are_stripped_on_save() {
        let db = db_with_account("acc1");
        let saved = save_signature(
            &db,
            "acc1",
            input(r#"<p onclick="alert(1)">Ana</p><script>alert(2)</script><img src="x" onerror="alert(3)">"#),
            NOW,
        )
        .unwrap();
        assert!(!saved.html.contains("script"), "{}", saved.html);
        assert!(!saved.html.contains("onclick"), "{}", saved.html);
        assert!(!saved.html.contains("onerror"), "{}", saved.html);
        assert!(!saved.html.contains("alert"), "{}", saved.html);
        assert!(saved.html.contains("Ana"));
    }

    #[test]
    fn a_javascript_link_loses_its_href() {
        let out = clean_signature_html(r#"<p><a href="javascript:alert(1)">site</a></p>"#);
        assert!(!out.contains("javascript:"), "{out}");
        assert!(out.contains("site"));
    }

    #[test]
    fn a_pasted_data_url_logo_is_kept() {
        let out = clean_signature_html(r#"<p><img src="data:image/png;base64,AAAA" alt="logo"></p>"#);
        assert!(out.contains("data:image/png;base64,AAAA"), "{out}");
    }

    #[test]
    fn a_signature_with_no_text_or_image_is_stored_as_none() {
        for blank in ["", "   ", "<p></p>", "<p><br></p><p> </p>", "<script>x</script>"] {
            assert_eq!(clean_signature_html(blank), "", "{blank:?}");
        }
    }

    #[test]
    fn an_oversized_signature_is_refused() {
        let db = db_with_account("acc1");
        let huge = format!("<p>{}</p>", "a".repeat(MAX_SIGNATURE_BYTES));
        let err = save_signature(&db, "acc1", input(&huge), NOW).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        assert!(get_signature(&db, "acc1").unwrap().updated_at.is_none());
    }

    #[test]
    fn an_unknown_account_is_not_found() {
        let db = db_with_account("acc1");
        assert!(matches!(get_signature(&db, "nope"), Err(AppError::NotFound(_))));
        assert!(matches!(
            save_signature(&db, "nope", input("<p>x</p>"), NOW),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn the_signature_goes_with_its_account() {
        let db = db_with_account("acc1");
        save_signature(&db, "acc1", input("<p>Ana</p>"), NOW).unwrap();
        db.connection()
            .execute("DELETE FROM accounts WHERE id = 'acc1'", [])
            .unwrap();
        assert!(db.get_account_signature("acc1").unwrap().is_none());
    }

    #[tokio::test]
    async fn the_provider_signature_is_imported_sanitized() {
        let db = db_with_account("acc1");
        let provider = crate::sync::provider::FakeEmailProvider::new("ana@example.com", "Ana");
        provider.set_signature(Some(
            r#"<div onmouseover="x()">Ana <b>Lopez</b></div><script>y()</script>"#,
        ));
        let html = import_provider_signature(&db, "acc1", &provider)
            .await
            .unwrap()
            .unwrap();
        assert!(html.contains("Ana <b>Lopez</b>"), "{html}");
        assert!(!html.contains("onmouseover") && !html.contains("script"), "{html}");
        // Importing does not save.
        assert!(get_signature(&db, "acc1").unwrap().updated_at.is_none());
    }

    #[tokio::test]
    async fn a_provider_without_a_signature_imports_none() {
        let db = db_with_account("acc1");
        let provider = crate::sync::provider::FakeEmailProvider::new("ana@example.com", "Ana");
        assert_eq!(import_provider_signature(&db, "acc1", &provider).await.unwrap(), None);
        provider.set_signature(Some("<p> </p>"));
        assert_eq!(import_provider_signature(&db, "acc1", &provider).await.unwrap(), None);
    }
}
