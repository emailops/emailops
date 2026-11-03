//! Mail stored before inline images were downloaded by attachment id keeps
//! their `cid:` references, which render as empty boxes, and no later sync
//! rewrites a stored body. So an old message is fetched again when it is opened.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::services::logger::log;
use crate::sync::provider::EmailProvider;

use super::provider::build_provider_for_account;

/// Room for a message with several large images, without leaving an old
/// message on a spinner when the network is gone.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Messages the provider has answered for in this run. A reply can quote an
/// image it does not carry, so its body stays unresolved: this keeps such a
/// message from costing a request on every open.
static ANSWERED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

fn answered(email_id: &str) -> bool {
    ANSWERED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(email_id)
}

fn mark_answered(email_id: &str) {
    ANSWERED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(email_id.to_string());
}

/// Pure: how many images `body` still points at by content id.
fn unresolved_images(body: &str) -> usize {
    ["src=\"cid:", "src='cid:", "src=cid:"]
        .iter()
        .map(|reference| body.matches(reference).count())
        .sum()
}

/// The body of `email_id`, with the inline images it lost if the provider can
/// give them back. A failure only logs: the stored body is still worth showing.
pub async fn body_with_inline_images(db: &Arc<Database>, email_id: &str) -> Result<String> {
    let body = db.get_email_body(email_id)?;
    if unresolved_images(&body) == 0 || answered(email_id) {
        return Ok(body);
    }
    let Some(email) = db.get_email(email_id)? else {
        return Ok(body);
    };
    let Some(account) = db.get_account(&email.account_id)? else {
        return Ok(body);
    };
    // Only these two ever stored a body without its images.
    if !matches!(account.provider.as_str(), "gmail" | "outlook") {
        return Ok(body);
    }
    match build_provider_for_account(&account, None).await {
        Ok(provider) => Ok(repair_body(db, email_id, body, provider.as_ref()).await),
        Err(e) => {
            log(
                "warn",
                "sync",
                format!("Could not reach the provider for {email_id}: {e}"),
            );
            Ok(body)
        }
    }
}

/// Fetch the message again and keep the result if it resolves more images.
async fn repair_body(db: &Database, email_id: &str, body: String, provider: &dyn EmailProvider) -> String {
    let fresh = match tokio::time::timeout(FETCH_TIMEOUT, provider.get_message(email_id)).await {
        Ok(Ok((fresh, _category, _attachments))) => fresh,
        Ok(Err(AppError::NotFound(_))) => {
            mark_answered(email_id);
            return body;
        }
        Ok(Err(e)) => {
            log("warn", "sync", format!("Could not fetch the images of {email_id}: {e}"));
            return body;
        }
        Err(_) => {
            log("warn", "sync", format!("Timed out fetching the images of {email_id}"));
            return body;
        }
    };
    mark_answered(email_id);
    if unresolved_images(&fresh.body) >= unresolved_images(&body) {
        return body;
    }
    match db.set_email_body(email_id, &fresh.body) {
        Ok(()) => fresh.body,
        Err(e) => {
            log("error", "sync", format!("Could not save the images of {email_id}: {e}"));
            fresh.body
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::attachments::tests::{make_account, make_email};
    use crate::sync::provider::{EmailCategory, FakeEmailProvider};

    const OLD: &str = r#"<p>See <img src="cid:chart" alt="image.png"></p>"#;
    const FIXED: &str = r#"<p>See <img src="data:image/png;base64,AAA" alt="image.png"></p>"#;

    fn stored(id: &str, provider: &str) -> Arc<Database> {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        make_account(&db, "acc-1", provider, "me@example.com");
        let mut email = make_email("acc-1", id, "ana@example.org", "Report");
        email.body = OLD.to_string();
        db.insert_email(&email).expect("insert email");
        db
    }

    fn holding(id: &str, body: &str) -> FakeEmailProvider {
        let fake = FakeEmailProvider::new("me@example.com", "Me");
        let mut email = make_email("acc-1", id, "ana@example.org", "Report");
        email.body = body.to_string();
        fake.add_message(email, EmailCategory::Primary, Vec::new());
        fake
    }

    #[test]
    fn counts_the_images_a_body_points_at_by_content_id() {
        let cases = [
            (r#"<img src="cid:a">"#, 1),
            ("<img src='cid:a'><img src='cid:b'>", 2),
            ("<img src=cid:a>", 1),
            (r#"<img src="data:image/png;base64,AAA">"#, 0),
            ("<p>The header is called cid: in the spec</p>", 0),
        ];
        for (body, images) in cases {
            assert_eq!(unresolved_images(body), images, "{body}");
        }
    }

    #[tokio::test]
    async fn an_old_body_gets_its_images_back_and_keeps_them() {
        let db = stored("repair-back", "gmail");
        let fake = holding("repair-back", FIXED);

        let body = repair_body(&db, "repair-back", OLD.to_string(), &fake).await;

        assert_eq!(body, FIXED);
        assert_eq!(db.get_email_body("repair-back").expect("body"), FIXED);
    }

    #[tokio::test]
    async fn a_fetch_that_resolves_nothing_more_keeps_the_stored_body() {
        let db = stored("repair-same", "gmail");
        let fake = holding("repair-same", OLD);

        let body = repair_body(&db, "repair-same", OLD.to_string(), &fake).await;

        assert_eq!(body, OLD);
        assert!(
            answered("repair-same"),
            "a quoted image has no part to fetch: do not ask on every open"
        );
    }

    #[tokio::test]
    async fn a_failed_fetch_keeps_the_stored_body_and_tries_again_next_time() {
        let db = stored("repair-offline", "gmail");
        let fake = holding("repair-offline", FIXED);
        fake.fail_message("repair-offline");

        let body = repair_body(&db, "repair-offline", OLD.to_string(), &fake).await;

        assert_eq!(body, OLD);
        assert!(!answered("repair-offline"));
    }

    #[tokio::test]
    async fn a_message_gone_from_the_provider_is_not_asked_for_again() {
        let db = stored("repair-gone", "gmail");
        let fake = FakeEmailProvider::new("me@example.com", "Me");

        let body = repair_body(&db, "repair-gone", OLD.to_string(), &fake).await;

        assert_eq!(body, OLD);
        assert!(answered("repair-gone"));
    }

    #[tokio::test]
    async fn imap_mail_never_lost_its_images_and_is_left_alone() {
        let db = stored("repair-imap", "imap");

        let body = body_with_inline_images(&db, "repair-imap").await.expect("body");

        assert_eq!(body, OLD);
        assert!(!answered("repair-imap"), "no provider was built for it");
    }
}
