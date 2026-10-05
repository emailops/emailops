// Reading-pane sender controls: the Unsubscribe link only when the message
// offers a way out, and the banner on a blocked or unsubscribed sender.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getSenderStatus: vi.fn(),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useSenderStore } from '@/stores/senderStore';
import type { SenderStatus } from '@/types';
import { SenderBanner, UnsubscribeButton } from './SenderControls';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
  useSenderStore.setState({ statusByEmail: {}, blocked: [], dialog: null });
});

const status = (extra: Partial<SenderStatus> = {}): SenderStatus => ({
  address: 'deals@shop.example',
  blocked: false,
  unsubscribe: null,
  unsubscribedAt: null,
  ...extra,
});

async function render(node: React.ReactNode) {
  await act(async () => {
    root.render(node);
  });
}

const byTestId = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);

describe('UnsubscribeButton', () => {
  it('is absent when the message offers no way to unsubscribe', async () => {
    vi.mocked(api.getSenderStatus).mockResolvedValue(status());
    await render(<UnsubscribeButton accountId="acc" emailId="m1" senderName="Shop" />);
    expect(api.getSenderStatus).toHaveBeenCalledWith('acc', 'm1');
    expect(byTestId('unsubscribe-button')).toBeNull();
  });

  it('opens the confirmation for this message', async () => {
    vi.mocked(api.getSenderStatus).mockResolvedValue(
      status({ unsubscribe: { kind: 'oneClick', target: 'shop.example', url: null } }),
    );
    await render(<UnsubscribeButton accountId="acc" emailId="m1" senderName="Shop" />);

    await act(async () => byTestId('unsubscribe-button')?.click());

    expect(useSenderStore.getState().dialog).toEqual({
      type: 'unsubscribe',
      accountId: 'acc',
      emailId: 'm1',
      senderName: 'Shop',
    });
  });

  it('says "Unsubscribed" once the user asked', async () => {
    vi.mocked(api.getSenderStatus).mockResolvedValue(
      status({ unsubscribe: { kind: 'mailto', target: 'leave@shop.example', url: null }, unsubscribedAt: 100 }),
    );
    await render(<UnsubscribeButton accountId="acc" emailId="m1" senderName="Shop" />);
    expect(byTestId('unsubscribe-button')).toBeNull();
    expect(byTestId('unsubscribed-label')?.textContent).toBe('Unsubscribed');
  });
});

describe('SenderBanner', () => {
  it('renders nothing for a sender that is neither blocked nor unsubscribed', async () => {
    vi.mocked(api.getSenderStatus).mockResolvedValue(status());
    await render(<SenderBanner accountId="acc" emailId="m1" />);
    expect(container.textContent).toBe('');
  });

  it('tells a blocked sender apart and offers Unblock', async () => {
    vi.mocked(api.getSenderStatus).mockResolvedValue(status({ blocked: true }));
    await render(<SenderBanner accountId="acc" emailId="m1" />);

    expect(byTestId('blocked-sender-banner')?.textContent).toContain('You blocked this sender');
    const unblock = [...container.querySelectorAll('button')].find((b) => b.textContent === 'Unblock');
    await act(async () => unblock?.click());

    expect(useSenderStore.getState().dialog).toEqual({
      type: 'unblock',
      accountId: 'acc',
      address: 'deals@shop.example',
    });
  });

  it('says when the user unsubscribed', async () => {
    vi.mocked(api.getSenderStatus).mockResolvedValue(status({ unsubscribedAt: 1_700_000_000 }));
    await render(<SenderBanner accountId="acc" emailId="m1" />);
    expect(byTestId('unsubscribed-banner')?.textContent).toContain('You unsubscribed from this sender on');
  });
});
