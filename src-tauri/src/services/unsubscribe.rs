//! One-click unsubscribe (RFC 2369 `List-Unsubscribe` + RFC 8058
//! `List-Unsubscribe-Post`).
//!
//! The raw headers never reach the webview (see `Email::headers`). The reading
//! pane asks for a derived [`UnsubscribeOption`] — how the list can be left
//! and which host or address that contacts — and the unsubscribe itself is
//! carried out here, re-parsing the stored headers rather than trusting
//! anything the frontend sends back.
//!
//! Three methods, preferred in this order:
//!
//! 1. **One-click** (RFC 8058): an HTTPS POST of `List-Unsubscribe=One-Click`
//!    to the list's https URI. No browser, no page, no confirmation step on
//!    the sender's side.
//! 2. **Mailto**: an email from the account that received the message.
//! 3. **Link**: the sender's unsubscribe page, opened in the system browser by
//!    the frontend (never fetched by the backend — a page is for a person).
//!
//! Contacting the list is a third-party call the user explicitly asks for;
//! see DECISIONS 2026-10-01 "One-click unsubscribe contacts the sender only
//! when the user asks".

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
#[cfg(feature = "ts")]
use ts_rs::TS;
use url::Url;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Account;
use crate::services::logger;
use crate::sync::provider::{EmailBody, EmailProvider};

/// Longest `List-Unsubscribe` header considered. Real ones are a few hundred
/// bytes; anything this long is junk or an attack on the parser.
const MAX_HEADER_LEN: usize = 8 * 1024;
/// Longest single URI considered.
const MAX_URI_LEN: usize = 2048;
/// Caps on what a mailto may put in the message we send for the user.
const MAX_MAILTO_SUBJECT: usize = 256;
const MAX_MAILTO_BODY: usize = 2048;

/// How a list can be left, fully parsed. Backend-only: the webview gets the
/// minimal [`UnsubscribeOption`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsubscribeMethod {
    /// RFC 8058: POST `List-Unsubscribe=One-Click` to this https URL.
    OneClick { url: Url },
    /// Send an email to `to`, with the subject and body the list asked for.
    Mailto {
        to: String,
        subject: Option<String>,
        body: Option<String>,
    },
    /// The sender's unsubscribe page (https only).
    Link { url: Url },
}

/// Which method the reading pane offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub enum UnsubscribeKind {
    OneClick,
    Mailto,
    Link,
}

impl UnsubscribeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UnsubscribeKind::OneClick => "one_click",
            UnsubscribeKind::Mailto => "mailto",
            UnsubscribeKind::Link => "link",
        }
    }
}

/// What the webview may know about a message's unsubscribe option: the
/// method, the host or address it contacts (for the confirmation dialog), and
/// — for a link only, since the frontend opens it — the validated https URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeOption {
    pub kind: UnsubscribeKind,
    /// The host a one-click or link contacts, or the mailto address.
    pub target: String,
    /// The page to open; `Some` only for [`UnsubscribeKind::Link`].
    pub url: Option<String>,
}

impl UnsubscribeMethod {
    pub fn kind(&self) -> UnsubscribeKind {
        match self {
            UnsubscribeMethod::OneClick { .. } => UnsubscribeKind::OneClick,
            UnsubscribeMethod::Mailto { .. } => UnsubscribeKind::Mailto,
            UnsubscribeMethod::Link { .. } => UnsubscribeKind::Link,
        }
    }

    /// The minimal, webview-safe view of this method.
    pub fn option(&self) -> UnsubscribeOption {
        match self {
            UnsubscribeMethod::OneClick { url } => UnsubscribeOption {
                kind: UnsubscribeKind::OneClick,
                target: url.host_str().unwrap_or_default().to_string(),
                url: None,
            },
            UnsubscribeMethod::Mailto { to, .. } => UnsubscribeOption {
                kind: UnsubscribeKind::Mailto,
                target: to.clone(),
                url: None,
            },
            UnsubscribeMethod::Link { url } => UnsubscribeOption {
                kind: UnsubscribeKind::Link,
                target: url.host_str().unwrap_or_default().to_string(),
                url: Some(url.to_string()),
            },
        }
    }
}

/// Pure: the best way to leave the list, from the stored `List-Unsubscribe`
/// and `List-Unsubscribe-Post` values. `None` when the message offers no
/// usable method.
///
/// Only `<…>`-bracketed URIs count (RFC 2369); of those only `https:` and
/// `mailto:` — `http:` would send the request in the clear, and anything else
/// (`javascript:`, `file:`, custom schemes) is never followed. One-click needs
/// the exact RFC 8058 `List-Unsubscribe=One-Click` value *and* an https URI.
pub fn parse_unsubscribe(
    list_unsubscribe: Option<&str>,
    list_unsubscribe_post: Option<&str>,
) -> Option<UnsubscribeMethod> {
    let raw = list_unsubscribe?;
    let header = raw.trim();
    if header.is_empty() || raw.len() > MAX_HEADER_LEN {
        return None;
    }
    let mut https: Option<Url> = None;
    let mut mailto: Option<UnsubscribeMethod> = None;
    for uri in bracketed_uris(header) {
        if uri.len() > MAX_URI_LEN {
            continue;
        }
        let lower = uri.to_ascii_lowercase();
        if lower.starts_with("https:") {
            if https.is_none() {
                https = parse_https(uri);
            }
        } else if lower.starts_with("mailto:") && mailto.is_none() {
            mailto = parse_mailto(&uri["mailto:".len()..]);
        }
    }
    let one_click = list_unsubscribe_post.is_some_and(is_one_click_post);
    match (https, mailto) {
        (Some(url), _) if one_click => Some(UnsubscribeMethod::OneClick { url }),
        (_, Some(mailto)) => Some(mailto),
        (Some(url), None) => Some(UnsubscribeMethod::Link { url }),
        (None, None) => None,
    }
}

/// The URIs between `<` and `>`, in header order. Text outside brackets
/// (comments, stray words) is ignored; an unclosed bracket ends the list.
fn bracketed_uris(header: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            break;
        };
        let uri = after[..end].trim();
        if !uri.is_empty() {
            out.push(uri);
        }
        rest = &after[end + 1..];
    }
    out
}

fn is_one_click_post(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("List-Unsubscribe=One-Click")
}

/// An https URL with a host and no credentials. Whitespace inside is folding
/// left over from the header and is removed first.
fn parse_https(uri: &str) -> Option<Url> {
    let compact: String = uri.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(&compact).ok()?;
    let has_host = url.host_str().is_some_and(|h| !h.is_empty());
    let has_credentials = !url.username().is_empty() || url.password().is_some();
    (url.scheme() == "https" && has_host && !has_credentials).then_some(url)
}

/// `addr?subject=…&body=…` (after `mailto:`). Exactly one address; header
/// fields other than subject and body are ignored (a list never needs Cc or
/// Bcc to unsubscribe you).
fn parse_mailto(rest: &str) -> Option<UnsubscribeMethod> {
    let (addr, query) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };
    let to = urlencoding::decode(addr.trim()).ok()?.trim().to_string();
    if !is_plain_address(&to) {
        return None;
    }
    let mut subject = None;
    let mut body = None;
    for pair in query.unwrap_or_default().split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let decoded = urlencoding::decode(value).ok()?.into_owned();
        match key.to_ascii_lowercase().as_str() {
            "subject" => {
                // A subject is one header line: folding it keeps a crafted
                // `%0D%0A` from injecting headers into the message we send.
                let line: String = decoded.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
                let line = line.trim().to_string();
                if line.chars().count() > MAX_MAILTO_SUBJECT {
                    return None;
                }
                subject = Some(line).filter(|s| !s.is_empty());
            }
            "body" => {
                if decoded.chars().count() > MAX_MAILTO_BODY {
                    return None;
                }
                body = Some(decoded).filter(|s| !s.trim().is_empty());
            }
            _ => {}
        }
    }
    Some(UnsubscribeMethod::Mailto { to, subject, body })
}

/// One `local@domain` address: no list, no display name, no whitespace or
/// control characters, a dotted domain.
fn is_plain_address(addr: &str) -> bool {
    if addr.is_empty() || addr.len() > 254 {
        return false;
    }
    if addr
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | ';' | '<' | '>' | '"' | '(' | ')'))
    {
        return false;
    }
    match addr.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    }
}

/// How long a one-click POST may take to connect, and in total. A list
/// server that does not answer in this time is reported as a failure; the
/// user can try again.
const ONE_CLICK_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ONE_CLICK_TIMEOUT: Duration = Duration::from_secs(20);
/// Redirects a one-click POST may follow — only to https.
const ONE_CLICK_MAX_REDIRECTS: usize = 3;
/// The only header the request carries beyond the form body: no cookies, no
/// referrer, nothing that identifies the user beyond the URL the list chose.
const ONE_CLICK_USER_AGENT: &str = "EmailOps";

/// The HTTP client for one-click POSTs: short timeouts, no cookie store, a
/// plain user agent, and redirects followed only to https (at most
/// [`ONE_CLICK_MAX_REDIRECTS`]).
pub fn one_click_client() -> Result<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= ONE_CLICK_MAX_REDIRECTS || attempt.url().scheme() != "https" {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .connect_timeout(ONE_CLICK_CONNECT_TIMEOUT)
        .timeout(ONE_CLICK_TIMEOUT)
        .redirect(policy)
        .user_agent(ONE_CLICK_USER_AGENT)
        .build()
        .map_err(|e| AppError::IoError(format!("could not build the unsubscribe HTTP client: {e}")))
}

/// RFC 8058: POST `List-Unsubscribe=One-Click` as a form to `url`. Any 2xx
/// is success; anything else — including a redirect the client refused to
/// follow — is an error naming the status.
pub async fn post_one_click(client: &reqwest::Client, url: &Url) -> Result<()> {
    let response = client
        .post(url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(AppError::SyncError(format!(
            "the list server answered {status} to the unsubscribe request"
        )))
    }
}

/// Carry out the unsubscribe the message offers, then record it.
///
/// The method is re-derived from the stored headers — the frontend only says
/// which message. One-click POSTs through `client`; mailto sends from the
/// account that received the message through `provider` (`None` = an account
/// that cannot send from here); a link was opened by the frontend in the
/// system browser and is only recorded.
pub async fn unsubscribe(
    db: &Arc<Database>,
    account: &Account,
    email_id: &str,
    client: &reqwest::Client,
    provider: Option<&dyn EmailProvider>,
    now: i64,
) -> Result<UnsubscribeKind> {
    let email = crate::services::ownership::email_in_account(db, &account.id, email_id)?;
    let headers = db
        .get_email_headers_batch(std::slice::from_ref(&email.id))?
        .remove(&email.id);
    let method = headers
        .as_ref()
        .and_then(|h| parse_unsubscribe(h.list_unsubscribe.as_deref(), h.list_unsubscribe_post.as_deref()))
        .ok_or_else(|| AppError::InvalidInput("This message offers no way to unsubscribe".to_string()))?;
    let address = email.sender_email.trim().to_lowercase();
    let kind = method.kind();
    match &method {
        UnsubscribeMethod::OneClick { url } => {
            logger::log(
                "info",
                "account",
                format!("[{}] Unsubscribing from {address} (one-click)", account.email),
            );
            post_one_click(client, url).await?;
        }
        UnsubscribeMethod::Mailto { to, subject, body } => {
            let provider = provider.ok_or_else(|| {
                AppError::InvalidInput("This account cannot send the unsubscribe email from here".to_string())
            })?;
            let body = EmailBody::plain(body.clone().unwrap_or_else(|| "Unsubscribe".to_string())).without_footer();
            crate::services::emails::send_new_email_with_provider(
                db,
                &account.id,
                vec![to.clone()],
                Vec::new(),
                subject.as_deref().unwrap_or("Unsubscribe"),
                &body,
                Vec::new(),
                provider,
            )
            .await?;
        }
        UnsubscribeMethod::Link { .. } => {}
    }
    db.upsert_sender_unsubscribe(&account.id, &address, kind.as_str(), now)?;
    logger::log(
        "success",
        "account",
        match kind {
            UnsubscribeKind::Link => format!("[{}] Opened the unsubscribe page of {address}", account.email),
            _ => format!("[{}] Unsubscribed from {address}", account.email),
        },
    );
    Ok(kind)
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::models::headers::RawHeaders;
    use crate::models::Email;
    use crate::sync::provider::FakeEmailProvider;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn one_click_posts_the_rfc8058_form_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/u/abc"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string("List-Unsubscribe=One-Click"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = one_click_client().unwrap();

        post_one_click(&client, &Url::parse(&format!("{}/u/abc", server.uri())).unwrap())
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert!(requests[0].headers.get("cookie").is_none());
        assert_eq!(
            requests[0].headers.get("user-agent").and_then(|v| v.to_str().ok()),
            Some("EmailOps")
        );
    }

    #[tokio::test]
    async fn a_server_error_is_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let client = one_click_client().unwrap();

        let err = post_one_click(&client, &Url::parse(&server.uri()).unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("500"), "{err}");
    }

    #[tokio::test]
    async fn a_redirect_to_a_non_https_address_is_not_followed() {
        let server = MockServer::start().await;
        let target = format!("{}/landing", server.uri());
        Mock::given(method("POST"))
            .and(path("/u"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/landing"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let client = one_click_client().unwrap();

        let err = post_one_click(&client, &Url::parse(&format!("{}/u", server.uri())).unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("302"), "{err}");
    }

    fn account() -> Account {
        Account {
            id: "acc-1".to_string(),
            provider: "gmail".to_string(),
            email: "me@example.com".to_string(),
            name: "Me".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    fn db_with(list_unsubscribe: &str, post: Option<&str>) -> Arc<Database> {
        let db = Database::new_for_testing().unwrap();
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acc-1', 'gmail', 'me@example.com', 'Me', 0)",
                [],
            )
            .unwrap();
        db.insert_emails_batch(&[Email {
            id: "m1".to_string(),
            account_id: "acc-1".to_string(),
            thread_id: "t1".to_string(),
            message_id: None,
            references: None,
            subject: "Weekly deals".to_string(),
            sender: "Deals".to_string(),
            sender_email: "Deals@Shop.example".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "b".to_string(),
            snippet: "b".to_string(),
            timestamp: 1,
            is_read: true,
            triage_status: None,
            category: "promotions".to_string(),
            mailbox: "inbox".to_string(),
            is_sent: false,
            is_starred: false,
            headers: Some(RawHeaders {
                list_unsubscribe: Some(list_unsubscribe.to_string()),
                list_unsubscribe_post: post.map(str::to_string),
                ..Default::default()
            }),
        }])
        .unwrap();
        Arc::new(db)
    }

    #[tokio::test]
    async fn mailto_sends_the_requested_message_from_the_receiving_account() {
        let db = db_with(
            "<mailto:leave@shop.example?subject=Unsubscribe%20me&body=Remove%20me>",
            None,
        );
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let client = one_click_client().unwrap();

        let kind = unsubscribe(&db, &account(), "m1", &client, Some(&provider), 50)
            .await
            .unwrap();

        assert_eq!(kind, UnsubscribeKind::Mailto);
        let sent = provider.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to_emails, vec!["leave@shop.example".to_string()]);
        assert_eq!(sent[0].from_email, "me@example.com");
        assert_eq!(sent[0].subject, "Unsubscribe me");
        assert_eq!(sent[0].body.text, "Remove me");
        assert!(!sent[0].body.append_footer, "no footer on a machine-read request");
        assert_eq!(
            db.sender_unsubscribed_at("acc-1", "deals@shop.example").unwrap(),
            Some(50)
        );
    }

    #[tokio::test]
    async fn a_link_is_only_recorded_never_fetched() {
        let db = db_with("<https://shop.example/leave>", None);
        let client = one_click_client().unwrap();

        let kind = unsubscribe(&db, &account(), "m1", &client, None, 60).await.unwrap();

        assert_eq!(kind, UnsubscribeKind::Link);
        assert_eq!(
            db.sender_unsubscribed_at("acc-1", "deals@shop.example").unwrap(),
            Some(60)
        );
    }

    #[tokio::test]
    async fn a_message_without_an_option_is_refused_and_nothing_is_recorded() {
        let db = db_with("<http://shop.example/leave>", None);
        let client = one_click_client().unwrap();

        assert!(unsubscribe(&db, &account(), "m1", &client, None, 1).await.is_err());
        assert_eq!(db.sender_unsubscribed_at("acc-1", "deals@shop.example").unwrap(), None);
    }

    #[tokio::test]
    async fn a_failed_mailto_send_is_not_recorded() {
        let db = db_with("<mailto:leave@shop.example>", None);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_sends(Some("smtp down"));
        let client = one_click_client().unwrap();

        assert!(unsubscribe(&db, &account(), "m1", &client, Some(&provider), 1)
            .await
            .is_err());
        assert_eq!(db.sender_unsubscribed_at("acc-1", "deals@shop.example").unwrap(), None);
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    const ONE_CLICK: Option<&str> = Some("List-Unsubscribe=One-Click");

    fn parse(header: &str, post: Option<&str>) -> Option<UnsubscribeMethod> {
        parse_unsubscribe(Some(header), post)
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn https_with_the_rfc8058_post_header_is_one_click() {
        assert_eq!(
            parse("<https://lists.example.com/u/abc>", ONE_CLICK),
            Some(UnsubscribeMethod::OneClick {
                url: url("https://lists.example.com/u/abc")
            })
        );
    }

    #[test]
    fn the_post_header_value_is_matched_case_insensitively_and_trimmed() {
        let m = parse("<https://lists.example.com/u>", Some("  list-unsubscribe=one-click "));
        assert!(matches!(m, Some(UnsubscribeMethod::OneClick { .. })));
    }

    #[test]
    fn a_post_header_with_another_value_is_not_one_click() {
        let m = parse("<https://lists.example.com/u>", Some("List-Unsubscribe=Yes"));
        assert!(matches!(m, Some(UnsubscribeMethod::Link { .. })), "{m:?}");
    }

    #[test]
    fn https_without_the_post_header_is_a_link() {
        assert_eq!(
            parse("<https://news.example.org/leave?id=7>", None),
            Some(UnsubscribeMethod::Link {
                url: url("https://news.example.org/leave?id=7")
            })
        );
    }

    #[test]
    fn mailto_beats_a_plain_link() {
        let m = parse(
            "<https://news.example.org/leave>, <mailto:leave@news.example.org>",
            None,
        );
        assert_eq!(
            m,
            Some(UnsubscribeMethod::Mailto {
                to: "leave@news.example.org".into(),
                subject: None,
                body: None
            })
        );
    }

    #[test]
    fn one_click_beats_mailto() {
        let m = parse(
            "<mailto:leave@news.example.org>, <https://news.example.org/oc>",
            ONE_CLICK,
        );
        assert!(matches!(m, Some(UnsubscribeMethod::OneClick { .. })), "{m:?}");
    }

    #[test]
    fn the_post_header_alone_without_an_https_uri_falls_back_to_mailto() {
        let m = parse("<mailto:leave@news.example.org>", ONE_CLICK);
        assert!(matches!(m, Some(UnsubscribeMethod::Mailto { .. })), "{m:?}");
    }

    #[test]
    fn mailto_subject_and_body_are_percent_decoded() {
        let m = parse(
            "<mailto:unsub@example.com?subject=Unsubscribe%20me&body=Please%20remove%0Athis%20address>",
            None,
        );
        assert_eq!(
            m,
            Some(UnsubscribeMethod::Mailto {
                to: "unsub@example.com".into(),
                subject: Some("Unsubscribe me".into()),
                body: Some("Please remove\nthis address".into()),
            })
        );
    }

    #[test]
    fn a_newline_in_the_mailto_subject_is_folded_into_a_space() {
        let m = parse("<mailto:unsub@example.com?subject=Hi%0D%0ABcc:%20x@evil.example>", None);
        let Some(UnsubscribeMethod::Mailto { subject, .. }) = m else {
            panic!("{m:?}")
        };
        let subject = subject.unwrap();
        assert!(!subject.contains('\n') && !subject.contains('\r'), "{subject:?}");
    }

    #[test]
    fn mailto_fields_other_than_subject_and_body_are_ignored() {
        let m = parse("<mailto:unsub@example.com?cc=boss@example.com&subject=x>", None);
        assert_eq!(
            m,
            Some(UnsubscribeMethod::Mailto {
                to: "unsub@example.com".into(),
                subject: Some("x".into()),
                body: None
            })
        );
    }

    #[test]
    fn unsafe_or_unsupported_schemes_are_rejected() {
        for header in [
            "<http://news.example.org/leave>",
            "<javascript:alert(1)>",
            "<file:///etc/passwd>",
            "<ftp://news.example.org/leave>",
            "<emailops://unsubscribe>",
            "<data:text/html,hi>",
        ] {
            assert_eq!(parse(header, ONE_CLICK), None, "{header}");
        }
    }

    #[test]
    fn an_http_entry_is_skipped_in_favour_of_a_later_https_one() {
        let m = parse("<http://a.example.com/u>, <https://b.example.com/u>", None);
        assert_eq!(
            m,
            Some(UnsubscribeMethod::Link {
                url: url("https://b.example.com/u")
            })
        );
    }

    #[test]
    fn uris_outside_angle_brackets_are_ignored() {
        assert_eq!(parse("https://news.example.org/leave", None), None);
        assert_eq!(parse("mailto:leave@example.org", None), None);
    }

    #[test]
    fn malformed_values_yield_nothing() {
        for header in [
            "",
            "   ",
            "<>",
            "<https://>",
            "<https://news.example.org/leave",
            "<mailto:>",
            "<mailto:not-an-address>",
            "<mailto:a@b.example, c@d.example>",
            "<mailto:a@b@c.example>",
            "<mailto:someone@localhost>",
            "<https://user:pw@news.example.org/leave>",
        ] {
            assert_eq!(parse(header, None), None, "{header:?}");
        }
        assert_eq!(parse_unsubscribe(None, ONE_CLICK), None);
    }

    #[test]
    fn overlong_values_are_rejected() {
        let long_uri = format!("<https://news.example.org/{}>", "a".repeat(MAX_URI_LEN));
        assert_eq!(parse(&long_uri, None), None);
        let long_header = format!("<mailto:a@b.example>{}", " ".repeat(MAX_HEADER_LEN));
        assert_eq!(parse(&long_header, None), None);
        let long_subject = format!("<mailto:a@b.example?subject={}>", "x".repeat(MAX_MAILTO_SUBJECT + 1));
        assert_eq!(parse(&long_subject, None), None);
        let long_body = format!("<mailto:a@b.example?body={}>", "x".repeat(MAX_MAILTO_BODY + 1));
        assert_eq!(parse(&long_body, None), None);
    }

    #[test]
    fn folded_whitespace_inside_an_https_uri_is_removed() {
        let m = parse("<https://news.example.org/\r\n leave?id=1>", None);
        assert_eq!(
            m,
            Some(UnsubscribeMethod::Link {
                url: url("https://news.example.org/leave?id=1")
            })
        );
    }

    #[test]
    fn the_webview_option_exposes_only_host_or_address() {
        let one_click = parse("<https://lists.example.com/u/secret-token>", ONE_CLICK).unwrap();
        assert_eq!(
            one_click.option(),
            UnsubscribeOption {
                kind: UnsubscribeKind::OneClick,
                target: "lists.example.com".into(),
                url: None
            }
        );
        let mailto = parse("<mailto:leave@example.org?subject=x>", None).unwrap();
        assert_eq!(
            mailto.option(),
            UnsubscribeOption {
                kind: UnsubscribeKind::Mailto,
                target: "leave@example.org".into(),
                url: None
            }
        );
        let link = parse("<https://news.example.org/leave>", None).unwrap();
        assert_eq!(
            link.option(),
            UnsubscribeOption {
                kind: UnsubscribeKind::Link,
                target: "news.example.org".into(),
                url: Some("https://news.example.org/leave".into())
            }
        );
    }
}
