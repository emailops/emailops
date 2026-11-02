// The warning on a row follows the verdict, so mail the detector flagged shows
// it even when no `junk` tag was written or the tag load failed.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { initI18n } from '@/i18n';
import { useJunkStore } from '@/stores/junkStore';
import { useTagStore } from '@/stores/tagStore';
import type { Email, EmailTag, JunkVerdict } from '@/types';
import { EmailRow } from './EmailRow';

const email = {
  id: 'email-1',
  accountId: 'acct-1',
  threadId: 'thread-1',
  subject: 'Quarterly report',
  sender: 'Ada Example',
  senderEmail: 'ada@example.com',
  recipients: [],
  cc: [],
  body: '',
  snippet: '',
  timestamp: 1_700_000_000,
  isRead: true,
  category: 'primary',
  mailbox: 'inbox',
} as unknown as Email;

const flagged = { emailId: 'email-1', band: 'junk', primaryKind: 'phishing', userOverride: null } as JunkVerdict;
const junkTag: EmailTag = { emailId: 'email-1', tagType: 'junk', tagValue: 'spam', confidence: null, createdAt: 0 };

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useJunkStore.setState({ verdictsByEmail: {}, loaded: {} });
  useTagStore.setState({ tagsByEmail: {} });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function renderRow() {
  act(() => {
    root.render(<EmailRow email={email} isSelected={false} onClick={() => {}} compact />);
  });
  return {
    text: container.textContent ?? '',
    dimmed: container.querySelector('.opacity-55') !== null,
  };
}

describe('EmailRow junk warning', () => {
  it('shows a verdict that has no tag', () => {
    useJunkStore.setState({ verdictsByEmail: { 'email-1': flagged } });

    const row = renderRow();

    expect(row.text).toContain('phishing');
    expect(row.dimmed).toBe(true);
  });

  it('names a confirmed message spam when the detector called it legitimate', () => {
    useJunkStore.setState({
      verdictsByEmail: { 'email-1': { ...flagged, band: 'clean', primaryKind: 'legit', userOverride: 'junk' } },
    });

    expect(renderRow().text).toContain('spam');
  });

  it('prefers the tag when there is one', () => {
    useJunkStore.setState({ verdictsByEmail: { 'email-1': flagged } });
    useTagStore.setState({ tagsByEmail: { 'email-1': [junkTag] } });

    const row = renderRow();

    expect(row.text).toContain('spam');
    expect(row.text).not.toContain('phishing');
  });

  it('shows nothing once the user said it is not junk', () => {
    useJunkStore.setState({ verdictsByEmail: { 'email-1': { ...flagged, userOverride: 'not_junk' } } });
    useTagStore.setState({ tagsByEmail: { 'email-1': [junkTag] } });

    const row = renderRow();

    expect(row.text).not.toContain('spam');
    expect(row.dimmed).toBe(false);
  });

  it('leaves clean mail alone', () => {
    useJunkStore.setState({ verdictsByEmail: { 'email-1': { ...flagged, band: 'clean' } } });

    expect(renderRow().dimmed).toBe(false);
  });
});
