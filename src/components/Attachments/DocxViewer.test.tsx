// A Word attachment renders like an email body: converted to HTML, then
// sanitized into the sandboxed frame with no remote content by default. A
// file that will not convert says so and logs why.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => {}) }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  isSenderTrusted: vi.fn(async () => false),
  addTrustedSender: vi.fn(),
}));
vi.mock('@/lib/docxPreview', () => ({ docxToPreviewHtml: vi.fn() }));

import { docxToPreviewHtml } from '@/lib/docxPreview';
import { useLogStore } from '@/stores/logStore';
import { useThemeStore } from '@/stores/themeStore';
import { DocxViewer } from './DocxViewer';

const TRACKER = 'https://tracker.example/pixel.gif';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useLogStore.setState({ entries: [] });
  useThemeStore.setState({ scheme: 'light' });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(): Promise<void> {
  await act(async () =>
    root.render(
      <DocxViewer
        dataUrl="data:application/octet-stream;base64,QUJD"
        title="minutes.docx"
        accountId="acct-1"
        senderEmail="sender@example.com"
      />,
    ),
  );
}

describe('DocxViewer', () => {
  it('converts the file and shows it sanitized in the email frame', async () => {
    vi.mocked(docxToPreviewHtml).mockResolvedValue(`<p>Quarterly review</p><img src="${TRACKER}">`);
    await render();

    const bytes = new Uint8Array(vi.mocked(docxToPreviewHtml).mock.calls[0][0]);
    expect([...bytes]).toEqual([0x41, 0x42, 0x43]);
    const srcdoc = container.querySelector('iframe')?.getAttribute('srcdoc') ?? '';
    expect(srcdoc).toContain('<p>Quarterly review</p>');
    expect(srcdoc).not.toContain(TRACKER);
    expect(srcdoc).toContain('Content-Security-Policy');
  });

  it('keeps the page as paper on the dark theme, as the PDF viewer does', async () => {
    // A white Word page would otherwise be inverted like a light email.
    useThemeStore.setState({ scheme: 'dark' });
    vi.mocked(docxToPreviewHtml).mockResolvedValue(
      '<div class="docx-wrapper" style="background: gray"><section class="docx" style="background: white"><p>Page</p></section></div>',
    );
    await render();

    const srcdoc = container.querySelector('iframe')?.getAttribute('srcdoc') ?? '';
    expect(srcdoc).toContain('<p>Page</p>');
    expect(srcdoc).toContain('<meta name="color-scheme" content="light">');
    expect(srcdoc).not.toContain('invert(1)');
  });

  it('says the preview failed and logs the reason', async () => {
    vi.mocked(docxToPreviewHtml).mockRejectedValue(new Error('end of central directory not found'));
    await render();

    expect(container.querySelector('iframe')).toBeNull();
    expect(container.textContent).toContain('attachments:viewer.previewLoadFailed');
    const [log] = useLogStore.getState().entries;
    expect(log.level).toBe('error');
    expect(log.message).toContain('minutes.docx');
    expect(log.message).toContain('end of central directory not found');
  });
});
