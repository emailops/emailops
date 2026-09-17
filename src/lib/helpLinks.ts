/**
 * `help://<lang>/<page>#<anchor>` — the link a chat answer uses to cite a
 * section of the bundled user guides (see
 * `src-tauri/src/services/help_docs/retrieval.rs`, `help_link`). The UI turns
 * it into the public docs URL, so the chip opens the same page the guide is
 * published at. Pure, so the mapping is pinned by tests.
 */

/** Public docs site the guides in `docs/site/` are published to. */
export const HELP_DOCS_BASE_URL = 'https://getemailops.com';

const HELP_LINK = /^help:\/\/([a-z]{2})\/([a-z0-9-]+)(?:#([\p{L}\p{N}-]+))?$/u;

export interface HelpLink {
  lang: string;
  page: string;
  /** `null` for a page-level link (the intro section). */
  anchor: string | null;
}

/** Parse a help link; `null` for anything that is not one — a model can
 *  invent a scheme-shaped string, and that must render as plain text. */
export function parseHelpLink(href: string): HelpLink | null {
  const m = HELP_LINK.exec(href.trim());
  if (!m) return null;
  return { lang: m[1], page: m[2], anchor: m[3] ?? null };
}

/** `help://es/ai-features#choosing-a-backend` →
 *  `https://getemailops.com/es/docs/ai-features/#choosing-a-backend`. */
export function helpLinkToDocsUrl(href: string): string | null {
  const link = parseHelpLink(href);
  if (!link) return null;
  const base = `${HELP_DOCS_BASE_URL}/${link.lang}/docs/${link.page}/`;
  return link.anchor ? `${base}#${link.anchor}` : base;
}
