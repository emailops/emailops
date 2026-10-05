import { describe, expect, it } from 'vitest';
import type { AccountSignature } from '@/types';
import {
  applyAccountSignature,
  draftComposeBody,
  hasSignature,
  insertSignature,
  replaceBodyKeepingSignature,
  SIGNATURE_ATTR,
  signatureFor,
  swapSignature,
  withoutSignature,
} from './signature';

const ANA = '<p>Ana Lopez</p><p>Example Ltd</p>';
const BEA = '<p>Bea Ruiz</p>';
const block = (html: string) => `<div ${SIGNATURE_ATTR}="">${html}</div>`;

function sig(over: Partial<AccountSignature> = {}): AccountSignature {
  return { accountId: 'acc1', html: ANA, useForNew: true, useForReplies: true, updatedAt: 1, ...over };
}

function signatureHtmlOf(body: string): string | null {
  const doc = new DOMParser().parseFromString(`<body>${body}</body>`, 'text/html');
  const blocks = doc.body.querySelectorAll(`[${SIGNATURE_ATTR}]`);
  if (blocks.length > 1) throw new Error(`signature inserted ${blocks.length} times`);
  return blocks[0]?.innerHTML ?? null;
}

describe('signatureFor', () => {
  it.each([
    ['new', sig(), ANA],
    ['reply', sig(), ANA],
    ['forward', sig(), ANA],
    ['new', sig({ useForNew: false }), null],
    ['reply', sig({ useForReplies: false }), null],
    ['forward', sig({ useForReplies: false }), null],
    ['reply', sig({ useForNew: false }), ANA],
    ['new', sig({ html: '' }), null],
    ['new', null, null],
  ] as const)('%s with %o → %s', (kind, signature, expected) => {
    expect(signatureFor(signature, kind)).toBe(expected);
  });
});

describe('insertSignature', () => {
  it('puts the signature below an empty line in a new message', () => {
    expect(insertSignature('', ANA, 'new')).toBe(`<p></p><p></p>${block(ANA)}`);
  });

  it('keeps a prefilled body above the signature', () => {
    expect(insertSignature('<p>Hello</p>', ANA, 'reply')).toBe(`<p>Hello</p><p></p>${block(ANA)}`);
  });

  it('puts the signature above the forwarded message', () => {
    const forwarded = '<p></p><p>---------- Forwarded message ----------</p><p>Original</p>';
    const out = insertSignature(forwarded, ANA, 'forward');
    expect(out).toBe(`<p></p>${block(ANA)}${forwarded}`);
  });

  it('does nothing without a signature', () => {
    expect(insertSignature('<p>Hello</p>', null, 'new')).toBe('<p>Hello</p>');
  });

  it('never inserts a second signature', () => {
    const once = insertSignature('', ANA, 'new');
    expect(insertSignature(once, BEA, 'new')).toBe(once);
    expect(signatureHtmlOf(insertSignature(once, ANA, 'new'))).toBe(ANA);
  });
});

describe('swapSignature', () => {
  it("replaces the old account's signature with the new one, keeping the text", () => {
    const body = insertSignature('<p>Hello</p>', ANA, 'new');
    const out = swapSignature(body, BEA, 'new');
    expect(signatureHtmlOf(out)).toBe(BEA);
    expect(out.startsWith('<p>Hello</p>')).toBe(true);
  });

  it('removes the block when the new account has no signature', () => {
    const body = insertSignature('<p>Hello</p>', ANA, 'new');
    const out = swapSignature(body, null, 'new');
    expect(hasSignature(out)).toBe(false);
    expect(out).toContain('<p>Hello</p>');
  });

  it('adds the signature when the body had none', () => {
    expect(signatureHtmlOf(swapSignature('<p>Hello</p>', BEA, 'new'))).toBe(BEA);
  });

  it('swaps in place above a forwarded message', () => {
    const forwarded = '<p></p><p>Forwarded</p>';
    const out = swapSignature(insertSignature(forwarded, ANA, 'forward'), BEA, 'forward');
    expect(out).toBe(`<p></p>${block(BEA)}${forwarded}`);
  });
});

describe('replaceBodyKeepingSignature', () => {
  it('lands an AI draft above the signature instead of wiping it', () => {
    const current = insertSignature('<p>brief</p>', ANA, 'new');
    const out = replaceBodyKeepingSignature(current, '<p>Dear Bea,</p><p>Thanks.</p>', 'new');
    expect(out).toBe(`<p>Dear Bea,</p><p>Thanks.</p><p></p>${block(ANA)}`);
  });

  it('keeps a signature the user edited in the composer', () => {
    const current = `<p>brief</p>${block('<p>Ana, on the road</p>')}`;
    expect(signatureHtmlOf(replaceBodyKeepingSignature(current, '<p>Draft</p>', 'reply'))).toBe(
      '<p>Ana, on the road</p>',
    );
  });

  it('keeps the signature above a new forwarded message', () => {
    const current = insertSignature('<p></p><p>Old</p>', ANA, 'forward');
    const out = replaceBodyKeepingSignature(current, '<p></p><p>Forwarded</p>', 'forward');
    expect(out).toBe(`<p></p>${block(ANA)}<p></p><p>Forwarded</p>`);
  });

  it('is just the new body when there is no signature', () => {
    expect(replaceBodyKeepingSignature('<p>brief</p>', '<p>Draft</p>', 'new')).toBe('<p>Draft</p>');
  });
});

describe('hasSignature', () => {
  it('detects the signature block', () => {
    expect(hasSignature(insertSignature('', ANA, 'new'))).toBe(true);
    expect(hasSignature('<p>Ana Lopez</p>')).toBe(false);
  });
});

describe('applyAccountSignature (the From account changed or its signature loaded)', () => {
  it('inserts into a fresh composer', () => {
    expect(signatureHtmlOf(applyAccountSignature('', ANA, 'new', { insertIfMissing: true }))).toBe(ANA);
  });

  it('swaps the block of any composer', () => {
    const body = insertSignature('<p>Hi</p>', ANA, 'new');
    expect(signatureHtmlOf(applyAccountSignature(body, BEA, 'new', { insertIfMissing: false }))).toBe(BEA);
  });

  it('leaves a reopened draft without a block alone', () => {
    // A draft that went through the provider lost the block's marker: its
    // signature is plain text now, and a second one must not be added.
    const draft = '<p>Hi</p><p>Ana Lopez</p>';
    expect(applyAccountSignature(draft, BEA, 'new', { insertIfMissing: false })).toBe(draft);
  });
});

describe('withoutSignature', () => {
  it('drops the signature block, so it is not read as part of the text', () => {
    expect(withoutSignature(insertSignature('<p>brief</p>', ANA, 'new'))).toBe('<p>brief</p><p></p>');
    expect(withoutSignature('<p>brief</p>')).toBe('<p>brief</p>');
  });
});

describe('draftComposeBody (reopening a saved draft in a compose tab)', () => {
  it('opens a plain-text draft (saved by the chat) with the signature inserted', () => {
    const out = draftComposeBody({ body: 'Thursday works.', bodyHtml: null });
    expect(out.bodyHtml).toContain('Thursday works.');
    expect(out.insertSignature).toBe(true);
  });

  it('opens a draft the composer saved as it is: its signature is already in it', () => {
    const html = `<p>Thursday works.</p>${block(ANA)}`;
    expect(draftComposeBody({ body: 'Thursday works.', bodyHtml: html })).toEqual({
      bodyHtml: html,
      insertSignature: false,
    });
  });
});
