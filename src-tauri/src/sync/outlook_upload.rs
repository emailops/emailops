//! Pure planning for Outlook attachments that do not fit in one request.
//!
//! Microsoft Graph takes a file attachment inline (base64 in the JSON) only
//! below 3 MB, and a whole request only up to about 4 MB. From 3 MB up to
//! 150 MB an attachment has to go through an upload session on a message that
//! already exists as a draft: `createUploadSession`, then `PUT`s of byte
//! ranges to the pre-authenticated `uploadUrl`
//! (<https://learn.microsoft.com/graph/outlook-large-attachments>). This
//! module decides which way each attachment goes and provides the byte-range
//! arithmetic; `outlook.rs` does the requests.

use base64::{
    alphabet,
    engine::{self, GeneralPurpose, GeneralPurposeConfig},
    Engine,
};
use serde_json::{json, Value};

use crate::models::error::{AppError, Result};
use crate::sync::provider::EmailAttachment;

/// Below this an attachment may travel inline; from it on Graph requires an
/// upload session (and refuses a session for anything smaller).
pub const INLINE_ATTACHMENT_LIMIT: u64 = 3 * 1024 * 1024;

/// The largest attachment Graph accepts through an upload session.
pub const MAX_ATTACHMENT_SIZE: u64 = 150 * 1024 * 1024;

/// Bytes per `PUT`. Graph asks for ranges under 4 MB; this is nine 320 KiB
/// blocks, and a multiple of 3 so that a range starts on a base64 quantum.
pub const UPLOAD_CHUNK_SIZE: u64 = 9 * 320 * 1024;

// Checked at compile time: Graph asks for ranges under 4 MB, and a range has to
// start on a base64 quantum for `EncodedContent::range` to decode it alone.
const _: () = assert!(UPLOAD_CHUNK_SIZE < 4_000_000 && UPLOAD_CHUNK_SIZE.is_multiple_of(3));

/// How one attachment reaches Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentRoute {
    /// Base64 in a JSON request.
    Inline,
    /// An upload session on the draft.
    UploadSession,
}

/// How a message's attachments reach Graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentPlan {
    /// One route per attachment, in the order given.
    pub routes: Vec<AttachmentRoute>,
    /// Everything fits in the message's own request (`sendMail`, `reply`, the
    /// draft create/update). Otherwise the message is created as a draft
    /// without attachments and each one is added to it on its own.
    pub single_request: bool,
}

/// Pure: decide the route of each attachment, given `(name, size in bytes)`.
///
/// An attachment over Graph's maximum is refused here, before anything is
/// sent. The message's own request is used only while every attachment is
/// under the inline limit *and* they stay under it together: base64 adds a
/// third, which puts the JSON at Graph's 4 MB request limit.
pub fn plan_attachments(files: &[(&str, u64)]) -> Result<AttachmentPlan> {
    if let Some((name, _)) = files.iter().find(|(_, size)| *size > MAX_ATTACHMENT_SIZE) {
        return Err(AppError::InvalidInput(format!(
            "\"{name}\" is larger than the {} MB Outlook accepts for one attachment",
            MAX_ATTACHMENT_SIZE / (1024 * 1024)
        )));
    }
    let routes: Vec<AttachmentRoute> = files
        .iter()
        .map(|(_, size)| {
            if *size < INLINE_ATTACHMENT_LIMIT {
                AttachmentRoute::Inline
            } else {
                AttachmentRoute::UploadSession
            }
        })
        .collect();
    let total: u64 = files.iter().map(|(_, size)| size).sum();
    Ok(AttachmentPlan {
        routes,
        single_request: total < INLINE_ATTACHMENT_LIMIT,
    })
}

/// The size in bytes of an attachment's content, from its base64 text alone.
pub fn decoded_len(data: &str) -> u64 {
    let symbols = data.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=').count() as u64;
    symbols * 3 / 4
}

/// The `Content-Range` of bytes `start..end` of `total`.
pub fn content_range(start: u64, end: u64, total: u64) -> String {
    format!("bytes {}-{}/{}", start, end.saturating_sub(1), total)
}

/// Where Graph wants the upload to continue, from `nextExpectedRanges`
/// (`"2097152"`, `"2097152-"` or `"26-99"`): the start of the first range.
pub fn next_offset(next_expected_ranges: &[String]) -> Option<u64> {
    next_expected_ranges.first()?.split('-').next()?.trim().parse().ok()
}

/// An attachment's base64 text, prepared for reading byte ranges without
/// decoding the whole content at once: a 150 MB file is already held once as
/// text, and a decoded copy next to it would double that.
pub struct EncodedContent<'a> {
    /// The text without line breaks, borrowed when it had none.
    text: std::borrow::Cow<'a, str>,
    len: u64,
}

impl<'a> EncodedContent<'a> {
    pub fn new(data: &'a str) -> Self {
        let text = if data.bytes().any(|b| b.is_ascii_whitespace()) {
            std::borrow::Cow::Owned(data.chars().filter(|c| !c.is_ascii_whitespace()).collect())
        } else {
            std::borrow::Cow::Borrowed(data)
        };
        let len = decoded_len(&text);
        Self { text, len }
    }

    /// The size of the content in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bytes `start..end` of the content, decoded from only the part of the
    /// text that holds them: every 3 bytes are 4 symbols.
    pub fn range(&self, start: u64, end: u64) -> Result<Vec<u8>> {
        if start > end || end > self.len {
            return Err(AppError::InvalidInput(format!(
                "byte range {start}..{end} is outside an attachment of {} bytes",
                self.len
            )));
        }
        const STANDARD: GeneralPurpose = GeneralPurpose::new(
            &alphabet::STANDARD,
            GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
        );
        const URL_SAFE: GeneralPurpose = GeneralPurpose::new(
            &alphabet::URL_SAFE,
            GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
        );
        let first_symbol = (start / 3 * 4) as usize;
        let last_symbol = (end.div_ceil(3) * 4).min(self.text.len() as u64) as usize;
        let symbols = self
            .text
            .get(first_symbol..last_symbol)
            .ok_or_else(|| AppError::InvalidInput("attachment content is not valid base64".to_string()))?;
        let decoded = STANDARD
            .decode(symbols)
            .or_else(|_| URL_SAFE.decode(symbols))
            .map_err(|e| AppError::InvalidInput(format!("attachment content is not valid base64: {e}")))?;
        let skip = (start % 3) as usize;
        let wanted = (end - start) as usize;
        decoded.get(skip..skip + wanted).map(<[u8]>::to_vec).ok_or_else(|| {
            AppError::InvalidInput("attachment content is shorter than its base64 text says".to_string())
        })
    }
}

/// The body of `createUploadSession` for an attachment of `size` bytes.
/// `force_inline` marks the body's inline images, as the JSON payload does.
pub fn upload_session_payload(attachment: &EmailAttachment, size: u64, force_inline: bool) -> Value {
    let mut item = json!({
        "attachmentType": "file",
        "name": attachment.filename,
        "size": size,
        "contentType": attachment.mime_type,
        "isInline": force_inline || attachment.is_inline,
    });
    if let Some(cid) = attachment.content_id.as_deref().filter(|s| !s.is_empty()) {
        item["contentId"] = Value::String(cid.to_string());
    }
    json!({ "AttachmentItem": item })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};

    const MIB: u64 = 1024 * 1024;

    fn plan(files: &[(&str, u64)]) -> AttachmentPlan {
        plan_attachments(files).expect("plan")
    }

    #[test]
    fn attachments_are_routed_by_size() {
        use AttachmentRoute::{Inline, UploadSession};
        let cases: Vec<(&str, Vec<u64>, Vec<AttachmentRoute>, bool)> = vec![
            ("no attachments", vec![], vec![], true),
            ("one small file", vec![10_000], vec![Inline], true),
            ("just under the inline limit", vec![3 * MIB - 1], vec![Inline], true),
            ("at the inline limit", vec![3 * MIB], vec![UploadSession], false),
            ("a large file", vec![20 * MIB], vec![UploadSession], false),
            ("at the maximum", vec![150 * MIB], vec![UploadSession], false),
            (
                "small files that fit together",
                vec![MIB, MIB],
                vec![Inline, Inline],
                true,
            ),
            (
                "small files that together fill the request",
                vec![2 * MIB, MIB],
                vec![Inline, Inline],
                false,
            ),
            (
                "a small file next to a large one",
                vec![10_000, 5 * MIB],
                vec![Inline, UploadSession],
                false,
            ),
            ("an empty file", vec![0], vec![Inline], true),
        ];
        for (label, sizes, routes, single_request) in cases {
            let files: Vec<(&str, u64)> = sizes.iter().map(|size| ("file.bin", *size)).collect();
            assert_eq!(plan(&files), AttachmentPlan { routes, single_request }, "{label}");
        }
    }

    #[test]
    fn an_attachment_over_the_maximum_is_refused_by_name() {
        let err = plan_attachments(&[("small.pdf", 10), ("video.mov", 150 * MIB + 1)]).expect_err("refused");
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        let message = err.to_string();
        assert!(message.contains("video.mov"), "{message}");
        assert!(message.contains("150 MB"), "{message}");
    }

    #[test]
    fn the_content_size_is_read_off_the_base64_text() {
        for len in [0usize, 1, 2, 3, 4, 5, 1_000, 1_001, 1_002] {
            let bytes = vec![7u8; len];
            assert_eq!(decoded_len(&STANDARD.encode(&bytes)), len as u64, "padded {len}");
            assert_eq!(
                decoded_len(&STANDARD_NO_PAD.encode(&bytes)),
                len as u64,
                "unpadded {len}"
            );
        }
        assert_eq!(
            decoded_len("AAAA\r\nAAAA\r\nAA==\r\n"),
            7,
            "line breaks are not content"
        );
    }

    #[test]
    fn content_range_names_the_inclusive_last_byte() {
        assert_eq!(content_range(0, 2_097_152, 3_483_322), "bytes 0-2097151/3483322");
        assert_eq!(
            content_range(2_097_152, 3_483_322, 3_483_322),
            "bytes 2097152-3483321/3483322"
        );
    }

    #[test]
    fn the_next_offset_is_the_start_of_the_first_expected_range() {
        let ranges = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(next_offset(&ranges(&["2097152"])), Some(2_097_152));
        assert_eq!(next_offset(&ranges(&["2097152-"])), Some(2_097_152));
        assert_eq!(next_offset(&ranges(&["26-99", "200-"])), Some(26));
        assert_eq!(next_offset(&ranges(&["0-"])), Some(0));
        assert_eq!(next_offset(&[]), None);
        assert_eq!(next_offset(&ranges(&["soon"])), None);
    }

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn any_byte_range_decodes_from_its_own_part_of_the_text() {
        let bytes = sample(1_000);
        let ranges = [
            (0, 0),
            (0, 1),
            (0, 3),
            (0, 1_000),
            (3, 9),
            (1, 2),
            (5, 11),
            (998, 1_000),
            (999, 1_000),
        ];
        for encoded in [
            STANDARD.encode(&bytes),
            STANDARD_NO_PAD.encode(&bytes),
            URL_SAFE_NO_PAD.encode(&bytes),
        ] {
            for (start, end) in ranges {
                assert_eq!(
                    EncodedContent::new(&encoded).range(start, end).expect("decode"),
                    bytes[start as usize..end as usize],
                    "{start}..{end}"
                );
            }
        }
    }

    #[test]
    fn consecutive_chunks_rebuild_the_content() {
        // One byte more than two chunks, so the last range is a single byte.
        let bytes = sample(2 * UPLOAD_CHUNK_SIZE as usize + 1);
        let encoded = STANDARD.encode(&bytes);
        let content = EncodedContent::new(&encoded);
        let total = content.len();
        let mut rebuilt = Vec::new();
        let mut start = 0;
        while start < total {
            let end = (start + UPLOAD_CHUNK_SIZE).min(total);
            rebuilt.extend(content.range(start, end).expect("decode"));
            start = end;
        }
        assert_eq!(rebuilt, bytes);
    }

    #[test]
    fn text_with_line_breaks_still_decodes_by_range() {
        let bytes = sample(100);
        let wrapped: String = STANDARD
            .encode(&bytes)
            .as_bytes()
            .chunks(10)
            .map(|line| format!("{}\r\n", String::from_utf8_lossy(line)))
            .collect();
        let content = EncodedContent::new(&wrapped);
        assert_eq!(content.len(), 100);
        assert_eq!(content.range(10, 61).expect("decode"), bytes[10..61]);
    }

    #[test]
    fn a_range_that_is_not_base64_or_out_of_bounds_is_an_error() {
        assert!(EncodedContent::new("!!!!").range(0, 3).is_err());
        let encoded = STANDARD.encode([1u8, 2, 3]);
        assert!(EncodedContent::new(&encoded).range(0, 4).is_err());
        assert!(EncodedContent::new(&encoded).range(2, 1).is_err());
    }

    fn attachment(name: &str, content_id: Option<&str>, is_inline: bool) -> EmailAttachment {
        EmailAttachment {
            filename: name.to_string(),
            mime_type: "application/pdf".to_string(),
            data: String::new(),
            content_id: content_id.map(str::to_string),
            is_inline,
        }
    }

    #[test]
    fn the_upload_session_describes_the_file() {
        assert_eq!(
            upload_session_payload(&attachment("report.pdf", None, false), 3_483_322, false),
            json!({
                "AttachmentItem": {
                    "attachmentType": "file",
                    "name": "report.pdf",
                    "size": 3_483_322,
                    "contentType": "application/pdf",
                    "isInline": false,
                }
            })
        );
    }

    #[test]
    fn the_upload_session_of_an_inline_image_carries_its_content_id() {
        let payload = upload_session_payload(&attachment("photo.png", Some("img-1"), false), 4_000_000, true);
        assert_eq!(payload["AttachmentItem"]["isInline"], true);
        assert_eq!(payload["AttachmentItem"]["contentId"], "img-1");
        let payload = upload_session_payload(&attachment("photo.png", Some(""), true), 4_000_000, false);
        assert_eq!(payload["AttachmentItem"]["isInline"], true);
        assert!(payload["AttachmentItem"].get("contentId").is_none());
    }
}
