// Regression: the preview inherited its background from the host. Tasks and
// Memory host it on a light panel; the Lens row drawer is dark, so the
// subject and sender (dark text) disappeared on it while the email body,
// drawn in its own white iframe, stayed readable.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

// Capture what the frame is asked to render — the sanitised HTML is the
// behaviour under test, not the iframe plumbing.
vi.mock('@/components/shared/EmailHtmlFrame', () => ({
  EmailHtmlFrame: ({ html }: { html: string }) => <div data-testid="frame" data-html={html} />,
}));

vi.mock('@/lib/api', () => ({
  getEmailById: vi.fn(),
  getEmailBody: vi.fn(),
  getPref: vi.fn().mockResolvedValue(null),
  isSenderTrusted: vi.fn().mockResolvedValue(false),
}));

import * as api from '@/lib/api';
import type { Email } from '@/types';
import { EmailPreviewById } from './EmailPreviewById';

const EMAIL: Email = {
  id: 'e1',
  accountId: 'acct-1',
  threadId: 't1',
  subject: 'Quarterly newsletter',
  sender: 'Newsletter',
  senderEmail: 'news@sender.example.test',
  recipients: [],
  cc: [],
  body: '',
  snippet: 'Hello',
  timestamp: 1_700_000_000,
  isRead: false,
  triageStatus: null,
  category: 'primary',
  mailbox: 'inbox',
  isSent: false,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.mocked(api.getEmailById).mockResolvedValue(EMAIL);
  vi.mocked(api.getEmailBody).mockResolvedValue('<p>Hello</p>');
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function mount(email: Email = EMAIL) {
  vi.mocked(api.getEmailById).mockResolvedValue(email);
  await act(async () => {
    root.render(<EmailPreviewById accountId="acct-1" emailId="e1" emptyMessage="none" />);
  });
  await act(async () => {
    await Promise.resolve();
  });
}

describe('EmailPreviewById surface', () => {
  it('paints its own light surface so the header is readable on any host', async () => {
    await mount();
    const view = container.firstElementChild as HTMLElement;
    expect(view.textContent).toContain('Quarterly newsletter');
    expect(view.className).toContain('bg-white');
  });

  it('localizes the placeholder for an email without a subject', async () => {
    await mount({ ...EMAIL, subject: '' } as Email);
    expect(container.querySelector('h2')?.textContent).toBe('common:labels.noSubject');
  });
});
