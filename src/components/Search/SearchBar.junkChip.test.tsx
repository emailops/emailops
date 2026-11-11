// The search panel paints its own rows, and a search reaches flagged mail, so
// the warning has to be here as well as in the list.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/lib/api', () => ({
  listThreads: vi.fn(),
  autocompleteSenders: vi.fn(),
  getJunkVerdicts: vi.fn(),
}));

import type { ThreadPage } from '@/lib/api';
import * as api from '@/lib/api';
import { useJunkStore } from '@/stores/junkStore';
import type { Email, JunkVerdict } from '@/types';
import { SearchBar } from './SearchBar';

function hit(id: string) {
  return {
    id,
    accountId: 'acct',
    threadId: `t-${id}`,
    subject: 'Invoice',
    sender: 'Billing',
    senderEmail: 'billing@example.com',
    snippet: 'snippet',
    timestamp: 1_700_000_000,
    isRead: true,
  } as unknown as Email;
}

const RESULT: ThreadPage = {
  emails: [hit('e1'), hit('e2')],
  threadLatestAt: {},
  nextCursor: null,
  parsedQuery: null,
};

const FLAGGED = { emailId: 'e1', band: 'junk', primaryKind: 'phishing', userOverride: null } as JunkVerdict;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  useJunkStore.setState({ verdictsByEmail: {}, loaded: {} });
  vi.mocked(api.listThreads).mockResolvedValue(RESULT);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function search() {
  act(() => {
    root.render(
      <SearchBar accountId="acct" onSelectEmail={() => {}} onApplySearch={() => {}} view="inbox" onClose={() => {}} />,
    );
  });
  const input = container.querySelector('input') as HTMLInputElement;
  act(() => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setValue?.call(input, 'invoice');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(900);
  });
}

describe('SearchBar junk warning', () => {
  it('marks a flagged hit, and only that one', async () => {
    vi.mocked(api.getJunkVerdicts).mockResolvedValue({ e1: FLAGGED });

    await search();

    expect(container.querySelectorAll('[data-testid="junk-chip"]')).toHaveLength(1);
    expect(container.textContent).toContain('phishing');
  });

  it('previews the view it opened over, without logging a search', async () => {
    vi.mocked(api.getJunkVerdicts).mockResolvedValue({});

    await search();

    expect(api.listThreads).toHaveBeenLastCalledWith('acct', 'invoice', 'inbox', false, null, 50, false);
  });

  it('does not pass a first page off as every result', async () => {
    vi.mocked(api.getJunkVerdicts).mockResolvedValue({});
    vi.mocked(api.listThreads).mockResolvedValue({
      ...RESULT,
      nextCursor: { threadTs: 1, timestamp: 1, id: 'e2' },
    });

    await search();

    expect(container.textContent).toContain('inbox:searchPanel.firstResults');
  });

  it('shows no hit whose verdict could not be read', async () => {
    vi.mocked(api.getJunkVerdicts).mockRejectedValue(new Error('no verdicts'));

    await search();

    expect(container.textContent).not.toContain('Invoice');
    expect(container.textContent).toContain('no verdicts');
  });
});
