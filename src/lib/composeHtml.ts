/**
 * Pure helpers for converting Tiptap (or any contenteditable) HTML output
 * into the `body` / `bodyHtml` / `inlineImages` triple that the backend
 * `send_reply` / `send_new_email` commands expect.
 *
 * Why a separate module: the Tiptap editor itself is hard to test (it needs
 * a real DOM plus a ProseMirror schema); but the bit that matters for
 * correctness — pulling `data:image/...;base64,...` blobs out of the HTML
 * and replacing them with `cid:...` references — is just string + DOM
 * walking and is exhaustively unit-testable.
 *
 * Contract:
 *   prepareOutgoingHtml(htmlFromEditor) → {
 *     bodyHtml: string,          // same HTML but with data: images replaced by cid:
 *     plainText: string,         // best-effort text fallback (Tiptap stores it cleanly)
 *     inlineImages: EmailAttachment[],
 *   }
 *
 * The backend then sanitizes `bodyHtml` again via ammonia. Defense in depth.
 */

import type { EmailAttachment } from '@/lib/api';

export interface PreparedOutgoing {
  bodyHtml: string;
  plainText: string;
  inlineImages: EmailAttachment[];
}

/** Match `data:image/<subtype>;base64,<payload>` (optionally `;charset=...`). */
const DATA_IMAGE_RE = /^data:(image\/[a-zA-Z0-9.+-]+)(?:;[^,]*)?;base64,([A-Za-z0-9+/=_-]+)$/;

/**
 * Convert the editor's HTML output into the shape the backend expects.
 * Each data-URL image becomes a separate inline attachment with a
 * deterministic-ish content ID, and the `<img>` `src` is rewritten to
 * `cid:<contentId>`.
 *
 * `cidPrefix` is exposed for tests so we can assert stable output. In prod
 * we default to a short random suffix.
 */
export function prepareOutgoingHtml(html: string, cidPrefix?: string): PreparedOutgoing {
  const prefix = cidPrefix ?? `img-${Math.random().toString(36).slice(2, 8)}`;
  // jsdom + browsers both provide DOMParser.
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const inlineImages: EmailAttachment[] = [];
  let counter = 0;

  for (const img of Array.from(doc.querySelectorAll('img'))) {
    const src = img.getAttribute('src') ?? '';
    const match = DATA_IMAGE_RE.exec(src);
    if (!match) continue; // remote URLs stay as-is; ammonia will keep https.
    const mimeType = match[1];
    const data = match[2];
    counter += 1;
    const contentId = `${prefix}-${counter}`;
    const extension = mimeTypeToExtension(mimeType);
    inlineImages.push({
      filename: `inline-${counter}.${extension}`,
      mimeType,
      data,
      contentId,
      isInline: true,
    });
    img.setAttribute('src', `cid:${contentId}`);
  }

  const bodyEl = doc.body;
  const bodyHtml = bodyEl ? bodyEl.innerHTML : html;
  const plainText = htmlToPlainText(bodyHtml);
  return { bodyHtml, plainText, inlineImages };
}

/**
 * Best-effort plaintext fallback for receivers that won't render HTML.
 * Block-level tags become line breaks; `<br>` becomes a single newline;
 * images become `[image]` placeholders; everything else is text content.
 *
 * This is intentionally simple — receivers that can render HTML (which is
 * ~all of them in 2026) use the HTML part. The text/plain part is just
 * not-completely-broken fallback.
 */
export function htmlToPlainText(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const lines: string[] = [];
  let current = '';

  const flush = () => {
    lines.push(current);
    current = '';
  };

  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      current += node.textContent ?? '';
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node as Element;
    const tag = el.tagName.toLowerCase();
    if (tag === 'br') {
      flush();
      return;
    }
    if (tag === 'img') {
      const alt = el.getAttribute('alt');
      current += alt ? `[image: ${alt}]` : '[image]';
      return;
    }
    if (tag === 'a') {
      const href = el.getAttribute('href') ?? '';
      const label = el.textContent ?? '';
      // If label and href differ, show "label (href)" so the receiver can see the URL.
      if (href && href !== label) {
        current += `${label} (${href})`;
      } else {
        current += label;
      }
      return;
    }
    const blockTags = new Set(['p', 'div', 'li', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'pre', 'hr']);
    const isBlock = blockTags.has(tag);
    if (isBlock && current.length > 0) flush();
    for (const child of Array.from(el.childNodes)) walk(child);
    if (isBlock) flush();
  };

  if (doc.body) {
    for (const child of Array.from(doc.body.childNodes)) walk(child);
  }
  if (current.length > 0) flush();

  // Collapse runs of >2 blank lines down to a single blank line, and trim.
  return lines
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function mimeTypeToExtension(mime: string): string {
  switch (mime.toLowerCase()) {
    case 'image/png':
      return 'png';
    case 'image/jpeg':
    case 'image/jpg':
      return 'jpg';
    case 'image/gif':
      return 'gif';
    case 'image/webp':
      return 'webp';
    case 'image/svg+xml':
      return 'svg';
    case 'image/bmp':
      return 'bmp';
    default:
      return 'bin';
  }
}

/**
 * Wrap a plain-text body (e.g. an AI draft, or a user typing in the legacy
 * textarea) in minimal HTML so Tiptap can render it without losing line
 * breaks.
 */
export function plainTextToHtml(text: string): string {
  // Split on blank lines into paragraphs; within a paragraph, convert single
  // newlines to <br>. Escape HTML special chars first.
  const escaped = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const paragraphs = escaped.split(/\n{2,}/);
  return paragraphs.map((p) => `<p>${p.replace(/\n/g, '<br>')}</p>`).join('');
}
