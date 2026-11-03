// The dates and labels of a message follow the UI language.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'es' } }),
}));
vi.mock('@/lib/api', () => ({ getEmailBody: vi.fn(async () => '') }));
vi.mock('./AiMarkupBanner', () => ({ AiMarkupBanner: () => null }));
vi.mock('./CalendarInviteCard', () => ({ CalendarInviteCard: () => null }));
vi.mock('./EmailAttachments', () => ({ EmailAttachments: () => null }));
vi.mock('./EmailBody', () => ({ EmailBody: () => null }));
vi.mock('./JunkBanner', () => ({ JunkBanner: () => null }));
vi.mock('./SenderControls', () => ({ SenderBanner: () => null, UnsubscribeButton: () => null }));
vi.mock('./TranslationControls', () => ({ TranslatedEmailBody: () => null, TranslationControls: () => null }));

import type { Email } from '@/types';
import { ThreadEmailItem } from './ThreadEmailItem';

const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  sender: 'Bea',
  senderEmail: 'bea@example.com',
  recipients: ['me@example.com'],
  cc: ['carol@example.com'],
  snippet: '',
  body: 'body',
  isSent: false,
  // 14 November 2023.
  timestamp: 1_700_000_000,
} as unknown as Email;

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

async function mount(isExpanded: boolean) {
  await act(async () => {
    root.render(
      <ThreadEmailItem
        email={email}
        isExpanded={isExpanded}
        isLast
        highlightQuery={null}
        onToggle={() => {}}
        onOpenAttachment={() => {}}
      />,
    );
  });
}

describe('ThreadEmailItem in Spanish', () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = () => {};
  });

  it('writes the collapsed date in the UI language', async () => {
    await mount(false);
    expect(container.textContent).toContain('14 nov');
    expect(container.textContent).toContain('inbox:emailView.noPreview');
  });

  it('writes the full date in the UI language', async () => {
    await mount(true);
    expect(container.textContent).toMatch(/14 nov 2023/);
    expect(container.textContent).not.toContain('Nov');
  });

  it('labels the recipients through i18n', async () => {
    await mount(true);
    expect(container.textContent).toContain('inbox:emailView.toLine');
    expect(container.textContent).toContain('inbox:emailView.ccLine');
  });
});
