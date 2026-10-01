/**
 * Per-account email signatures for the three compose surfaces (ComposeModal,
 * ComposeTabView, ReplyCompose) and the AI reply draft.
 *
 * A signature is plain text plus an optional image (a logo, a handwritten
 * signature) shown at a chosen width. Text is stored in `signature:<id>`, the
 * image in `signature_image:<id>` as `{"src": "data:image/…", "width": 160}`.
 *
 * It is inserted into the editor below the conventional `--` delimiter
 * paragraph. The delimiter is also how the signature is found again — Tiptap
 * normalizes the HTML on every keystroke (wrapper `<div>`s and `data-*`
 * attributes do not survive), so the delimiter paragraph is the only marker
 * that is stable across edits. That is what lets the composers:
 *   - swap the signature when the user changes the From account,
 *   - keep it after an AI draft replaces the body,
 *   - ignore it when deciding whether a composer is empty (autosave, send),
 *   - leave it out of the brief sent to the AI draft generator.
 *
 * Everything here is pure (string in → string out); see `signature.test.ts`.
 */
import { htmlToPlainText } from './composeHtml';

/** Preference key holding the signature text of one account. */
export function signaturePrefKey(accountId: string): string {
  return `signature:${accountId}`;
}

/** Preference key holding the signature image of one account (JSON). */
export function signatureImagePrefKey(accountId: string): string {
  return `signature_image:${accountId}`;
}

/** Longest signature text accepted, in characters. Mirrored by the backend check. */
export const SIGNATURE_MAX_LENGTH = 4000;

/** Largest signature image accepted, as a data URL. Mirrored by the backend check. */
export const SIGNATURE_IMAGE_MAX_BYTES = 1_000_000;
/** Allowed signature image display widths, in CSS pixels. */
export const SIGNATURE_IMAGE_MIN_WIDTH = 40;
export const SIGNATURE_IMAGE_MAX_WIDTH = 600;
export const SIGNATURE_IMAGE_DEFAULT_WIDTH = 160;

const IMAGE_DATA_URL = /^data:image\/(png|jpe?g|gif|webp);base64,[A-Za-z0-9+/=]+$/;

export interface SignatureImage {
  /** `data:image/<png|jpeg|gif|webp>;base64,…` */
  src: string;
  /** Display width in pixels; height follows the aspect ratio. */
  width: number;
}

/** A complete signature: its text and optional image. */
export interface Signature {
  text: string;
  image: SignatureImage | null;
}

export const NO_SIGNATURE: Signature = { text: '', image: null };

export function clampSignatureWidth(width: number): number {
  if (!Number.isFinite(width)) return SIGNATURE_IMAGE_DEFAULT_WIDTH;
  return Math.round(Math.min(SIGNATURE_IMAGE_MAX_WIDTH, Math.max(SIGNATURE_IMAGE_MIN_WIDTH, width)));
}

/** Whether `src` is an image data URL the signature accepts. */
export function isSignatureImageSrc(src: string): boolean {
  return src.length <= SIGNATURE_IMAGE_MAX_BYTES && IMAGE_DATA_URL.test(src);
}

/** Serialize an image for its preference ('' removes it). */
export function serializeSignatureImage(image: SignatureImage | null): string {
  if (!image) return '';
  return JSON.stringify({ src: image.src, width: clampSignatureWidth(image.width) });
}

/** Parse a stored image preference; anything malformed reads as "no image". */
export function parseSignatureImage(raw: string | null | undefined): SignatureImage | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as { src?: unknown; width?: unknown };
    if (typeof v.src !== 'string' || !isSignatureImageSrc(v.src)) return null;
    return { src: v.src, width: clampSignatureWidth(typeof v.width === 'number' ? v.width : Number.NaN) };
  } catch {
    return null;
  }
}

function isEmptySignature(sig: Signature): boolean {
  return !sig.text.trim() && !sig.image;
}

/** Accepts the legacy plain-string form too (text only). */
function normalize(sig: Signature | string): Signature {
  return typeof sig === 'string' ? { text: sig, image: null } : sig;
}

/**
 * The delimiter paragraph, as Tiptap serializes it (`-- ` loses its trailing
 * space). `&nbsp;`/non-breaking space variants are accepted when reading.
 */
const DELIMITER_HTML = '<p>--</p>';
const DELIMITER = /<p>--(?:\s|&nbsp;|\u00a0)*<\/p>/;
/** Where quoted history starts in a reply or forward. */
const QUOTE_START =
  /<blockquote|<p>-{5,}|<p>(?:-{2,}\s*)?(?:forwarded message|mensaje reenviado|message transf|weitergeleitete)/i;

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/**
 * Editor HTML for a signature: the `--` delimiter, one paragraph per text
 * line (blank lines kept), then the image at its width. Empty string when
 * there is no signature.
 */
export function signatureToHtml(signature: Signature | string): string {
  const sig = normalize(signature);
  if (isEmptySignature(sig)) return '';
  const text = sig.text.replace(/\r\n/g, '\n').trim();
  const lines = text ? text.split('\n').map((line) => (line.trim() ? `<p>${escapeHtml(line)}</p>` : '<p></p>')) : [];
  const image =
    sig.image && isSignatureImageSrc(sig.image.src)
      ? `<img src="${sig.image.src}" alt="Signature" width="${clampSignatureWidth(sig.image.width)}">`
      : '';
  return `${DELIMITER_HTML}${lines.join('')}${image}`;
}

/**
 * Locate the user's signature: the first delimiter paragraph *before* any
 * quoted history, up to that history (reply/forward) or the end of the body.
 * A `--` inside the quoted original is someone else's signature and is never
 * matched. `null` when absent.
 */
function findSignature(html: string): { start: number; end: number } | null {
  const q = html.search(QUOTE_START);
  const own = q >= 0 ? html.slice(0, q) : html;
  const m = DELIMITER.exec(own);
  if (!m) return null;
  return { start: m.index, end: own.length };
}

/** Whether `html` already contains a signature. */
export function hasSignature(html: string): boolean {
  return findSignature(html) !== null;
}

/** `html` with its signature (if any) removed; quoted history is kept. */
export function stripSignature(html: string): string {
  const s = findSignature(html);
  return s ? html.slice(0, s.start) + html.slice(s.end) : html;
}

/**
 * Put `signature` into `html`, replacing any signature already there.
 *
 * - A new message gets an empty paragraph to type in, then the signature.
 * - Text followed by quoted history (reply/forward): the signature goes
 *   between the two, the way Gmail and Outlook place it on a top-posted reply.
 * - An empty signature removes the existing one.
 */
export function applySignature(html: string, signature: Signature | string): string {
  const block = signatureToHtml(signature);
  const existing = findSignature(html);
  if (existing) {
    return html.slice(0, existing.start) + block + html.slice(existing.end);
  }
  if (!block) return html;
  const base = html.trim();
  if (!base || base === '<p></p>') {
    return `<p></p>${block}`;
  }
  const q = base.search(QUOTE_START);
  if (q === 0) return `<p></p>${block}${base}`;
  if (q > 0) return `${base.slice(0, q)}${block}${base.slice(q)}`;
  return `${base}${block}`;
}

/**
 * The text the user actually wrote: the body with the signature removed.
 * Used for "is this composer empty?" (autosave, send button) and as the brief
 * for the AI draft generator, so neither treats the signature as content.
 */
export function bodyWithoutSignature(html: string): string {
  return htmlToPlainText(stripSignature(html)).trim();
}
