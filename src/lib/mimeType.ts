const FALLBACK_MIME = 'application/octet-stream';
/** `type/subtype` with RFC 6838 restricted-name characters only. */
const MIME_SHAPE = /^[a-z0-9][a-z0-9!#$&^_.+-]*\/[a-z0-9][a-z0-9!#$&^_.+-]*$/;

/**
 * Normalize a sender-declared MIME type before it decides how an attachment is
 * rendered or is interpolated into a `data:` URL: lowercase, parameters
 * dropped, and anything not shaped like `type/subtype` (a comma, a quote,
 * markup, whitespace) replaced by an opaque type.
 */
export function normalizeMimeType(raw: string): string {
  const base = raw.split(';')[0].trim().toLowerCase();
  return MIME_SHAPE.test(base) ? base : FALLBACK_MIME;
}

const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';

/** A Word (.docx) file by its mime type, or by its name when the sender
 *  declared a generic type. Mirrors the backend's `DocumentKind::detect`. */
export function isDocxAttachment(mimeType: string, filename: string): boolean {
  return mimeType.toLowerCase() === DOCX_MIME || filename.toLowerCase().endsWith('.docx');
}
