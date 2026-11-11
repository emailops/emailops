import DOMPurify from 'dompurify';
import type { Email } from '@/types';

/** Name to show for a message's sender. A bare-address From header (no
 *  display name) syncs with an empty `sender`, so fall back to the address. */
export function senderName(email: Pick<Email, 'sender' | 'senderEmail'>): string {
  return email.sender || email.senderEmail;
}

function sameAddress(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

/**
 * Whether the user sent this message, judged against the address of the
 * account the row belongs to (`ownerEmail`), never the active account: the
 * "All accounts" view mixes rows from several.
 *
 * In Inbox and Spam only the provider's sent flag counts. A From header equal
 * to the user's address is trivially forged, and a spoofed message must keep
 * showing its sender rather than read "To: you".
 */
export function isOwnMessage(
  email: Pick<Email, 'isSent' | 'mailbox' | 'senderEmail'>,
  ownerEmail: string | undefined,
): boolean {
  if (email.mailbox === 'inbox' || email.mailbox === 'spam') return email.isSent;
  if (email.isSent || email.mailbox === 'sent') return true;
  return !!ownerEmail?.trim() && sameAddress(email.senderEmail, ownerEmail);
}

/**
 * One `recipients`/`cc` entry as stored: `Name <address>` or a bare address
 * (Gmail keeps the header text; Outlook and IMAP keep the address only). The
 * address is the last `<…>`, as in the Gmail sync, since a display name may
 * itself hold angle brackets. Returns null for anything without an `@`:
 * empty entries and `undisclosed-recipients:;` groups.
 */
export function parseAddressEntry(entry: string): { name: string; address: string } | null {
  let name = '';
  let address = entry.trim();
  const start = entry.lastIndexOf('<');
  const end = start === -1 ? -1 : entry.indexOf('>', start + 1);
  if (end !== -1) {
    address = entry.slice(start + 1, end).trim();
    name = entry
      .slice(0, start)
      .trim()
      .replace(/^"(.*)"$/, '$1')
      .replace(/\\(.)/g, '$1')
      .trim();
  }
  if (!address.includes('@')) return null;
  // A name that is itself an address says nothing the address does not.
  if (name.includes('@')) name = '';
  return { name, address };
}

/** Where a recipient's displayed name came from: the user themself, the
 *  stored header, the name that address uses when it writes to the account,
 *  or nothing (the address is shown). */
export type PartySource = 'self' | 'header' | 'contacts' | 'address';

export interface PartyEntry {
  address: string;
  /** Empty for `self` and `address`: the caller words those. */
  name: string;
  source: PartySource;
}

/** Who a list row names: its sender (`from`, today's row), or the recipients
 *  of a message the user sent (`to`). */
export type RowParty =
  | { mode: 'from' }
  | {
      mode: 'to';
      /** The first `maxNames` entries, To then Cc, the user last. */
      shown: PartyEntry[];
      /** How many entries did not fit in `shown`. */
      more: number;
      to: PartyEntry[];
      cc: PartyEntry[];
      /** The entry the avatar, drag image and menu stand for: the first
       *  recipient who is not the user, else the user, else none. */
      primary: PartyEntry | null;
    };

/**
 * Decide what a list row names. Pure: no i18n, no stores — `EmailRow` words
 * the result. `nameOf` receives a lowercased address and returns the name it
 * uses when it writes to the row's account, if any.
 *
 * `inSentView`: the row is listed by the Sent view, whose query already
 * counts it as sent (sent flag, Sent folder or sender = account). That covers
 * an Inbox copy of the user's own reply-all, which carries no sent flag and
 * would otherwise show the user's own name in Sent.
 */
export function rowParty(
  email: Email,
  {
    ownerEmail,
    nameOf,
    maxNames,
    inSentView = false,
  }: {
    ownerEmail: string | undefined;
    nameOf: (address: string) => string | undefined;
    maxNames: number;
    inSentView?: boolean;
  },
): RowParty {
  if (!inSentView && !isOwnMessage(email, ownerEmail)) return { mode: 'from' };

  const seen = new Set<string>();
  const toEntries = (list: string[]): PartyEntry[] =>
    list.flatMap((raw): PartyEntry[] => {
      const parsed = parseAddressEntry(raw);
      if (!parsed) return [];
      const key = parsed.address.toLowerCase();
      if (seen.has(key)) return [];
      seen.add(key);
      if (ownerEmail && sameAddress(parsed.address, ownerEmail)) {
        return [{ address: parsed.address, name: '', source: 'self' }];
      }
      if (parsed.name) return [{ address: parsed.address, name: parsed.name, source: 'header' }];
      const known = nameOf(key);
      if (known) return [{ address: parsed.address, name: known, source: 'contacts' }];
      return [{ address: parsed.address, name: '', source: 'address' }];
    });

  const to = toEntries(email.recipients);
  const cc = toEntries(email.cc);
  const all = [...to, ...cc];
  const ordered = [...all.filter((e) => e.source !== 'self'), ...all.filter((e) => e.source === 'self')];
  const shown = ordered.slice(0, maxNames);
  return { mode: 'to', shown, more: ordered.length - shown.length, to, cc, primary: ordered[0] ?? null };
}

// Inline CSS properties we explicitly refuse to honour. The rest of the CSS
// surface is allowed because email HTML is rendered inside a sandboxed
// `<iframe srcdoc>` (null origin, no allow-same-origin) — see
// `src/components/shared/EmailHtmlFrame.tsx`. That isolation prevents email
// CSS from cascading into the app, which was the original motivation for the
// previous tight allowlist. Behaviour (.htc-loading) is the one historical
// CSS-as-script vector that the sandbox does not fully neutralise.
const DISALLOWED_CSS_PROPS = new Set(['behavior', '-ms-behavior']);

// Only raster image MIME types are allowed inside CSS `url(data:…)`.
// SVG is *not* on this list because `data:image/svg+xml` can carry inline
// `<script>` / event handlers that the WebView will execute when the image
// is loaded as a CSS background.
const SAFE_CSS_DATA_URI_RE = /^data:image\/(?:png|jpe?g|gif|webp);/;

// Matches every `url(...)` token in a CSS value. Captures the inner argument
// with surrounding quotes (if any) preserved so the validator can strip them.
const CSS_URL_TOKEN_RE = /url\(\s*([^)]*?)\s*\)/g;

function cssValueHasOnlySafeUrls(val: string, allowRemote: boolean): boolean {
  CSS_URL_TOKEN_RE.lastIndex = 0;
  let match = CSS_URL_TOKEN_RE.exec(val);
  while (match !== null) {
    const inner = match[1]
      .replace(/^['"]|['"]$/g, '')
      .trim()
      .toLowerCase();
    const isHttp = inner.startsWith('http://') || inner.startsWith('https://');
    const isSafeData = SAFE_CSS_DATA_URI_RE.test(inner);
    if (!(isSafeData || (allowRemote && isHttp))) return false;
    match = CSS_URL_TOKEN_RE.exec(val);
  }
  return true;
}

// Split a CSS declaration list on `;`, but only at depth 0 with respect to
// parentheses. Without this, a value like `url(data:image/png;base64,abc)`
// would be cut in half on its inner semicolon and the resulting fragments
// would each evade the url() validator.
function splitCssDeclarations(raw: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < raw.length; i++) {
    const c = raw.charCodeAt(i);
    if (c === 40 /* ( */) depth++;
    else if (c === 41 /* ) */ && depth > 0) depth--;
    else if (c === 59 /* ; */ && depth === 0) {
      out.push(raw.slice(start, i));
      start = i + 1;
    }
  }
  if (start < raw.length) out.push(raw.slice(start));
  return out;
}

export function sanitizeCssValue(raw: string, allowRemote = false): string {
  return splitCssDeclarations(raw)
    .map((decl) => decl.trim())
    .filter((decl) => {
      if (!decl) return false;
      const colon = decl.indexOf(':');
      if (colon < 0) return false;
      const prop = decl.slice(0, colon).trim().toLowerCase();
      if (DISALLOWED_CSS_PROPS.has(prop)) return false;
      const val = decl
        .slice(colon + 1)
        .trim()
        .toLowerCase();
      // Block expression()/javascript: regardless of property.
      if (val.includes('expression(')) return false;
      if (val.includes('javascript:')) return false;
      // url() tokens: always require either a raster data: URI or an
      // http(s) URL when remote content is enabled. SVG data: URIs and
      // other schemes are rejected even with allowRemote=true.
      if (val.includes('url(') && !cssValueHasOnlySafeUrls(val, allowRemote)) return false;
      return true;
    })
    .join('; ');
}

// `<style>` is intentionally allowed: it is required by modern responsive
// email templates (ticket cut-outs, hero-image sizing, mobile media queries).
// The sandboxed iframe wrapper prevents the email's CSS from leaking into the
// app. All script-bearing or navigation-bearing tags remain forbidden.
const DOMPURIFY_CONFIG: Parameters<typeof DOMPurify.sanitize>[1] = {
  USE_PROFILES: { html: true },
  FORBID_TAGS: ['script', 'iframe', 'object', 'embed', 'form', 'input', 'button', 'link', 'meta', 'base'],
  ADD_TAGS: ['style'],
  // Email HTML is typically a body fragment. Without FORCE_BODY, top-level
  // `<style>` tags get parsed into `<head>` and then dropped along with the
  // rest of the head. Forcing body context keeps them alongside the markup
  // they're meant to style.
  FORCE_BODY: true,
  // DOMPurify's default FORBID_CONTENTS includes `style`, which strips the
  // CSS text inside any surviving `<style>` element. Override the default
  // (preserving the rest of the safety set) so style contents reach the
  // iframe intact. Tags themselves listed here are still removed via
  // FORBID_TAGS above where it matters (script, iframe, etc.).
  FORBID_CONTENTS: [
    'annotation-xml',
    'audio',
    'colgroup',
    'desc',
    'foreignobject',
    'head',
    'iframe',
    'math',
    'mi',
    'mn',
    'mo',
    'ms',
    'mtext',
    'noembed',
    'noframes',
    'plaintext',
    'script',
    'svg',
    'template',
    'thead',
    'title',
    'video',
    'xmp',
  ],
  ALLOW_DATA_ATTR: false,
  ADD_ATTR: ['target', 'rel'],
  // No `data:` here: DOMPurify already keeps it on inline image sources, and on
  // a link target it would be a document the frame loads unsanitized.
  ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto):|[^a-z]|[a-z+.-]+(?:[^a-z+.-:]|$))/i,
};

export function sanitizeEmailHtml(html: string): string {
  DOMPurify.addHook('uponSanitizeAttribute', (_node, data) => {
    if (data.attrName === 'style') {
      data.attrValue = sanitizeCssValue(data.attrValue, false);
    }
  });

  const clean = DOMPurify.sanitize(html, DOMPURIFY_CONFIG);

  DOMPurify.removeHook('uponSanitizeAttribute');
  return clean;
}

/** Elements that fetch a remote URL on their own, without user interaction.
 *  DOMPurify's `html` profile allows `audio`/`video`/`source`/`track` through,
 *  so gating only `img` left a wide-open tracking channel. */
const REMOTE_FETCHING_TAGS = new Set(['IMG', 'SOURCE', 'VIDEO', 'AUDIO', 'TRACK']);

/** URL-bearing attributes on those elements. */
const REMOTE_URL_ATTRS = ['src', 'poster'] as const;

/** An http(s) or protocol-relative URL, in any spelling the browser resolves to
 *  a remote host — something that leaves the device. */
const REMOTE_URL = /^\s*(?:https?:|[\\/]{2})/i;
/** A `url(...)` in CSS text pointing at such a URL. */
const REMOTE_CSS_URL = /url\(\s*['"]?\s*(?:https?:)?\/\//i;

/**
 * Like `sanitizeEmailHtml`, but also optionally blocks remote content.
 * When `allowRemoteContent` is false, remote `src`/`poster`/`srcset` attributes
 * are stripped from every element that fetches on its own
 * ([`REMOTE_FETCHING_TAGS`]). `hasBlockedImages` tells the caller whether any
 * were removed so it can show a "Load images" banner.
 *
 * To show images after blocking, call this again with `allowRemoteContent: true`
 * and the original (pre-sanitized) HTML — no need to store any intermediate state.
 *
 * Remote-content gating also propagates into CSS `url(...)` tokens inside
 * inline `style` attributes so a tracker can't sneak in via `background-image`,
 * and strips the legacy `background` attribute. CSS inside `<style>` blocks is
 * not rewritten: `EmailHtmlFrame` blocks those loads with a per-frame CSP, and
 * a remote `url(...)` there only raises `hasBlockedImages` so the banner shows.
 */
export function sanitizeEmailHtmlFull(
  html: string,
  allowRemoteContent: boolean,
): { html: string; hasBlockedImages: boolean } {
  let hasBlockedImages = false;

  DOMPurify.addHook('uponSanitizeAttribute', (_node, data) => {
    if (data.attrName === 'style') {
      data.attrValue = sanitizeCssValue(data.attrValue, allowRemoteContent);
    }
  });

  if (!allowRemoteContent) {
    DOMPurify.addHook('afterSanitizeAttributes', (node) => {
      const el = node as Element;
      // The legacy `background` attribute fetches on any element carrying it.
      if (REMOTE_URL.test(el.getAttribute?.('background') ?? '')) {
        el.removeAttribute('background');
        hasBlockedImages = true;
      }
      if (el.tagName === 'STYLE' && REMOTE_CSS_URL.test(el.textContent ?? '')) {
        hasBlockedImages = true;
      }
      if (!REMOTE_FETCHING_TAGS.has(el.tagName)) return;

      // `poster` is the sneakiest of these: a <video poster="https://…"> fetches
      // on render with no user interaction, making it a guaranteed read receipt.
      for (const attr of REMOTE_URL_ATTRS) {
        if (REMOTE_URL.test(el.getAttribute?.(attr) ?? '')) {
          el.removeAttribute(attr);
          hasBlockedImages = true;
        }
      }
      // srcset holds a comma-separated candidate list rather than one URL, so it
      // is dropped wholesale rather than pattern-matched.
      if (el.getAttribute?.('srcset')) {
        el.removeAttribute('srcset');
        hasBlockedImages = true;
      }
    });
  }

  const clean = DOMPurify.sanitize(html, DOMPURIFY_CONFIG);

  DOMPurify.removeHook('uponSanitizeAttribute');
  if (!allowRemoteContent) DOMPurify.removeHook('afterSanitizeAttributes');

  return { html: clean, hasBlockedImages };
}

export function getSafeExternalUrl(value: string): string | null {
  try {
    const url = new URL(value);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    return url.toString();
  } catch {
    return null;
  }
}

export interface ParsedMailto {
  to: string[];
  subject: string;
  body: string;
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/**
 * Parse a `mailto:` URL (RFC 6068) into compose-prefill fields. Returns null
 * unless at least one valid address is present, so callers can fall through
 * to their existing link handling for junk hrefs.
 */
export function parseMailtoUrl(value: string): ParsedMailto | null {
  if (!/^mailto:/i.test(value)) return null;

  const rest = value.slice('mailto:'.length);
  const queryIdx = rest.indexOf('?');
  const pathPart = queryIdx === -1 ? rest : rest.slice(0, queryIdx);
  const queryPart = queryIdx === -1 ? '' : rest.slice(queryIdx + 1);

  let subject = '';
  let body = '';
  const queryAddresses: string[] = [];
  for (const pair of queryPart.split('&')) {
    if (!pair) continue;
    const eq = pair.indexOf('=');
    const key = (eq === -1 ? pair : pair.slice(0, eq)).toLowerCase();
    const raw = eq === -1 ? '' : pair.slice(eq + 1);
    if (key === 'subject') subject = safeDecode(raw.replace(/\+/g, ' '));
    else if (key === 'body') body = safeDecode(raw.replace(/\+/g, ' '));
    else if (key === 'to') queryAddresses.push(safeDecode(raw));
  }

  const to: string[] = [];
  const seen = new Set<string>();
  for (const chunk of [safeDecode(pathPart), ...queryAddresses]) {
    for (const candidate of chunk.split(',')) {
      const address = candidate.trim().toLowerCase();
      if (!address.includes('@') || seen.has(address)) continue;
      seen.add(address);
      to.push(address);
    }
  }

  if (to.length === 0) return null;
  return { to, subject, body };
}
