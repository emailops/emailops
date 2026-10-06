// The composer's attach button: straight to the file picker while EO Docs is
// off; with EO Docs on, a choice between this computer and EO Docs, and the
// EO Docs picker warns that only EmailOps users can open them and asks for
// consent before anything is attached.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(() => Promise.resolve(null)),
  setPref: vi.fn(() => Promise.resolve()),
  listSharedDocs: vi.fn(() =>
    Promise.resolve([
      { id: 'd1', title: 'Plan', kind: 'doc', status: 'active' },
      { id: 'd2', title: 'Invite', kind: 'doc', status: 'invited' },
    ]),
  ),
}));

import { useSharedDocsEnabledStore } from '@/stores/featureToggleStore';
import { AttachMenu } from './AttachMenu';

const q = (sel: string) => document.querySelector<HTMLElement>(sel);

describe('AttachMenu', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onPickFiles = vi.fn();
  const onPickEoDocs = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.clearAllMocks();
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
      root.render(
        <AttachMenu accountId="acc-1" onPickFiles={onPickFiles} onPickEoDocs={onPickEoDocs} className="">
          clip
        </AttachMenu>,
      );
    });
  }

  it('opens the file picker directly while EO Docs is off', async () => {
    await mount(false);
    act(() => q('[data-testid="compose-attach"]')?.click());
    expect(onPickFiles).toHaveBeenCalled();
    expect(q('[data-testid="compose-attach-eodocs"]')).toBeNull();
  });

  it('offers this computer or EO Docs when EO Docs is on', async () => {
    await mount(true);
    act(() => q('[data-testid="compose-attach"]')?.click());
    act(() => q('[data-testid="compose-attach-computer"]')?.click());
    expect(onPickFiles).toHaveBeenCalled();
  });

  it('attaches picked EO Docs only after the warning is acknowledged', async () => {
    await mount(true);
    act(() => q('[data-testid="compose-attach"]')?.click());
    await act(async () => q('[data-testid="compose-attach-eodocs"]')?.click());

    expect(q('[data-testid="eodocs-attach-warning"]')?.textContent).toBe('documents:attachDialog.warning');
    expect(q('[data-testid="eodocs-attach-d2"]')).toBeNull();
    act(() => q('[data-testid="eodocs-attach-d1"]')?.click());
    expect((q('[data-testid="eodocs-attach-submit"]') as HTMLButtonElement).disabled).toBe(true);
    act(() => q('[data-testid="eodocs-attach-consent"]')?.click());
    act(() => q('[data-testid="eodocs-attach-submit"]')?.click());

    expect(onPickEoDocs).toHaveBeenCalledWith([expect.objectContaining({ id: 'd1' })]);
  });
});
