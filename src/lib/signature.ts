/**
 * Placing an account's signature in a composer body (HTML strings, as the
 * compose editor holds them).
 *
 * The signature travels inside one recognisable block,
 * `<div data-emailops-signature>`, which the compose editor schema keeps
 * (`composeEditorExtensions`). That block is what lets a composer swap the
 * signature when the From account changes, keep it when an AI draft replaces
 * the text, and never insert it twice. The send sanitizer drops the data
 * attribute, so recipients get a plain `<div>`.
 *
 * The HTML shows no `-- ` separator (as in Gmail); the text/plain part gets
 * the standard one when the signature closes the message
 * (`prepareOutgoingHtml`). In a new message or a reply the signature sits
 * below the text; in a forward it sits above the forwarded message.
 */

import type { AccountSignature } from '@/types';

export const SIGNATURE_ATTR = 'data-emailops-signature';

/** What the composer is writing. A forward follows the "replies and forwards" option. */
export type ComposeKind = 'new' | 'reply' | 'forward';

const EMPTY_LINE = '<p></p>';

/** The signature HTML to insert for `kind`, or null when there is none or it is switched off for it. */
export function signatureFor(signature: AccountSignature | null | undefined, kind: ComposeKind): string | null {
  if (!signature?.html.trim()) return null;
  const enabled = kind === 'new' ? signature.useForNew : signature.useForReplies;
  return enabled ? signature.html : null;
}

function parse(html: string): HTMLElement {
  return new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html').body;
}

function findBlock(body: HTMLElement): Element | null {
  return body.querySelector(`[${SIGNATURE_ATTR}]`);
}

function blockHtml(signatureHtml: string): string {
  return `<div ${SIGNATURE_ATTR}="">${signatureHtml}</div>`;
}

/** Whether the body already carries a signature block. */
export function hasSignature(bodyHtml: string): boolean {
  return findBlock(parse(bodyHtml)) !== null;
}

/**
 * Add the signature to a composer body: below the text in a new message or a
 * reply (an empty line apart), above the forwarded message in a forward. A
 * body that already has one is left alone, so a reopened draft, a message
 * taken back from the outbox or a maximized composer never gets a second.
 */
export function insertSignature(bodyHtml: string, signatureHtml: string | null, kind: ComposeKind): string {
  if (!signatureHtml || hasSignature(bodyHtml)) return bodyHtml;
  if (kind === 'forward') return `${EMPTY_LINE}${blockHtml(signatureHtml)}${bodyHtml}`;
  const text = bodyHtml.trim() ? bodyHtml : EMPTY_LINE;
  return `${text}${EMPTY_LINE}${blockHtml(signatureHtml)}`;
}

/**
 * Put another account's signature in place of the current one (the From
 * account changed). The text around it is untouched; `null` removes the
 * block. A body without a block gets the signature inserted.
 */
export function swapSignature(bodyHtml: string, signatureHtml: string | null, kind: ComposeKind): string {
  const body = parse(bodyHtml);
  const block = findBlock(body);
  if (!block) return insertSignature(bodyHtml, signatureHtml, kind);
  if (signatureHtml) {
    block.innerHTML = signatureHtml;
  } else {
    block.remove();
  }
  return body.innerHTML;
}

/**
 * The signature of the (new) From account into a composer body: swap the
 * block when there is one; otherwise insert it only when `insertIfMissing`
 * (a composer that starts fresh). A reopened draft without a block keeps its
 * text as is — its signature, if any, is plain text by now.
 */
export function applyAccountSignature(
  bodyHtml: string,
  signatureHtml: string | null,
  kind: ComposeKind,
  { insertIfMissing }: { insertIfMissing: boolean },
): string {
  if (!insertIfMissing && !hasSignature(bodyHtml)) return bodyHtml;
  return swapSignature(bodyHtml, signatureHtml, kind);
}

/**
 * Replace the text of a composer body while keeping its signature block as it
 * is (the user may have edited it): an AI draft lands above the signature
 * instead of wiping it, and a forward keeps it above the forwarded message.
 */
export function replaceBodyKeepingSignature(currentHtml: string, newBodyHtml: string, kind: ComposeKind): string {
  const block = findBlock(parse(currentHtml));
  if (!block) return newBodyHtml;
  if (kind === 'forward') return `${EMPTY_LINE}${block.outerHTML}${newBodyHtml}`;
  return `${newBodyHtml}${EMPTY_LINE}${block.outerHTML}`;
}

/** The body without its signature block: the text the user wrote, for an AI brief or a send check. */
export function withoutSignature(bodyHtml: string): string {
  const body = parse(bodyHtml);
  const block = findBlock(body);
  if (!block) return bodyHtml;
  block.remove();
  return body.innerHTML;
}
