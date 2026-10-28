// A PDF tab renders through pdf.js (PdfViewer, which owns the zoom); every
// other framed type stays in a fully sandboxed iframe, because the declared
// MIME type is the sender's word and markup could run script.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/components/Attachments/PdfViewer', () => ({
  PdfViewer: ({ title }: { title: string }) => <div data-testid="pdf-viewer-stub">{title}</div>,
}));

import type { AttachmentViewTab } from '@/stores/emailStore';
import { AttachmentTabView } from './AttachmentTabView';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
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
  dataUrl: `data:${mimeType};base64,AAAA`,
  isLoading: false,
});

describe('AttachmentTabView', () => {
  it('renders a pdf with the pdf.js viewer, not an iframe', () => {
    act(() => root.render(<AttachmentTabView tab={tab('application/pdf')} onClose={vi.fn()} />));
    expect(container.querySelector('[data-testid="pdf-viewer-stub"]')).not.toBeNull();
    expect(container.querySelector('iframe')).toBeNull();
  });

  it.each(['text/html', 'image/svg+xml', 'application/xhtml+xml', 'application/octet-stream', 'image/png'])(
    'frames %s in a fully restrictive sandbox',
    (mime) => {
      act(() => root.render(<AttachmentTabView tab={tab(mime)} onClose={vi.fn()} />));
      expect(container.querySelector('iframe')?.getAttribute('sandbox')).toBe('');
    },
  );
});
