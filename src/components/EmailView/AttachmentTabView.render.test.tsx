// A PDF tab renders through pdf.js (PdfViewer, which owns the zoom), and an
// HTML tab renders like an email body: sanitized, with remote content only by
// consent. Every other framed type stays in a fully sandboxed iframe, because
// the declared MIME type is the sender's word and markup could run script.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/components/Attachments/PdfViewer', () => ({
  PdfViewer: ({ title }: { title: string }) => <div data-testid="pdf-viewer-stub">{title}</div>,
}));

vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => {}) }));

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  isSenderTrusted: vi.fn(),
  addTrustedSender: vi.fn(),
}));

import * as api from '@/lib/api';
import type { AttachmentViewTab } from '@/stores/emailStore';
import { AttachmentTabView } from './AttachmentTabView';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.getPref).mockReset().mockResolvedValue(null);
  vi.mocked(api.isSenderTrusted).mockReset().mockResolvedValue(false);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const tab = (mimeType: string): AttachmentViewTab => ({
  type: 'attachment',
  id: 'att-1',
  accountId: 'acct-1',
  emailId: 'e1',
  filename: 'file',
  mimeType,
  senderEmail: 'sender@example.com',
  dataUrl: `data:${mimeType};base64,AAAA`,
  isLoading: false,
});

const TRACKER = 'https://tracker.example/pixel.gif';
const htmlTab: AttachmentViewTab = {
  ...tab('text/html'),
  dataUrl: `data:text/html;base64,${btoa(`<p>Invoice</p><img src="${TRACKER}">`)}`,
};

async function renderHtmlTab(): Promise<void> {
  await act(async () => root.render(<AttachmentTabView tab={htmlTab} onClose={vi.fn()} />));
}

const frame = () => container.querySelector('iframe');
const srcdoc = () => frame()?.getAttribute('srcdoc') ?? '';

describe('AttachmentTabView', () => {
  it('renders a pdf with the pdf.js viewer, not an iframe', () => {
    act(() => root.render(<AttachmentTabView tab={tab('application/pdf')} onClose={vi.fn()} />));
    expect(container.querySelector('[data-testid="pdf-viewer-stub"]')).not.toBeNull();
    expect(container.querySelector('iframe')).toBeNull();
  });

  it.each(['image/svg+xml', 'application/xhtml+xml', 'application/octet-stream', 'image/png'])(
    'frames %s in a fully restrictive sandbox',
    (mime) => {
      act(() => root.render(<AttachmentTabView tab={tab(mime)} onClose={vi.fn()} />));
      expect(container.querySelector('iframe')?.getAttribute('sandbox')).toBe('');
    },
  );

  it('renders html sanitized, with no remote content until the user allows it', async () => {
    await renderHtmlTab();

    expect(frame()?.getAttribute('src')).toBeNull();
    expect(srcdoc()).toContain('<p>Invoice</p>');
    expect(srcdoc()).not.toContain(TRACKER);
    expect(srcdoc()).toContain('Content-Security-Policy');

    const showImages = [...container.querySelectorAll('button')].find(
      (b) => b.textContent === 'inbox:emailView.showImages',
    );
    await act(async () => showImages?.click());

    expect(srcdoc()).toContain(TRACKER);
    expect(srcdoc()).not.toContain('Content-Security-Policy');
  });

  it('loads remote content in html when the preference allows it', async () => {
    vi.mocked(api.getPref).mockResolvedValue('true');
    await renderHtmlTab();

    expect(srcdoc()).toContain(TRACKER);
    expect(container.textContent).not.toContain('inbox:emailView.remoteImagesBlocked');
  });

  it('loads remote content in html from a sender the user trusts', async () => {
    vi.mocked(api.isSenderTrusted).mockResolvedValue(true);
    await renderHtmlTab();

    expect(api.isSenderTrusted).toHaveBeenCalledWith('acct-1', 'sender@example.com');
    expect(srcdoc()).toContain(TRACKER);
  });
});
