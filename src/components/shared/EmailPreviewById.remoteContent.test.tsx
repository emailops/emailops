// Regression: the shared email preview rendered mail through
// `sanitizeEmailHtml`, which only sanitises inline `style` — it never installs
// the hook that strips remote `src`/`poster`/`srcset`. `EmailBody` used
// `sanitizeEmailHtmlFull` and honoured `privacy.allow_remote_content` plus the
// trusted-sender allowlist; the preview honoured neither.
//
// Three surfaces render mail through this component (Tasks, Memory, the Lens
// row drawer), and none of them shows the "load images?" banner — so with the
// preference at its default OFF, opening a task extracted from a marketing
// email fired its tracking pixel with no signal to the user at all.

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
  getPref: vi.fn(),
  isSenderTrusted: vi.fn(),
}));

import * as api from '@/lib/api';
import type { Email } from '@/types';
import { EmailPreviewById } from './EmailPreviewById';

const TRACKER = 'https://tracker.example.test/pixel.gif';
const BODY_HTML = `<p>Hello</p><img src="${TRACKER}" width="1" height="1">`;

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
  vi.mocked(api.getEmailBody).mockResolvedValue(BODY_HTML);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => {
    root.render(<EmailPreviewById accountId="acct-1" emailId="e1" emptyMessage="none" />);
  });
  // Let the email fetch, the preference read and the trust check settle.
  await act(async () => {
    await Promise.resolve();
  });
}

function renderedHtml(): string {
  const frame = container.querySelector('[data-testid="frame"]');
  return frame?.getAttribute('data-html') ?? '';
}

describe('EmailPreviewById remote content', () => {
  it('strips remote image sources when the preference is unset (default off)', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    vi.mocked(api.isSenderTrusted).mockResolvedValue(false);

    await mount();

    expect(renderedHtml()).not.toContain(TRACKER);
  });

  it('strips remote image sources when the preference is explicitly off', async () => {
    vi.mocked(api.getPref).mockResolvedValue('false');
    vi.mocked(api.isSenderTrusted).mockResolvedValue(false);

    await mount();

    expect(renderedHtml()).not.toContain(TRACKER);
  });

  it('keeps remote images when the user has allowed remote content', async () => {
    vi.mocked(api.getPref).mockResolvedValue('true');
    vi.mocked(api.isSenderTrusted).mockResolvedValue(false);

    await mount();

    expect(renderedHtml()).toContain(TRACKER);
  });

  it('keeps remote images for a sender the user has trusted', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    vi.mocked(api.isSenderTrusted).mockResolvedValue(true);

    await mount();

    expect(renderedHtml()).toContain(TRACKER);
  });

  it('checks the allowlist for the sender of the previewed email', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    vi.mocked(api.isSenderTrusted).mockResolvedValue(false);

    await mount();

    expect(api.isSenderTrusted).toHaveBeenCalledWith('acct-1', EMAIL.senderEmail);
  });
});
