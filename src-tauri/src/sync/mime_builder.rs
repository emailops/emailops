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
    pub to_emails: &'a [String],
    pub cc_emails: &'a [String],
    pub subject: &'a str,
    pub in_reply_to: Option<&'a str>,
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

fn base_builder(params: &SendMimeParams<'_>) -> Result<lettre::message::MessageBuilder> {
    let from: Mailbox = params
        .from_email
        .parse()
        .map_err(|e| AppError::SyncError(format!("Invalid from address: {e}")))?;
    let mut builder = LettreMessage::builder().from(from).subject(params.subject);
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
        builder = builder.references(mid.to_string());
    }
    Ok(builder)
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
            to_emails: &to,
            cc_emails: &cc,
            subject: "hello",
            in_reply_to: None,
            body,
            attachments,
        })
        .expect("build_send_mime")
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
            to_emails: &to,
            cc_emails: &[],
            subject: "Re: hi",
            in_reply_to: Some("<abc-123@gmail.com>"),
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

    #[test]
    fn invalid_from_address_returns_error() {
        let to = vec!["you@example.com".to_string()];
        let err = build_send_mime(&SendMimeParams {
            from_email: "not-an-email",
            to_emails: &to,
            cc_emails: &[],
            subject: "x",
            in_reply_to: None,
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
            to_emails: &to,
            cc_emails: &[],
            subject: "x",
            in_reply_to: None,
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
            to_emails: &to,
            cc_emails: &[],
            subject: "Facturas T1 subidas y nuevo número de IVA",
            in_reply_to: None,
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
            to_emails: &to,
            cc_emails: &[],
            subject: "Hello World",
            in_reply_to: None,
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
    fn base64_decoder_accepts_standard_and_url_safe() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello"); // standard
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello"); // url-safe no pad
                                                                 // Whitespace in input must be tolerated.
        assert_eq!(decode_base64("aGVs\nbG8=").unwrap(), b"hello");
    }
}
