//! Pure planner for Microsoft Graph send payloads.
//!
//! Outlook does NOT take MIME bytes — Graph wants a `Message` JSON object
//! (https://learn.microsoft.com/en-us/graph/api/resources/message). We
//! construct the JSON here as a pure function so it's covered by unit tests
//! independently of the HTTP layer.
//!
//! Inline images use the Graph-specific `isInline: true` + `contentId` fields
//! on a `#microsoft.graph.fileAttachment`. Outlook resolves
//! `<img src="cid:foo">` in the HTML body against the attachment whose
//! `contentId == "foo"`.

use base64::{
    alphabet,
    engine::{self, GeneralPurpose, GeneralPurposeConfig},
    Engine,
};
use serde_json::{json, Value};

use crate::sync::provider::{email_footer_html, email_footer_plain, EmailAttachment, EmailBody};

/// Inputs needed to assemble a Graph send payload.
pub struct OutlookSendParams<'a> {
    pub to_emails: &'a [String],
    pub cc_emails: &'a [String],
    pub subject: &'a str,
    pub body: &'a EmailBody,
    pub attachments: &'a [EmailAttachment],
}

/// Build the JSON body for `POST /me/sendMail`.
pub fn build_send_mail_payload(params: &OutlookSendParams<'_>) -> Value {
    let message = build_message_object(params);
    json!({
        "message": message,
        "saveToSentItems": true,
    })
}

/// Build the JSON body for `POST /me/messages/{id}/reply`.
///
/// The `/reply` endpoint accepts an optional `message` override containing
/// body / recipients / attachments. When HTML or inline images are present we
/// use the `message` form; otherwise we use the simpler `comment` form which
/// preserves Outlook's quoted-history rendering.
pub fn build_reply_payload(params: &OutlookSendParams<'_>) -> Value {
    if params.body.has_html() || !params.body.inline_images.is_empty() {
        json!({
            "message": build_message_object(params),
        })
    } else {
        json!({
            "comment": format!("{}{}", params.body.text, email_footer_plain()),
            "message": {
                "toRecipients": build_recipients(params.to_emails),
                "ccRecipients": build_recipients(params.cc_emails),
            }
        })
    }
}

fn build_message_object(params: &OutlookSendParams<'_>) -> Value {
    let mut message = json!({
        "subject": params.subject,
        "body": build_body_object(params.body),
        "toRecipients": build_recipients(params.to_emails),
        "ccRecipients": build_recipients(params.cc_emails),
    });

    let mut atts: Vec<Value> = Vec::new();
    // Inline images first — keeps the JSON readable for debugging, no semantic effect.
    for img in &params.body.inline_images {
        atts.push(build_attachment_json(img, true));
    }
    for att in params.attachments {
        atts.push(build_attachment_json(att, false));
    }
    if !atts.is_empty() {
        message["attachments"] = Value::Array(atts);
    }
    message
}

fn build_body_object(body: &EmailBody) -> Value {
    if let Some(html) = &body.html {
        json!({
            "contentType": "HTML",
            "content": format!("{}{}", html, email_footer_html()),
        })
    } else {
        json!({
            "contentType": "Text",
            "content": format!("{}{}", body.text, email_footer_plain()),
        })
    }
}

fn build_attachment_json(att: &EmailAttachment, force_inline: bool) -> Value {
    // Graph expects standard base64 (not URL-safe). Re-encode defensively in
    // case the frontend handed us URL-safe data.
    let clean: String = att.data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let standardized = to_standard_base64(&clean);
    let is_inline = force_inline || att.is_inline;
    let mut obj = json!({
        "@odata.type": "#microsoft.graph.fileAttachment",
        "name": att.filename,
        "contentType": att.mime_type,
        "contentBytes": standardized,
        "isInline": is_inline,
    });
    if let Some(cid) = att.content_id.as_deref().filter(|s| !s.is_empty()) {
        obj["contentId"] = Value::String(cid.to_string());
    }
    obj
}

fn build_recipients(addresses: &[String]) -> Vec<Value> {
    addresses
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let bare = extract_bare_address(s);
            json!({ "emailAddress": { "address": bare } })
        })
        .collect()
}

fn extract_bare_address(s: &str) -> String {
    if let Some(start) = s.find('<') {
        if let Some(end) = s[start..].find('>') {
            return s[start + 1..start + end].trim().to_string();
        }
    }
    s.trim().to_string()
}

fn to_standard_base64(data: &str) -> String {
    if data.contains('-') || data.contains('_') {
        const LENIENT: GeneralPurpose = GeneralPurpose::new(
            &alphabet::URL_SAFE,
            GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
        );
        match LENIENT.decode(data) {
            Ok(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
            Err(_) => data.to_string(),
        }
    } else {
        data.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn body_plain(text: &str) -> EmailBody {
        EmailBody::plain(text)
    }

    fn body_html(text: &str, html: &str) -> EmailBody {
        EmailBody::with_html(text, html)
    }

    fn params<'a>(body: &'a EmailBody, attachments: &'a [EmailAttachment], to: &'a [String]) -> OutlookSendParams<'a> {
        OutlookSendParams {
            to_emails: to,
            cc_emails: &[],
            subject: "hello",
            body,
            attachments,
        }
    }

    #[test]
    fn plain_text_send_payload_uses_text_content_type() {
        let to = vec!["a@b.com".to_string()];
        let payload = build_send_mail_payload(&params(&body_plain("hi"), &[], &to));
        assert_eq!(payload["saveToSentItems"], true);
        assert_eq!(payload["message"]["body"]["contentType"], "Text");
        assert!(payload["message"]["body"]["content"]
            .as_str()
            .unwrap()
            .starts_with("hi"));
        assert!(payload["message"]["attachments"].is_null());
    }

    #[test]
    fn html_body_uses_html_content_type_and_appends_html_footer() {
        let to = vec!["a@b.com".to_string()];
        let payload = build_send_mail_payload(&params(&body_html("hi", "<p>hi</p>"), &[], &to));
        assert_eq!(payload["message"]["body"]["contentType"], "HTML");
        let content = payload["message"]["body"]["content"].as_str().unwrap();
        assert!(content.contains("<p>hi</p>"));
        // HTML footer (not plain) is appended.
        assert!(
            content.contains("<a href"),
            "html footer must use anchor, got {content}"
        );
    }

    #[test]
    fn inline_image_becomes_file_attachment_with_is_inline_and_content_id() {
        let to = vec!["a@b.com".to_string()];
        let mut body = body_html("see image", "<p><img src=\"cid:logo\"></p>");
        body.inline_images.push(EmailAttachment {
            filename: "logo.png".into(),
            mime_type: "image/png".into(),
            data: STANDARD.encode(b"PNG"),
            content_id: Some("logo".into()),
            is_inline: true,
        });
        let payload = build_send_mail_payload(&params(&body, &[], &to));
        let atts = payload["message"]["attachments"].as_array().unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["@odata.type"], "#microsoft.graph.fileAttachment");
        assert_eq!(atts[0]["isInline"], true);
        assert_eq!(atts[0]["contentId"], "logo");
        assert_eq!(atts[0]["contentType"], "image/png");
    }

    #[test]
    fn regular_attachment_is_not_inline_and_has_no_content_id() {
        let to = vec!["a@b.com".to_string()];
        let att = EmailAttachment {
            filename: "report.pdf".into(),
            mime_type: "application/pdf".into(),
            data: STANDARD.encode(b"PDF"),
            content_id: None,
            is_inline: false,
        };
        let payload = build_send_mail_payload(&params(&body_plain("see attached"), &[att], &to));
        let atts = payload["message"]["attachments"].as_array().unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["isInline"], false);
        assert!(atts[0]["contentId"].is_null());
    }

    #[test]
    fn recipients_strip_display_name_wrapper() {
        let to = vec!["Alice <a@b.com>".to_string(), "Bob <bob@b.com>".to_string()];
        let payload = build_send_mail_payload(&params(&body_plain("hi"), &[], &to));
        let recipients = payload["message"]["toRecipients"].as_array().unwrap();
        assert_eq!(recipients[0]["emailAddress"]["address"], "a@b.com");
        assert_eq!(recipients[1]["emailAddress"]["address"], "bob@b.com");
    }

    #[test]
    fn url_safe_base64_is_re_encoded_to_standard() {
        let to = vec!["a@b.com".to_string()];
        let att = EmailAttachment {
            filename: "x.bin".into(),
            mime_type: "application/octet-stream".into(),
            data: "-_A=".into(), // url-safe `+/A=`
            content_id: None,
            is_inline: false,
        };
        let payload = build_send_mail_payload(&params(&body_plain("hi"), &[att], &to));
        let content_bytes = payload["message"]["attachments"][0]["contentBytes"].as_str().unwrap();
        assert_eq!(content_bytes, "+/A=", "URL-safe must be re-encoded for Graph");
    }

    #[test]
    fn reply_payload_plain_uses_comment_form() {
        let to = vec!["a@b.com".to_string()];
        let payload = build_reply_payload(&params(&body_plain("yep"), &[], &to));
        assert!(
            payload["comment"].as_str().unwrap().starts_with("yep"),
            "plain reply must use the `comment` form to keep Outlook's quoted history"
        );
    }

    #[test]
    fn reply_payload_html_uses_message_form() {
        let to = vec!["a@b.com".to_string()];
        let payload = build_reply_payload(&params(&body_html("yep", "<p>yep</p>"), &[], &to));
        // No `comment` — replaced by full message body so HTML is honored.
        assert!(payload["comment"].is_null());
        assert_eq!(payload["message"]["body"]["contentType"], "HTML");
    }

    #[test]
    fn empty_recipients_are_filtered_out() {
        let to = vec!["a@b.com".to_string(), "".to_string(), "   ".to_string()];
        let payload = build_send_mail_payload(&params(&body_plain("hi"), &[], &to));
        let recipients = payload["message"]["toRecipients"].as_array().unwrap();
        assert_eq!(recipients.len(), 1);
    }
}
