import { getSafeExternalUrl } from '@/lib/emailFormatting';

/** How a `url` column's cell should be rendered. */
export type UrlCellPlan = { kind: 'link'; href: string } | { kind: 'text'; text: string };

/**
 * Decide whether a `url` column value is safe to render as a live link.
 *
 * Lens values are extracted from email bodies, so this is untrusted input. Only
 * `http`/`https` become links; anything else (`javascript:`, `data:`, `file:`,
 * custom schemes, plain non-URL text) renders as text. Kept as a pure function
 * so the scheme rules are table-testable without mounting the table.
 */
export function planUrlCell(value: unknown): UrlCellPlan {
  const text = String(value);
  const safe = getSafeExternalUrl(text);
  return safe ? { kind: 'link', href: safe } : { kind: 'text', text };
}
