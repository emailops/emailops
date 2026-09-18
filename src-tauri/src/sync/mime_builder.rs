//! Pure MIME builder for outgoing mail.
//!
//! Hand-rolled MIME is a maintenance trap (boundary leakage, missing CRLF,
//! quoted-printable rules, broken UTF-8 subject lines, etc.). We already
//! depend on `lettre` for SMTP/IMAP, so we route Gmail and IMAP through the
//! same `lettre::Message` builder and serialize once at the bottom.
//!
//! The MIME tree we produce, per case:
//!
//! - plain only, no attachments
//!   `text/plain`
//! - plain + attachments
//!   `multipart/mixed { text/plain, atts... }`
//! - text + html, no images, no attachments
//!   `multipart/alternative { text/plain, text/html }`
//! - text + html + inline images, no attachments
//!   `multipart/related { multipart/alternative { text, html }, inline... }`
//! - text + html + attachments, no inline images
//!   `multipart/mixed { multipart/alternative { text, html }, atts... }`
//! - text + html + inline images + attachments
//!   `multipart/mixed { multipart/related { alt, inline... }, atts... }`
//!
//! Inline images are referenced from the HTML body via `cid:<content_id>`
//! and the related part gives each image a matching `Content-ID:` header.
//!
//! Outlook does NOT use this — Microsoft Graph wants the message as JSON,
//! handled in `outlook_payload.rs`.

use base64::{engine::general_purpose::STANDARD, Engine};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::Message as LettreMessage;

use crate::models::error::{AppError, Result};
use crate::sync::provider::{EmailAttachment, EmailBody};

/// Inputs needed to assemble an outgoing message. `in_reply_to` carries the
/// original `Message-ID` for reply threading (e.g. `<abc@gmail.com>`); pass
/// `None` for fresh mail.
pub struct SendMimeParams<'a> {
    pub from_email: &'a str,
    /// Display name for the From header (`Name <address>`); `None` sends the
    /// bare address.
    pub from_name: Option<&'a str>,
    pub to_emails: &'a [String],
    pub cc_emails: &'a [String],
    pub subject: &'a str,
    pub in_reply_to: Option<&'a str>,
    /// The parent's own `References` header, so this reply can continue the
    /// chain instead of starting a new one. `None` for fresh mail, and for a
    /// parent that carried no chain (it was the thread root, or it was synced
    /// before we started storing the header).
    pub references: Option<&'a str>,
    pub body: &'a EmailBody,
    /// Regular file attachments (rendered as `Content-Disposition: attachment`).
    /// Inline images live in `body.inline_images`, never here.
    pub attachments: &'a [EmailAttachment],
}

/// Build the full MIME for an outgoing message and return it as a `String`.
///
/// IMAP send pushes the result through `lettre`'s SMTP transport directly via
/// [`build_lettre_message`]. Gmail wraps the bytes with base64url for the
/// `raw` field of `/users/me/messages/send`.
pub fn build_send_mime(params: &SendMimeParams<'_>) -> Result<String> {
    let msg = build_lettre_message(params)?;
    Ok(String::from_utf8_lossy(&msg.formatted()).into_owned())
}

/// Build a `lettre::Message`. Exposed for callers that want to push the
/// `Message` straight through an SMTP transport (IMAP path) instead of
/// serializing to bytes first (Gmail path).
pub fn build_lettre_message(params: &SendMimeParams<'_>) -> Result<LettreMessage> {
    let builder = base_builder(params)?;

    let body_text = format!("{}{}", params.body.text, params.body.footer_plain());

    let msg = match (params.body.html.as_deref(), params.attachments.is_empty()) {
        // Plain text, no attachments — lettre infers `text/plain; charset=utf-8`.
        (None, true) => builder
            .body(body_text)
            .map_err(|e| AppError::SyncError(format!("Failed to build text message: {e}")))?,

        // Plain text + attachments.
        (None, false) => {
            let mut mp =
                MultiPart::mixed().singlepart(SinglePart::builder().header(ContentType::TEXT_PLAIN).body(body_text));
            for att in params.attachments {
                mp = mp.singlepart(build_attachment_part(att)?);
            }
            builder
                .multipart(mp)
                .map_err(|e| AppError::SyncError(format!("Failed to build message: {e}")))?
        }

        // HTML + (optional inline images) + (optional attachments).
        (Some(html), _) => {
            let body_html = format!("{}{}", html, params.body.footer_html());
            let alternative = MultiPart::alternative_plain_html(body_text, body_html);

            let inline_images = &params.body.inline_images;
            let related_or_alt = if inline_images.is_empty() {
                alternative
            } else {
                let mut related = MultiPart::related().multipart(alternative);
                for img in inline_images {
                    related = related.singlepart(build_inline_image_part(img)?);
                }
                related
            };

            let top = if params.attachments.is_empty() {
                related_or_alt
            } else {
                let mut mixed = MultiPart::mixed().multipart(related_or_alt);
                for att in params.attachments {
                    mixed = mixed.singlepart(build_attachment_part(att)?);
                }
                mixed
            };

            builder
                .multipart(top)
                .map_err(|e| AppError::SyncError(format!("Failed to build message: {e}")))?
        }
    };

    Ok(msg)
}

/// Normalize a subject for a reply: prefix `Re: ` unless one (in any case)
/// is already present. Shared by the Gmail send path and the optimistic
/// local Sent row so the stored subject matches what goes on the wire.
pub fn reply_subject(subject: &str) -> String {
    if subject.to_ascii_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {}", subject)
    }
}

/// RFC 5322 `Message-ID` of a built message, angle brackets included
/// (e.g. `<abc.123@host>`). lettre generates one at build time when the
/// builder didn't set it explicitly, so this is `Some` for every message
/// produced by [`build_lettre_message`]. The send paths store it on the
/// optimistic local Sent row so the sync reconciler can exact-match the
/// provider's Sent copy later.
pub fn extract_message_id(msg: &LettreMessage) -> Option<String> {
    msg.headers().get_raw("Message-ID").map(|raw| {
        let trimmed = raw.trim();
        // lettre stores the raw header value without the angle brackets;
        // normalize to the bracketed wire form either way.
        if trimmed.starts_with('<') {
            trimmed.to_string()
        } else {
            format!("<{trimmed}>")
        }
    })
}

fn base_builder(params: &SendMimeParams<'_>) -> Result<lettre::message::MessageBuilder> {
    let address: lettre::Address = params
        .from_email
        .parse()
        .map_err(|e| AppError::SyncError(format!("Invalid from address: {e}")))?;
    let from = Mailbox::new(params.from_name.map(str::to_string), address);
    // Generate a client-side Message-ID (`<uuid@hostname>`) instead of leaving
    // it to the mail relay. Gmail and IMAP Sent copies preserve it, which lets
    // the sync reconciler exact-match the provider's Sent copy against the
    // optimistic local row inserted at send time.
    let mut builder = LettreMessage::builder()
        .from(from)
        .subject(params.subject)
        .message_id(None);
    for to in params.to_emails {
        builder = builder.to(to
            .parse()
            .map_err(|e| AppError::SyncError(format!("Invalid to address {to}: {e}")))?);
    }
    for cc in params.cc_emails {
        builder = builder.cc(cc
            .parse()
            .map_err(|e| AppError::SyncError(format!("Invalid cc address {cc}: {e}")))?);
    }
    if let Some(mid) = params.in_reply_to {
        builder = builder.in_reply_to(mid.to_string());
        builder = builder.references(reply_references(params.references, mid));
    }
    Ok(builder)
}

/// The `References` header for a reply: RFC 5322 §3.6.4 — the parent's own
/// `References` followed by the parent's `Message-ID`.
///
/// Sending only the parent's Message-ID (what this used to do) declares the
/// reply a new thread root. Worse, it is contagious: the recipient's client
/// builds its next reply's chain from ours, so the broken root propagates back
/// and one conversation fragments into pairs of messages.
fn reply_references(parent_references: Option<&str>, parent_message_id: &str) -> String {
    let parent_chain = parent_references.map(str::trim).unwrap_or_default();
    if parent_chain.is_empty() {
        return parent_message_id.to_string();
    }
    // Some clients already append their own Message-ID to References; don't
    // repeat it.
    if parent_chain.split_whitespace().last() == Some(parent_message_id) {
        return parent_chain.to_string();
    }
    format!("{parent_chain} {parent_message_id}")
}

fn build_attachment_part(att: &EmailAttachment) -> Result<SinglePart> {
    let bytes = decode_base64(&att.data)?;
    let ct: ContentType = att.mime_type.parse().unwrap_or_else(|_| {
        // "application/octet-stream" is a hard-coded, well-formed MIME literal.
        #[allow(clippy::unwrap_used)]
        let fallback = ContentType::parse("application/octet-stream").unwrap();
        fallback
    });
    Ok(Attachment::new(att.filename.clone()).body(bytes, ct))
}

fn build_inline_image_part(img: &EmailAttachment) -> Result<SinglePart> {
    let Some(cid) = img.content_id.as_deref().filter(|s| !s.is_empty()) else {
        return Err(AppError::InvalidInput("Inline image is missing contentId".to_string()));
    };
    let bytes = decode_base64(&img.data)?;
    let ct: ContentType = img.mime_type.parse().unwrap_or_else(|_| {
        // "application/octet-stream" is a hard-coded, well-formed MIME literal.
        #[allow(clippy::unwrap_used)]
        let fallback = ContentType::parse("application/octet-stream").unwrap();
        fallback
    });
    // Lettre's `Attachment::new_inline(cid)` produces a part with
    // `Content-Disposition: inline` and a `Content-ID: <cid>` header, which
    // is exactly what `<img src="cid:..."` in the HTML body resolves against.
    // Lettre 0.11 `Attachment::new_inline` sets both `Content-Disposition: inline`
    // and `Content-ID: <cid>`, which is what the HTML's `cid:` URI resolves
    // against per RFC 2392.
    Ok(Attachment::new_inline(cid.to_string()).body(bytes, ct))
}

/// Decode base64 input that may be standard or URL-safe, padded or not. We
/// accept both because the frontend pastes whatever the file/Clipboard API
/// produced, and Gmail's `raw` field uses URL-safe — keeping one decoder for
/// both directions avoids surprises.
fn decode_base64(data: &str) -> Result<Vec<u8>> {
    // Strip any whitespace introduced by line-wrapping (some sources emit
    // 76-char-wrapped base64).
    let cleaned: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    // Standard base64 first; fall back to URL-safe if that fails.
    match STANDARD.decode(&cleaned) {
        Ok(b) => Ok(b),
        Err(_) => base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cleaned.trim_end_matches('='))
            .map_err(|e| AppError::SyncError(format!("Base64 decode failed: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(body: &EmailBody, attachments: &[EmailAttachment]) -> String {
        let to = vec!["you@example.com".to_string()];
        let cc: Vec<String> = vec![];
        build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &cc,
            subject: "hello",
            in_reply_to: None,
            references: None,
            body,
            attachments,
        })
        .expect("build_send_mime")
    }

    fn from_line(from_name: Option<&str>) -> String {
        let to = vec!["you@example.com".to_string()];
        let mime = build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name,
            to_emails: &to,
            cc_emails: &[],
            subject: "hello",
            in_reply_to: None,
            references: None,
            body: &EmailBody::plain("hi"),
            attachments: &[],
        })
        .expect("build_send_mime");
        mime.lines()
            .find(|l| l.starts_with("From:"))
            .expect("From header")
            .to_string()
    }

    // Regression: the From header carried only the address, so recipients (and
    // the synced Sent copy) showed no sender name.
    #[test]
    fn from_header_carries_the_display_name() {
        assert_eq!(from_line(Some("Ada Example")), "From: \"Ada Example\" <me@example.com>");
    }

    #[test]
    fn from_header_encodes_a_non_ascii_display_name() {
        let line = from_line(Some("Adá Exámple"));
        assert!(
            line.starts_with("From: =?utf-8?"),
            "RFC 2047-encoded name expected, got: {line}"
        );
        assert!(
            line.ends_with("<me@example.com>"),
            "address must follow the name, got: {line}"
        );
    }

    #[test]
    fn from_header_without_a_display_name_is_the_bare_address() {
        assert_eq!(from_line(None), "From: me@example.com");
    }

    #[test]
    fn plain_text_only_has_no_multipart() {
        let mime = p(&EmailBody::plain("hi there"), &[]);
        assert!(mime.contains("Subject: hello"));
        assert!(mime.contains("From: me@example.com"));
        assert!(mime.contains("To: you@example.com"));
        // No multipart envelope.
        assert!(
            !mime.to_lowercase().contains("multipart/"),
            "plain-only message should not be multipart, got:\n{mime}"
        );
        assert!(mime.contains("hi there"));
        // Footer always appended (default English, brand "EmailOps").
        assert!(mime.contains("Sent with EmailOps"), "footer must be appended");
    }

    #[test]
    fn without_footer_suppresses_the_footer() {
        // Drafts push footer-free bodies so a push→pull→send round-trip does not
        // bake the "Sent with EmailOps" line in twice.
        let body = EmailBody::plain("draft body").without_footer();
        let mime = p(&body, &[]);
        assert!(mime.contains("draft body"));
        assert!(
            !mime.contains("Sent with EmailOps"),
            "footer must be suppressed for footer-free bodies, got:\n{mime}"
        );
    }

    #[test]
    fn footer_follows_body_language() {
        use crate::services::i18n::Language;
        let body = EmailBody::plain("hola").with_language(Language::Es);
        let mime = p(&body, &[]);
        assert!(
            mime.contains("Enviado con EmailOps"),
            "Spanish body must get the Spanish footer, got:\n{mime}"
        );
        assert!(
            !mime.contains("Sent with"),
            "must not fall back to English, got:\n{mime}"
        );
    }

    #[test]
    fn html_alternative_when_html_present() {
        let mime = p(&EmailBody::with_html("hi there", "<p>hi <b>there</b></p>"), &[]);
        let lower = mime.to_lowercase();
        assert!(
            lower.contains("multipart/alternative"),
            "expected multipart/alternative when html present, got:\n{mime}"
        );
        assert!(lower.contains("text/plain"));
        assert!(lower.contains("text/html"));
        // HTML body present (possibly QP-encoded but the tag names should survive).
        assert!(mime.contains("hi") && mime.contains("there"));
    }

    #[test]
    fn related_wraps_alternative_when_inline_images_present() {
        let mut body = EmailBody::with_html("see image", "<p>see <img src=\"cid:img1\"></p>");
        body.inline_images.push(EmailAttachment {
            filename: "pic.png".into(),
            mime_type: "image/png".into(),
            // 1×1 transparent PNG (8 bytes, not real but enough for the test).
            data: STANDARD.encode([0u8, 1, 2, 3, 4, 5, 6, 7]),
            content_id: Some("img1".into()),
            is_inline: true,
        });
        let mime = p(&body, &[]);
        let lower = mime.to_lowercase();
        assert!(
            lower.contains("multipart/related"),
            "inline images must produce multipart/related, got:\n{mime}"
        );
        assert!(lower.contains("multipart/alternative"));
        // The Content-ID for the inline image must appear so the HTML's
        // `cid:img1` reference resolves.
        assert!(mime.contains("img1"), "Content-ID must reference img1, got:\n{mime}");
        // The inline image is marked Content-Disposition: inline.
        assert!(
            lower.contains("content-disposition: inline"),
            "inline images must use Content-Disposition: inline, got:\n{mime}"
        );
    }

    #[test]
    fn attachments_produce_mixed_envelope() {
        let att = EmailAttachment {
            filename: "report.pdf".into(),
            mime_type: "application/pdf".into(),
            data: STANDARD.encode(b"%PDF-1.4 fake"),
            content_id: None,
            is_inline: false,
        };
        let mime = p(&EmailBody::plain("see attached"), &[att]);
        let lower = mime.to_lowercase();
        assert!(lower.contains("multipart/mixed"));
        assert!(lower.contains("application/pdf"));
        assert!(
            lower.contains("content-disposition: attachment"),
            "attachments must use Content-Disposition: attachment, got:\n{mime}"
        );
        assert!(mime.contains("report.pdf"));
    }

    #[test]
    fn html_with_attachment_yields_mixed_around_alternative() {
        let att = EmailAttachment {
            filename: "a.bin".into(),
            mime_type: "application/octet-stream".into(),
            data: STANDARD.encode(b"abc"),
            content_id: None,
            is_inline: false,
        };
        let mime = p(&EmailBody::with_html("plain", "<p>html</p>"), &[att]);
        let lower = mime.to_lowercase();
        assert!(lower.contains("multipart/mixed"));
        assert!(lower.contains("multipart/alternative"));
        assert!(lower.contains("application/octet-stream"));
    }

    #[test]
    fn html_with_inline_and_attachment_nests_related_inside_mixed() {
        let mut body = EmailBody::with_html("see attached", "<p><img src=\"cid:i\"></p>");
        body.inline_images.push(EmailAttachment {
            filename: "i.png".into(),
            mime_type: "image/png".into(),
            data: STANDARD.encode(b"PNGDATA"),
            content_id: Some("i".into()),
            is_inline: true,
        });
        let att = EmailAttachment {
            filename: "f.pdf".into(),
            mime_type: "application/pdf".into(),
            data: STANDARD.encode(b"PDF"),
            content_id: None,
            is_inline: false,
        };
        let mime = p(&body, &[att]);
        let lower = mime.to_lowercase();
        assert!(lower.contains("multipart/mixed"));
        assert!(lower.contains("multipart/related"));
        assert!(lower.contains("multipart/alternative"));
        assert!(lower.contains("application/pdf"));
    }

    #[test]
    fn in_reply_to_sets_threading_headers() {
        let to = vec!["you@example.com".to_string()];
        let mime = build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "Re: hi",
            in_reply_to: Some("<abc-123@gmail.com>"),
            references: None,
            body: &EmailBody::plain("yep"),
            attachments: &[],
        })
        .unwrap();
        assert!(
            mime.contains("In-Reply-To: <abc-123@gmail.com>"),
            "In-Reply-To header missing from:\n{mime}"
        );
        assert!(
            mime.contains("References: <abc-123@gmail.com>"),
            "References header missing from:\n{mime}"
        );
    }

    // ── References chain (RFC 5322 §3.6.4) ────────────────────────────────────

    #[test]
    fn a_reply_carries_the_parents_whole_reference_chain() {
        // Regression: References used to be set to the parent's Message-ID
        // alone, discarding the chain the parent carried. Every reply then
        // declared itself a new thread root — and because the recipient's
        // client faithfully copies our References, their next reply inherited
        // the wrong root too, so one conversation split into pairs.
        let to = vec!["you@example.com".to_string()];
        let mime = build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "Re: hi",
            in_reply_to: Some("<parent@example.com>"),
            references: Some("<root@example.com> <middle@example.com>"),
            body: &EmailBody::plain("yep"),
            attachments: &[],
        })
        .unwrap();

        assert!(
            mime.contains("References: <root@example.com> <middle@example.com> <parent@example.com>"),
            "References must be the parent's chain plus the parent's Message-ID, got:\n{mime}"
        );
        assert!(
            mime.contains("In-Reply-To: <parent@example.com>"),
            "In-Reply-To stays the immediate parent:\n{mime}"
        );
    }

    #[test]
    fn reply_references_falls_back_to_the_parent_when_it_had_no_chain() {
        assert_eq!(reply_references(None, "<parent@example.com>"), "<parent@example.com>");
        assert_eq!(
            reply_references(Some("   "), "<parent@example.com>"),
            "<parent@example.com>"
        );
    }

    #[test]
    fn reply_references_appends_the_parent_to_its_chain() {
        assert_eq!(
            reply_references(Some("<root@example.com>"), "<parent@example.com>"),
            "<root@example.com> <parent@example.com>"
        );
    }

    #[test]
    fn reply_references_does_not_repeat_a_parent_already_ending_the_chain() {
        // Some clients already include their own Message-ID in References.
        assert_eq!(
            reply_references(Some("<root@example.com> <parent@example.com>"), "<parent@example.com>"),
            "<root@example.com> <parent@example.com>"
        );
    }

    // What makes "no account can be created that the send path will reject" a
    // checked fact rather than a claim: the validator guarding account creation
    // and the parser building the From header must agree, in both directions.
    #[test]
    fn every_accepted_account_address_builds_a_valid_from_header() {
        use crate::util::email_addr::parse_account_address;

        let to = vec!["you@example.com".to_string()];
        let build = |from: &str| {
            build_send_mime(&SendMimeParams {
                from_email: from,
                from_name: None,
                to_emails: &to,
                cc_emails: &[],
                subject: "hello",
                in_reply_to: None,
                references: None,
                body: &EmailBody::plain("hi"),
                attachments: &[],
            })
        };

        for raw in [
            "alex@example.de",
            "alex.doe+tag@mail.example.co.uk",
            "Alex.Doe@Example.de",
        ] {
            let accepted = parse_account_address(raw).unwrap_or_else(|| panic!("validator must accept {raw}"));
            assert!(
                build(&accepted).is_ok(),
                "send path rejected an address account creation accepts: {raw}"
            );
        }

        // The bug itself, rejected at both ends: a bare login name is no address.
        assert!(parse_account_address("alex").is_none());
        assert!(build("alex").is_err());
    }

    #[test]
    fn invalid_from_address_returns_error() {
        let to = vec!["you@example.com".to_string()];
        let err = build_send_mime(&SendMimeParams {
            from_email: "not-an-email",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "x",
            in_reply_to: None,
            references: None,
            body: &EmailBody::plain("hi"),
            attachments: &[],
        });
        assert!(err.is_err(), "must reject malformed from address");
    }

    #[test]
    fn inline_image_without_content_id_returns_error() {
        let mut body = EmailBody::with_html("x", "<p>x</p>");
        body.inline_images.push(EmailAttachment {
            filename: "x.png".into(),
            mime_type: "image/png".into(),
            data: STANDARD.encode(b"x"),
            content_id: None, // ← missing
            is_inline: true,
        });
        let to = vec!["you@example.com".to_string()];
        let result = build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "x",
            in_reply_to: None,
            references: None,
            body: &body,
            attachments: &[],
        });
        assert!(
            matches!(result, Err(AppError::InvalidInput(_))),
            "inline image without contentId must error"
        );
    }

    #[test]
    fn non_ascii_subject_is_rfc2047_encoded() {
        // Regression: "número" used to be sent as raw UTF-8 bytes in the Subject
        // header and arrived garbled. lettre must wrap it in an RFC 2047
        // encoded-word so receiving MTAs decode it correctly.
        let to = vec!["you@example.com".to_string()];
        let mime = build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "Facturas T1 subidas y nuevo número de IVA",
            in_reply_to: None,
            references: None,
            body: &EmailBody::plain("hi"),
            attachments: &[],
        })
        .unwrap();
        // The raw non-ASCII bytes must NOT appear unwrapped on the Subject line.
        let subject_line = mime
            .lines()
            .find(|l| l.starts_with("Subject:"))
            .expect("Subject header present");
        assert!(
            !subject_line.contains("número"),
            "Subject must be RFC 2047 encoded, not raw UTF-8: {subject_line}"
        );
        assert!(
            subject_line.contains("=?") && subject_line.contains("?="),
            "expected RFC 2047 encoded-word in Subject, got: {subject_line}"
        );
    }

    #[test]
    fn ascii_subject_is_not_encoded() {
        let to = vec!["you@example.com".to_string()];
        let mime = build_send_mime(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "Hello World",
            in_reply_to: None,
            references: None,
            body: &EmailBody::plain("hi"),
            attachments: &[],
        })
        .unwrap();
        assert!(
            mime.contains("Subject: Hello World"),
            "ASCII subject must pass through unencoded, got:\n{mime}"
        );
    }

    #[test]
    fn extract_message_id_returns_the_generated_header() {
        let to = vec!["you@example.com".to_string()];
        let msg = build_lettre_message(&SendMimeParams {
            from_email: "me@example.com",
            from_name: None,
            to_emails: &to,
            cc_emails: &[],
            subject: "hello",
            in_reply_to: None,
            references: None,
            body: &EmailBody::plain("hi"),
            attachments: &[],
        })
        .unwrap();
        let mid = extract_message_id(&msg).expect("lettre generates a Message-ID at build time");
        assert!(
            mid.starts_with('<') && mid.ends_with('>') && mid.contains('@'),
            "expected an RFC 5322 msg-id, got: {mid}"
        );
        // The extracted value must match what actually goes on the wire.
        let mime = String::from_utf8_lossy(&msg.formatted()).into_owned();
        assert!(
            mime.contains(&format!("Message-ID: {mid}")),
            "extracted Message-ID must appear in the serialized MIME, got: {mid}\n{mime}"
        );
    }

    #[test]
    fn base64_decoder_accepts_standard_and_url_safe() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello"); // standard
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello"); // url-safe no pad
                                                                 // Whitespace in input must be tolerated.
        assert_eq!(decode_base64("aGVs\nbG8=").unwrap(), b"hello");
    }
}
