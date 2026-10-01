// Multi-selection in the email list: Ctrl/⌘+click and Shift+click select
// several emails, a bar offers Delete / Move for all of them, Delete deletes
// the selection, Escape clears it, and dragging a selected row carries the
// whole selection to a folder.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { count?: number }) => (opts?.count !== undefined ? `${key}:${opts.count}` : key),
    i18n: { language: 'en' },
  }),
}));

const api = vi.hoisted(() => ({
  currentPlatform: vi.fn(() => 'linux'),
  getEmailTagsBatch: vi.fn(async () => ({})),
  getJunkConfig: vi.fn(async () => null),
  getJunkVerdicts: vi.fn(async () => []),
  setJunkConfig: vi.fn(async () => {}),
  setJunkFeedback: vi.fn(async () => {}),
  getThread: vi.fn(async (_a: string, threadId: string) => [{ id: `${threadId}-msg`, accountId: 'a1', threadId }]),
}));
vi.mock('@/lib/api', () => api);
vi.mock('./InboxSearchBox', () => ({ InboxSearchBox: () => null }));

// Render every row (jsdom has no layout, so the real virtualizer shows none).
vi.mock('./VirtualEmailList', () => ({
  VirtualEmailList: (props: {
    emails: Email[];
    onRowClick?: (email: Email, e: React.MouseEvent) => void;
    multiSelectedIds?: ReadonlySet<string>;
    multiSelectedEmails?: Email[];
    scrollContainerRef: React.RefObject<HTMLDivElement>;
  }) => (
    <div ref={props.scrollContainerRef}>
      {props.emails.map((email) => (
        <EmailRow
          key={email.id}
          email={email}
          isSelected={false}
          onClick={(e) => props.onRowClick?.(email, e as React.MouseEvent)}
          isMultiSelected={props.multiSelectedIds?.has(email.id) ?? false}
          dragCompanions={props.multiSelectedEmails}
        />
      ))}
    </div>
  ),
}));

const deleteEmail = vi.hoisted(() => vi.fn(async (_accountId: string, _emailId: string) => {}));
const moveEmail = vi.hoisted(() => vi.fn(async (_accountId: string, _emailId: string, _target: string) => {}));

import { EMAIL_DRAG_MIME } from '@/lib/emailDrag';
import { useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import type { Account, Email, EmailCategory } from '@/types';
import { EmailRow } from './EmailRow';
import { Inbox } from './Inbox';

const emails = ['e1', 'e2', 'e3', 'e4'].map(
  (id, i) =>
    ({
      id,
      threadId: `t${i + 1}`,
      accountId: 'a1',
      mailbox: 'inbox',
      category: 'primary',
      isRead: true,
      subject: `Subject ${id}`,
      sender: `Sender ${id}`,
      senderEmail: `${id}@example.com`,
      timestamp: 1_700_000_000 - i,
      recipients: [],
      cc: [],
    }) as unknown as Email,
);

let container: HTMLDivElement;
let root: Root;
let onSelect: ReturnType<typeof vi.fn<(email: Email, opts?: { auto?: boolean }) => void>>;

beforeEach(async () => {
  deleteEmail.mockClear();
  moveEmail.mockClear();
  api.getThread.mockClear();
  useEmailStore.setState({ deleteEmail, moveEmail } as never);
  useAccountStore.setState({ accounts: [{ id: 'a1', email: 'me@example.com', provider: 'imap' } as Account] } as never);
  onSelect = vi.fn();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(
      <Inbox
        emails={emails}
        isLoading={false}
        isSyncing={false}
        syncProgress={null}
        isLoadingMore={false}
        hasMore={false}
        totalCount={emails.length}
        selectedEmailId="e1"
        onSelectEmail={onSelect}
        onLoadMore={() => {}}
        selectedCategories={new Set<EmailCategory>()}
        onSelectCategories={() => {}}
      />,
    );
  });
  onSelect.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function row(id: string): HTMLElement {
  const el = Array.from(container.querySelectorAll<HTMLElement>('[role="button"]')).find((r) =>
    r.textContent?.includes(`Subject ${id}`),
  );
  if (!el) throw new Error(`row ${id} not rendered`);
  return el;
}

async function click(id: string, mods: { ctrlKey?: boolean; shiftKey?: boolean } = {}) {
  await act(async () => {
    row(id).dispatchEvent(new MouseEvent('click', { bubbles: true, ...mods }));
  });
}

async function key(k: string) {
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
  });
}

function bar(): HTMLElement | null {
  return container.querySelector('[role="toolbar"]');
}

function selected(): string[] {
  return ['e1', 'e2', 'e3', 'e4'].filter((id) => row(id).getAttribute('aria-pressed') === 'true');
}

describe('Inbox multi-selection', () => {
  it('a plain click still just opens the email', async () => {
    await click('e3');
    expect(onSelect).toHaveBeenCalledWith(emails[2]);
    expect(bar()).toBeNull();
  });

  it('Ctrl+click selects the open email and the clicked one, and shows the bar', async () => {
    await click('e3', { ctrlKey: true });
    expect(onSelect).not.toHaveBeenCalled();
    expect(selected()).toEqual(['e1', 'e3']);
    expect(bar()?.textContent).toContain('inbox:multiSelect.selected:2');
  });

  it('Shift+click selects a range', async () => {
    await click('e3', { shiftKey: true });
    expect(selected()).toEqual(['e1', 'e2', 'e3']);
  });

  it('Escape clears the selection', async () => {
    await click('e2', { ctrlKey: true });
    await key('Escape');
    expect(bar()).toBeNull();
    expect(selected()).toEqual([]);
  });

  it('Delete moves every selected thread to the Trash', async () => {
    await click('e3', { ctrlKey: true });
    await key('Delete');
    await act(async () => {});
    expect(deleteEmail.mock.calls.map((c) => c[1])).toEqual(['t1-msg', 't3-msg']);
    expect(bar()).toBeNull();
  });

  it('the bar Delete button does the same', async () => {
    await click('e2', { shiftKey: true });
    const del = Array.from(bar()?.querySelectorAll('button') ?? []).find(
      (b) => b.textContent === 'inbox:multiSelect.delete',
    );
    await act(async () => {
      del?.click();
    });
    expect(deleteEmail).toHaveBeenCalledTimes(2);
  });

  it('dragging a selected row carries the whole selection', async () => {
    await click('e3', { ctrlKey: true });
    const data: Record<string, string> = {};
    const dt = { setData: (t: string, v: string) => (data[t] = v), setDragImage: vi.fn(), effectAllowed: '' };
    const event = new Event('dragstart', { bubbles: true }) as Event & { dataTransfer: unknown };
    event.dataTransfer = dt;
    await act(async () => {
      row('e3').dispatchEvent(event);
    });
    const payload = JSON.parse(data[EMAIL_DRAG_MIME]);
    expect(payload.emailId).toBe('e3');
    expect(payload.extra.map((x: { emailId: string }) => x.emailId)).toEqual(['e1']);
    expect((dt.setDragImage.mock.calls[0][0] as HTMLElement).textContent).toContain('2');
  });

  it('dragging an unselected row moves only that row', async () => {
    await click('e3', { ctrlKey: true });
    const data: Record<string, string> = {};
    const dt = { setData: (t: string, v: string) => (data[t] = v), setDragImage: vi.fn(), effectAllowed: '' };
    const event = new Event('dragstart', { bubbles: true }) as Event & { dataTransfer: unknown };
    event.dataTransfer = dt;
    await act(async () => {
      row('e4').dispatchEvent(event);
    });
    expect(JSON.parse(data[EMAIL_DRAG_MIME]).extra).toBeUndefined();
  });

  it('the ✓ button selects all, then deselects all, and the bar stays open', async () => {
    await click('e3', { ctrlKey: true });
    const toggle = () => bar()?.querySelector<HTMLButtonElement>('button[aria-pressed]');
    expect(toggle()?.getAttribute('aria-label')).toBe('inbox:multiSelect.selectAll');

    await act(async () => toggle()?.click());
    expect(selected()).toEqual(['e1', 'e2', 'e3', 'e4']);
    expect(toggle()?.getAttribute('aria-pressed')).toBe('true');
    expect(toggle()?.getAttribute('aria-label')).toBe('inbox:multiSelect.deselectAll');

    await act(async () => toggle()?.click());
    expect(selected()).toEqual([]);
    expect(bar()).not.toBeNull();
    expect(bar()?.textContent).toContain('inbox:multiSelect.noneSelected');
  });

  it('while the bar is open, a plain click selects instead of opening', async () => {
    await click('e3', { ctrlKey: true });
    await click('e4');
    expect(onSelect).not.toHaveBeenCalled();
    expect(selected()).toEqual(['e1', 'e3', 'e4']);
    await click('e1');
    expect(selected()).toEqual(['e3', 'e4']);
  });

  it('Delete with nothing ticked does nothing, even with an email open', async () => {
    await click('e3', { ctrlKey: true });
    const toggle = bar()?.querySelector<HTMLButtonElement>('button[aria-pressed]');
    await act(async () => toggle?.click());
    await act(async () => toggle?.click());
    await key('Delete');
    expect(deleteEmail).not.toHaveBeenCalled();
    const del = Array.from(bar()?.querySelectorAll('button') ?? []).find(
      (b) => b.textContent === 'inbox:multiSelect.delete',
    );
    expect(del?.disabled).toBe(true);
  });

  it('✕ leaves the selection mode, and clicks open emails again', async () => {
    await click('e3', { ctrlKey: true });
    const exit = bar()?.querySelector<HTMLButtonElement>('button[aria-label="inbox:multiSelect.clear"]');
    await act(async () => exit?.click());
    expect(bar()).toBeNull();
    await click('e2');
    expect(onSelect).toHaveBeenCalledWith(emails[1]);
  });
});
