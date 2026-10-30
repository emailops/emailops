//! Plain text of a PDF, for the chat's open-document context.
//!
//! Pure: bytes in, text out. Fetching the bytes and putting the text in a
//! prompt live elsewhere (`services::chat::open_document`).

/// Why a PDF yielded no text the chat can use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfTextError {
    /// The bytes are not a PDF the parser can read (corrupt, encrypted with a
    /// password, an unsupported construct).
    #[error("the PDF could not be read: {0}")]
    Unreadable(String),
    /// The PDF parsed but carries no text layer — usually a scanned image,
    /// which would need OCR.
    #[error("the PDF has no extractable text")]
    NoText,
}

/// The text of `bytes`, whitespace tidied: NUL chars and trailing spaces
/// dropped and runs of blank lines folded into one, so layout padding does
/// not eat the prompt.
pub fn extract_pdf_text(bytes: &[u8]) -> Result<String, PdfTextError> {
    // The parser panics on some malformed files; the bytes are an attachment
    // someone sent, so a panic is an unreadable PDF, not a crash.
    let raw = std::panic::catch_unwind(|| {
        pdf_oxide::PdfDocument::from_bytes(bytes.to_vec()).and_then(|doc| doc.extract_all_text())
    })
    .map_err(|_| PdfTextError::Unreadable("the parser gave up on this file".to_string()))?
    .map_err(|e| PdfTextError::Unreadable(e.to_string()))?;
    // Pages are separated by form feeds. A glyph without any Unicode mapping
    // can come out as NUL, and the chat runtime cannot take a prompt that
    // holds one.
    let text = tidy_whitespace(&raw.replace('\x0c', "\n\n").replace('\0', ""));
    if text.is_empty() {
        return Err(PdfTextError::NoText);
    }
    Ok(text)
}

fn tidy_whitespace(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut blank_run = 0;
    for line in raw.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank_run += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank_run > 0 { "\n\n" } else { "\n" });
        }
        out.push_str(line);
        blank_run = 0;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-page PDF whose page shows `lines` in Helvetica, one per line.
    /// Built by hand (offsets included) so the tests need no fixture file.
    pub(crate) fn pdf_with_lines(lines: &[&str]) -> Vec<u8> {
        let mut content = String::from("BT /F1 12 Tf 72 720 Td 14 TL\n");
        for line in lines {
            let escaped = line.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
            content.push_str(&format!("({escaped}) Tj T*\n"));
        }
        content.push_str("ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
/Resources << /Font << /F1 5 0 R >> >> >>"
                .to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for off in offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn extracts_the_text_of_each_line() {
        let pdf = pdf_with_lines(&["Invoice 2041", "Total due: 1,250.00 EUR"]);
        let text = extract_pdf_text(&pdf).unwrap();
        assert!(text.contains("Invoice 2041"), "{text:?}");
        assert!(text.contains("Total due: 1,250.00 EUR"), "{text:?}");
    }

    #[test]
    fn nul_bytes_in_the_text_layer_are_dropped() {
        // A glyph the parser cannot map comes out as NUL, which the chat
        // runtime refuses (the prompt becomes a C string).
        let text = extract_pdf_text(&pdf_with_lines(&["Glucose\0 92 mg/dL"])).unwrap();
        assert!(!text.contains('\0'), "{text:?}");
        assert!(text.contains("Glucose 92 mg/dL"), "{text:?}");
    }

    #[test]
    fn a_glyph_missing_from_to_unicode_is_read_from_the_embedded_font() {
        // Chrome-printed reports embed a TrueType subset whose ToUnicode map
        // can lack a glyph; here the one for "í". The font itself still says
        // which character the glyph draws.
        let pdf = include_bytes!("../../tests/fixtures/pdf/missing_tounicode_glyph.pdf");
        let text = extract_pdf_text(pdf).unwrap();
        assert!(text.contains("Hematíes 4.71"), "{text:?}");
    }

    #[test]
    fn a_pdf_without_text_is_no_text() {
        assert_eq!(extract_pdf_text(&pdf_with_lines(&[])), Err(PdfTextError::NoText));
    }

    #[test]
    fn bytes_that_are_not_a_pdf_are_unreadable() {
        assert!(matches!(
            extract_pdf_text(b"definitely not a pdf"),
            Err(PdfTextError::Unreadable(_))
        ));
    }

    #[test]
    fn tidy_whitespace_folds_blank_runs_and_trailing_spaces() {
        assert_eq!(tidy_whitespace("\n\n a  \n\n\n\nb\nc   \n\n"), " a\n\nb\nc");
    }
}
