// The compose-side half of per-account signatures: the hook loads the From
// account's signature and keeps it in the body — added on open, swapped when
// the From account changes, re-attached after an AI draft replaces the body.

import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useComposeSignature } from './useComposeSignature';

const api = vi.hoisted(() => ({ getFullSignature: vi.fn(), getPref: vi.fn() }));
vi.mock('@/lib/api', () => api);

const SIGNATURES: Record<string, string> = { a: 'Alice\nAcme', b: 'Bob', none: '', logo: '' };
const LOGO = 'data:image/png;base64,AAAA';
const IMAGES: Record<string, { src: string; width: number }> = { logo: { src: LOGO, width: 150 } };

let container: HTMLDivElement;
let root: Root;
let latest: {
  body: string;
  setAccount: (id: string) => void;
  withSignature: (h: string) => string;
  withDraftSignature: (h: string) => string;
};

function Harness({ initialAccount, initialBody }: { initialAccount: string; initialBody: string }) {
  const [account, setAccount] = useState(initialAccount);
  const [body, setBody] = useState(initialBody);
  const { withSignature, withDraftSignature } = useComposeSignature(account, setBody);
  latest = { body, setAccount, withSignature, withDraftSignature };
  return null;
}

async function render(initialAccount: string, initialBody = '') {
  await act(async () => {
    root.render(<Harness initialAccount={initialAccount} initialBody={initialBody} />);
  });
}

beforeEach(() => {
  api.getPref.mockReset().mockResolvedValue(null);
  api.getFullSignature
    .mockReset()
    .mockImplementation(async (id: string) => ({ text: SIGNATURES[id] ?? '', image: IMAGES[id] ?? null }));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('useComposeSignature', () => {
  it('adds the account signature to an empty new message', async () => {
    await render('a');
    expect(latest.body).toBe('<p></p><p>--</p><p>Alice</p><p>Acme</p>');
  });

  it('places it between a reply and the quoted original', async () => {
    await render('b', '<p>Thanks</p><blockquote><p>q</p></blockquote>');
    expect(latest.body).toBe('<p>Thanks</p><p>--</p><p>Bob</p><blockquote><p>q</p></blockquote>');
  });

  it('swaps the signature when the From account changes', async () => {
    await render('a', '<p>Hi</p>');
    await act(async () => latest.setAccount('b'));
    expect(latest.body).toBe('<p>Hi</p><p>--</p><p>Bob</p>');
  });

  it('removes it when switching to an account without one', async () => {
    await render('a', '<p>Hi</p>');
    await act(async () => latest.setAccount('none'));
    expect(latest.body).toBe('<p>Hi</p>');
  });

  it('leaves the body untouched for an account without a signature', async () => {
    await render('none', '<p>Hi</p>');
    expect(latest.body).toBe('<p>Hi</p>');
  });

  it('re-attaches the signature to a replacement body (AI draft)', async () => {
    await render('a', '<p>brief</p>');
    expect(latest.withSignature('<p>Generated draft</p>')).toBe(
      '<p>Generated draft</p><p>--</p><p>Alice</p><p>Acme</p>',
    );
  });

  it('adds an image-only signature at its width', async () => {
    await render('logo', '<p>Hi</p>');
    expect(latest.body).toBe(`<p>Hi</p><p>--</p><img src="${LOGO}" alt="Signature" width="150">`);
  });

  it('signs an AI draft with the custom signature by default', async () => {
    await render('a', '<p>brief</p>');
    expect(latest.withDraftSignature('<p>Draft</p>')).toBe('<p>Draft</p><p>--</p><p>Alice</p><p>Acme</p>');
  });

  it.each(['name', 'none'])('leaves an AI draft unsigned when the sign-off is "%s"', async (mode) => {
    api.getPref.mockResolvedValue(mode);
    await render('a', '<p>brief</p>');
    expect(latest.withDraftSignature('<p>Draft</p>\n<p>Cordialement,</p><p>Maxime</p>')).toBe(
      '<p>Draft</p>\n<p>Cordialement,</p><p>Maxime</p>',
    );
    // A hand-written message still gets the signature.
    expect(latest.withSignature('<p>Hi</p>')).toContain('<p>Alice</p>');
  });

  it('keeps the body usable when the signature cannot be loaded', async () => {
    api.getFullSignature.mockRejectedValueOnce(new Error('db locked'));
    await render('a', '<p>Hi</p>');
    expect(latest.body).toBe('<p>Hi</p>');
  });
});
