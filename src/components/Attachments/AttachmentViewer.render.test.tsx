// The Attachments view renders an HTML attachment like an email body:
// sanitized, with remote content only by consent.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => {}) }));
vi.mock('@/hooks/useFormatters', () => ({ useFormatters: () => ({ dateTime: () => 'date' }) }));
vi.mock('@/hooks/useOpenAttachment', () => ({
  useOpenAttachment: () => ({ openAttachment: vi.fn(), confirmDialog: null }),
}));
vi.mock('./PdfViewer', () => ({ PdfViewer: () => null }));
vi.mock('@/lib/api', () => ({
  getAttachmentData: vi.fn(),
  getPref: vi.fn(),
  isSenderTrusted: vi.fn(),
  addTrustedSender: vi.fn(),
}));

import * as api from '@/lib/api';
import type { Attachment } from '@/types';
import { AttachmentViewer } from './AttachmentViewer';

const TRACKER = 'https://tracker.example/pixel.gif';

const attachment = (overrides: Partial<Attachment>): Attachment =>
  ({
    id: 'att-1',
    accountId: 'acct-1',
    emailId: 'e1',
    filename: 'page.html',
    mimeType: 'text/html',
    fileSize: 10,
    senderEmail: 'sender@example.com',
    emailTimestamp: 0,
    tags: [],
    ...overrides,
  }) as Attachment;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.getPref).mockReset().mockResolvedValue(null);
  vi.mocked(api.isSenderTrusted).mockReset().mockResolvedValue(false);
  vi.mocked(api.getAttachmentData)
    .mockReset()
    .mockResolvedValue(btoa(`<p>Invoice</p><img src="${TRACKER}">`));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const frame = () => container.querySelector('iframe');

describe('AttachmentViewer', () => {
  it.each([
    ['its declared type', { mimeType: 'text/html' }],
    ['its filename', { filename: 'page.htm', mimeType: 'application/octet-stream' }],
  ])('renders html known by %s with no remote content', async (_how, overrides) => {
    await act(async () => root.render(<AttachmentViewer attachment={attachment(overrides)} />));

    expect(frame()?.getAttribute('src')).toBeNull();
    const srcdoc = frame()?.getAttribute('srcdoc') ?? '';
    expect(srcdoc).toContain('<p>Invoice</p>');
    expect(srcdoc).not.toContain(TRACKER);
    expect(srcdoc).toContain('Content-Security-Policy');
    expect(api.isSenderTrusted).toHaveBeenCalledWith('acct-1', 'sender@example.com');
  });
});
