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

/// Largest image a signature may carry, decoded. A logo or a scanned
/// handwritten signature fits; the editor downscales on upload to stay under.
pub const MAX_SIGNATURE_IMAGE_BYTES: usize = 200 * 1024;

/// Widest image a signature may carry, in pixels (the editor downscales an
/// upload to 600 px; this leaves room for a pasted high-DPI logo).
pub const MAX_SIGNATURE_IMAGE_WIDTH: u32 = 1200;

/// Image types a signature may embed. Not SVG: it is a document that can
/// carry script and external references, and many clients refuse it.
const SIGNATURE_IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Why an embedded (`data:`) image cannot go into a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureImageProblem {
    /// Not PNG, JPEG, GIF or WebP (SVG, HTML, PDF…).
    UnsupportedType(String),
    /// Not base64, or not valid base64.
    BadEncoding,
    TooLarge {
        bytes: usize,
    },
    TooWide {
        width: u32,
    },
    /// The bytes are not the image the URL claims.
    Unreadable,
}

impl std::fmt::Display for SignatureImageProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(mime) => write!(
                f,
                "The signature contains an unsupported image type ({mime}). Use PNG, JPEG, GIF or WebP."
            ),
            Self::BadEncoding => write!(f, "The signature contains an image that is not correctly encoded."),
            Self::TooLarge { bytes } => write!(
                f,
                "A signature image is too large ({} KB, at most {} KB). Use a smaller image.",
                bytes / 1024,
                MAX_SIGNATURE_IMAGE_BYTES / 1024
            ),
            Self::TooWide { width } => write!(
                f,
                "A signature image is too wide ({width} px, at most {MAX_SIGNATURE_IMAGE_WIDTH} px)."
            ),
            Self::Unreadable => write!(f, "The signature contains an image that could not be read."),
        }
    }
}

/// Pure: whether one `src`/`href` value may stay in a signature. Only `data:`
/// URLs are inspected — remote and `cid:` images are not embedded bytes.
pub fn check_signature_image(url: &str) -> std::result::Result<(), SignatureImageProblem> {
    use base64::Engine as _;
    let Some(rest) = url
        .get(..5)
        .filter(|p| p.eq_ignore_ascii_case("data:"))
        .map(|_| &url[5..])
    else {
        return Ok(());
    };
    let (meta, payload) = rest.split_once(',').ok_or(SignatureImageProblem::BadEncoding)?;
    let mut parts = meta.split(';');
    let mime = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    if !SIGNATURE_IMAGE_TYPES.contains(&mime.as_str()) {
        return Err(SignatureImageProblem::UnsupportedType(mime));
    }
    if !parts.any(|p| p.trim().eq_ignore_ascii_case("base64")) {
        return Err(SignatureImageProblem::BadEncoding);
    }
    if !payload
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
    {
        return Err(SignatureImageProblem::BadEncoding);
    }
    // Size first, from the length: a huge payload is refused before decoding.
    let approx = payload.len() / 4 * 3;
    if approx > MAX_SIGNATURE_IMAGE_BYTES + 3 {
        return Err(SignatureImageProblem::TooLarge { bytes: approx });
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|_| SignatureImageProblem::BadEncoding)?;
    if bytes.len() > MAX_SIGNATURE_IMAGE_BYTES {
        return Err(SignatureImageProblem::TooLarge { bytes: bytes.len() });
    }
    let width = image_width(&mime, &bytes).ok_or(SignatureImageProblem::Unreadable)?;
    if width > MAX_SIGNATURE_IMAGE_WIDTH {
        return Err(SignatureImageProblem::TooWide { width });
    }
    Ok(())
}

/// Pure: the pixel width from an image's header, when the bytes really are
/// the declared type.
fn image_width(mime: &str, b: &[u8]) -> Option<u32> {
    let be16 = |i: usize| b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as u32);
    let le16 = |i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as u32);
    match mime {
        "image/png" => {
            if !b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) || b.get(12..16) != Some(b"IHDR") {
                return None;
            }
            b.get(16..20).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        }
        "image/gif" => {
            if !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
                return None;
            }
            le16(6)
        }
        "image/webp" => {
            if !b.starts_with(b"RIFF") || b.get(8..12) != Some(b"WEBP") {
                return None;
            }
            match b.get(12..16)? {
                b"VP8X" => b.get(24..27).map(|s| u32::from_le_bytes([s[0], s[1], s[2], 0]) + 1),
                b"VP8 " => le16(26).map(|w| w & 0x3FFF),
                b"VP8L" => b
                    .get(21..23)
                    .map(|s| (u32::from_le_bytes([s[0], s[1], 0, 0]) & 0x3FFF) + 1),
                _ => None,
            }
        }
        "image/jpeg" => {
            if !b.starts_with(&[0xFF, 0xD8]) {
                return None;
            }
            // Walk the segments to the frame header (SOF0–SOF15 except the
            // DHT/JPG/DAC markers), which holds the size.
            let mut i = 2;
            while i + 4 <= b.len() {
                if b[i] != 0xFF {
                    return None;
                }
                let marker = b[i + 1];
                let len = be16(i + 2)? as usize;
                if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                    return be16(i + 7);
                }
                i += 2 + len;
            }
            None
        }
        _ => None,
    }
}

/// Every URL-valued attribute of the HTML (`src`, `href`), read with the
/// sanitizer's own parser rather than a pattern.
fn url_attributes(html: &str) -> Vec<String> {
    let found = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&found);
    ammonia::Builder::default()
        .add_generic_attributes(["src", "href"])
        .add_url_schemes(["data", "cid"])
        .attribute_filter(move |_element, attribute, value| {
            if attribute == "src" || attribute == "href" {
                if let Ok(mut urls) = sink.lock() {
                    urls.push(value.to_string());
                }
            }
            Some(std::borrow::Cow::Borrowed(value))
        })
        .clean(html);
    // The builder (and its clone of the Arc) is gone once `clean` returned.
    std::sync::Arc::try_unwrap(found)
        .ok()
        .and_then(|urls| urls.into_inner().ok())
        .unwrap_or_default()
}

/// Pure: the first image problem of a (sanitized) signature, if any.
pub fn signature_image_problem(html: &str) -> Option<SignatureImageProblem> {
    url_attributes(html)
        .iter()
        .find_map(|url| check_signature_image(url).err())
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
    if let Some(problem) = signature_image_problem(&html) {
        return Err(AppError::InvalidInput(problem.to_string()));
    }
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

    // ---- Signature images -------------------------------------------------

    use base64::Engine as _;

    /// The first bytes of a PNG: signature + IHDR with the given size. Enough
    /// for the header checks; padded to `len` bytes.
    fn png(width: u32, height: u32, len: usize) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b.resize(len.max(b.len()), 0);
        b
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&[0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&width.to_le_bytes());
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(&[0, 0, 0]);
        b
    }

    fn webp_vp8x(width: u32, height: u32) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        b.extend_from_slice(&[10, 0, 0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        b.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        b
    }

    fn data_url(mime: &str, bytes: &[u8]) -> String {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    fn img(src: &str) -> String {
        format!(r#"<p>Ana</p><p><img src="{src}" alt="logo"></p>"#)
    }

    #[test]
    fn a_valid_png_logo_is_saved() {
        let db = db_with_account("acc1");
        let src = data_url("image/png", &png(300, 80, 2_000));
        let saved = save_signature(&db, "acc1", input(&img(&src)), NOW).unwrap();
        assert!(saved.html.contains(&src), "{}", saved.html);
    }

    #[test]
    fn jpeg_gif_and_webp_logos_are_accepted() {
        for src in [
            data_url("image/jpeg", &jpeg(400, 100)),
            data_url("image/gif", &gif(120, 40)),
            data_url("image/webp", &webp_vp8x(500, 120)),
        ] {
            assert_eq!(check_signature_image(&src), Ok(()), "{src}");
        }
    }

    #[test]
    fn an_svg_image_is_refused() {
        let db = db_with_account("acc1");
        let svg = data_url(
            "image/svg+xml",
            b"<svg xmlns='http://www.w3.org/2000/svg'><script>x()</script></svg>",
        );
        let err = save_signature(&db, "acc1", input(&img(&svg)), NOW).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        assert!(get_signature(&db, "acc1").unwrap().updated_at.is_none());
        assert!(matches!(
            check_signature_image(&svg),
            Err(SignatureImageProblem::UnsupportedType(_))
        ));
    }

    #[test]
    fn a_non_image_data_url_is_refused() {
        for src in [
            data_url("text/html", b"<b>x</b>"),
            data_url("application/pdf", b"%PDF-1.4"),
        ] {
            assert!(
                matches!(
                    check_signature_image(&src),
                    Err(SignatureImageProblem::UnsupportedType(_))
                ),
                "{src}"
            );
        }
        let db = db_with_account("acc1");
        let html = format!(r#"<p><a href="{}">x</a></p>"#, data_url("text/html", b"<b>x</b>"));
        assert!(matches!(
            save_signature(&db, "acc1", input(&html), NOW),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn an_image_over_the_size_cap_is_refused() {
        let db = db_with_account("acc1");
        let src = data_url("image/png", &png(300, 80, MAX_SIGNATURE_IMAGE_BYTES + 1));
        let err = save_signature(&db, "acc1", input(&img(&src)), NOW).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
        assert_eq!(
            check_signature_image(&src),
            Err(SignatureImageProblem::TooLarge {
                bytes: MAX_SIGNATURE_IMAGE_BYTES + 1
            })
        );
    }

    #[test]
    fn an_image_over_the_width_cap_is_refused() {
        let wide = MAX_SIGNATURE_IMAGE_WIDTH + 1;
        assert_eq!(
            check_signature_image(&data_url("image/png", &png(wide, 10, 100))),
            Err(SignatureImageProblem::TooWide { width: wide })
        );
        assert_eq!(
            check_signature_image(&data_url("image/jpeg", &jpeg(wide as u16, 10))),
            Err(SignatureImageProblem::TooWide { width: wide })
        );
    }

    #[test]
    fn bad_base64_or_mismatched_bytes_are_refused() {
        assert_eq!(
            check_signature_image("data:image/png;base64,AA$A"),
            Err(SignatureImageProblem::BadEncoding)
        );
        assert_eq!(
            check_signature_image("data:image/png,rawbytes"),
            Err(SignatureImageProblem::BadEncoding)
        );
        // Says PNG, is a GIF.
        assert_eq!(
            check_signature_image(&data_url("image/png", &gif(10, 10))),
            Err(SignatureImageProblem::Unreadable)
        );
    }

    #[test]
    fn remote_and_cid_images_are_not_data_urls_and_pass() {
        assert_eq!(check_signature_image("https://example.com/logo.png"), Ok(()));
        assert_eq!(check_signature_image("cid:logo"), Ok(()));
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
