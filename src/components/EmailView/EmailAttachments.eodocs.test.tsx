// A Word or Excel attachment can be opened in EO Docs: it is imported and the
// app is asked to show it. Other files, or EO Docs off, offer nothing new.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('@/lib/api', () => ({
  getEmailAttachmentMetas: vi.fn(async () => [
    {
      id: 'm1',
      filename: 'Budget.xlsx',
      mimeType: 'application/vnd.ms-excel',
      fileSize: 10,
      providerAttachmentId: 'p1',
    },
    { id: 'm2', filename: 'photo.png', mimeType: 'image/png', fileSize: 10, providerAttachmentId: 'p2' },
  ]),
  fetchEmailAttachmentBytes: vi.fn(async () => 'QUJD'),
}));
vi.mock('@/lib/officeImport', async (orig) => ({
  ...(await orig<typeof import('@/lib/officeImport')>()),
  importOfficeFile: vi.fn(async () => ({ docs: [{ id: 'd1', folderId: null }], skippedImages: 0 })),
}));

import { importOfficeFile } from '@/lib/officeImport';
import { useSharedDocsEnabledStore } from '@/stores/featureToggleStore';
import { useSharedDocsStore } from '@/stores/sharedDocsStore';
import { EmailAttachments } from './EmailAttachments';

describe('EmailAttachments — Open in EO Docs', () => {
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

  async function mount(eoDocs: boolean) {
    useSharedDocsEnabledStore.setState({ enabled: eoDocs, isLoading: false });
    await act(async () => {
      root.render(<EmailAttachments emailId="e1" accountId="acc-1" onOpenAttachment={() => {}} />);
    });
  }

  it('imports a spreadsheet attachment and asks to show it', async () => {
    await mount(true);
    expect(container.querySelector('[data-testid="attachment-open-eodocs-m2"]')).toBeNull();
    const before = useSharedDocsStore.getState().openRequests;
    await act(async () =>
      (container.querySelector('[data-testid="attachment-open-eodocs-m1"]') as HTMLButtonElement).click(),
    );
    expect(importOfficeFile).toHaveBeenCalledWith('acc-1', 'Budget.xlsx', 'QUJD', null);
    expect(useSharedDocsStore.getState().openRequests).toBe(before + 1);
    expect(useSharedDocsStore.getState().selectedId).toBe('d1');
  });

  it('offers nothing while EO Docs is off', async () => {
    await mount(false);
    expect(container.querySelector('[data-testid^="attachment-open-eodocs-"]')).toBeNull();
  });
});
