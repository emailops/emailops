// "My organization": a Contacts tab listing the people on the account's own
// domain, offered only when that domain is a company's, not a free provider's.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
const orgDomain = vi.hoisted(() => ({ value: 'acme.example' as string | null }));
vi.mock('@/lib/api', () => ({
  getOrganizationDomain: vi.fn(() => Promise.resolve(orgDomain.value)),
  listContacts: vi.fn(() => Promise.resolve({ items: [], total: 0, hasMore: false })),
  listContactsByCompany: vi.fn(() => Promise.resolve([])),
  getContactDetail: vi.fn(() => Promise.resolve(null)),
  currentPlatform: vi.fn(() => 'macos'),
}));

import * as api from '@/lib/api';
import { ContactsView } from './ContactsView';

describe('ContactsView — My organization', () => {
  let container: HTMLDivElement;
  let root: Root;

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

  async function mount() {
    await act(async () => {
      root.render(<ContactsView accountId="acc-1" onComposeTo={() => {}} onViewEmailsFrom={() => {}} />);
    });
  }

  it('lists only the people of the account domain', async () => {
    orgDomain.value = 'acme.example';
    await mount();
    const tab = container.querySelector('[data-testid="contacts-mode-organization"]') as HTMLButtonElement;
    expect(tab).not.toBeNull();
    await act(async () => tab.click());
    expect(api.listContacts).toHaveBeenLastCalledWith('acc-1', expect.objectContaining({ domain: 'acme.example' }));
  });

  it('is not offered for a free provider address', async () => {
    orgDomain.value = null;
    await mount();
    expect(container.querySelector('[data-testid="contacts-mode-organization"]')).toBeNull();
  });
});
