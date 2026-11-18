// The From list is ordered by sent-mail usage once the counts arrive, without
// changing the sender the composer opened with.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/RichTextEditor', () => ({ RichTextEditor: () => null }));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/lib/api', () => ({
  currentPlatform: () => 'macos',
  getPref: vi.fn(async () => null),
  getSentCounts: vi.fn(async () => [{ accountId: 'work', sentCount: 12 }]),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
}));

import type { Account } from '@/types';
import { ComposeModal } from './ComposeModal';

const home = { id: 'home', email: 'home@mail.example', name: 'Home', enabled: true } as Account;
const work = { id: 'work', email: 'work@mail.example', name: 'Work', enabled: true } as Account;

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

describe('ComposeModal From list', () => {
  it('puts the most used sender first and keeps the sender it opened with', async () => {
    await act(async () => {
      root.render(<ComposeModal accounts={[home, work]} defaultAccountId="home" onClose={() => {}} />);
    });
    const from = document.querySelector<HTMLSelectElement>('select[aria-label="compose:from"]');
    if (!from) throw new Error('From list not rendered');
    expect(Array.from(from.options).map((o) => o.value)).toEqual(['work', 'home']);
    expect(from.value).toBe('home');
  });
});
