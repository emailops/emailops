// A Word (.docx) attachment opens in the app's viewer, like a PDF, even when
// the sender declared a generic type; an old .doc still goes to the OS.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('@/lib/api', () => ({
  getEmailAttachmentMetas: vi.fn(async () => [
    {
      id: 'm1',
      filename: 'minutes.docx',
      mimeType: 'application/octet-stream',
      fileSize: 10,
      providerAttachmentId: 'p1',
      filePath: 'attachments/a/minutes.docx',
    },
    {
      id: 'm2',
      filename: 'legacy.doc',
      mimeType: 'application/msword',
      fileSize: 10,
      providerAttachmentId: 'p2',
      filePath: 'attachments/a/legacy.doc',
    },
  ]),
  openEmailAttachmentMeta: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';
import { EmailAttachments } from './EmailAttachments';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const button = (filename: string) =>
  [...container.querySelectorAll('button')].find((b) => b.title.startsWith(filename)) as HTMLButtonElement;

describe('EmailAttachments — viewing in the app', () => {
  it('opens a .docx in the viewer, not with the OS', async () => {
    const onOpen = vi.fn();
    await act(async () => root.render(<EmailAttachments emailId="e1" accountId="acc-1" onOpenAttachment={onOpen} />));
    await act(async () => button('minutes.docx').click());
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 'm1' }));
    expect(api.openEmailAttachmentMeta).not.toHaveBeenCalled();
  });

  it('leaves an old .doc to the OS', async () => {
    const onOpen = vi.fn();
    await act(async () => root.render(<EmailAttachments emailId="e1" accountId="acc-1" onOpenAttachment={onOpen} />));
    await act(async () => button('legacy.doc').click());
    expect(onOpen).not.toHaveBeenCalled();
    expect(api.openEmailAttachmentMeta).toHaveBeenCalledWith('acc-1', 'm2', false);
  });
});
