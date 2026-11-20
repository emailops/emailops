// The invite card above an email: answer buttons for a real invitation, a
// single "Add to calendar" for a publish, who the reply goes to, and what each
// backend outcome looks like.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import en from '@/locales/en/calendar.json';

// Real English texts, so the assertions read like what the user sees.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, string>) => {
      const found = key
        .replace(/^calendar:/, '')
        .split('.')
        .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], en);
      const text = typeof found === 'string' ? found : (opts?.defaultValue ?? key);
      return text.replace(/\{\{(\w+)\}\}/g, (_, name: string) => opts?.[name] ?? '');
    },
    i18n: { language: 'en' },
  }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '09:00', date: () => 'Tue 28 Jul', dateTime: () => 'Tue 28 Jul 09:00' }),
}));
const api = vi.hoisted(() => ({
  getCalendarInvite: vi.fn(),
  rsvpCalendarInvite: vi.fn(),
  addInviteToCalendar: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

import { useAccountStore } from '@/stores/accountStore';
import { useCalendarIntegrationStore } from '@/stores/calendarIntegrationStore';
import { useLogStore } from '@/stores/logStore';
import type { CalendarInvite, Email, InviteResponseOutcome } from '@/types';
import { CalendarInviteCard } from './CalendarInviteCard';

const email = { id: 'e1', accountId: 'acc1' } as Email;

function invite(over: Partial<CalendarInvite> = {}): CalendarInvite {
  return {
    uid: 'uid-1',
    summary: 'Weekly sync',
    location: '',
    organizer: 'lead@example.com',
    startTime: 1_785_000_000,
    endTime: 1_785_003_600,
    isAllDay: false,
    recurrence: null,
    method: 'REQUEST',
    isPublish: false,
    replyTo: 'lead@example.com',
    organizerIsSender: true,
    ...over,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useAccountStore.setState({ accounts: [{ id: 'acc1', provider: 'gmail' }] as never });
  useCalendarIntegrationStore.setState({ enabledIds: new Set(['acc1']), isLoaded: true });
  useLogStore.setState({ entries: [] });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function renderCard(inv: CalendarInvite) {
  api.getCalendarInvite.mockResolvedValue(inv);
  await act(async () => {
    root.render(<CalendarInviteCard email={email} />);
  });
}

const text = () => container.textContent ?? '';
const button = (label: string) =>
  [...container.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) as HTMLButtonElement;
const click = async (label: string) => {
  await act(async () => {
    button(label).click();
  });
};
const logs = () => useLogStore.getState().entries;

async function answerYes(outcome: InviteResponseOutcome, inv = invite()) {
  api.rsvpCalendarInvite.mockResolvedValue(outcome);
  await renderCard(inv);
  await click('Yes');
}

describe('CalendarInviteCard', () => {
  it('publish_invite_shows_add_to_calendar_only', async () => {
    api.addInviteToCalendar.mockResolvedValue({ kind: 'imported', replySentTo: null, replyError: null });
    await renderCard(invite({ isPublish: true, replyTo: null }));
    expect(button('Yes')).toBeUndefined();
    expect(button('No')).toBeUndefined();
    expect(button('Maybe')).toBeUndefined();
    expect(button('Add to calendar')).toBeDefined();
    await click('Add to calendar');
    expect(api.addInviteToCalendar).toHaveBeenCalledWith('acc1', 'e1', expect.any(String));
    expect(text()).toContain('Added to your calendar');
  });

  it('publish_already_in_calendar', async () => {
    api.addInviteToCalendar.mockResolvedValue({ kind: 'alreadyInCalendar' });
    await renderCard(invite({ isPublish: true, replyTo: null }));
    await click('Add to calendar');
    expect(text()).toContain('Already in your calendar');
  });

  it('shows_who_the_reply_goes_to', async () => {
    await renderCard(invite());
    expect(text()).toContain('Your reply will be sent to lead@example.com');
  });

  it('warns_when_organizer_is_not_the_sender', async () => {
    await renderCard(invite({ organizerIsSender: false }));
    const line = [...container.querySelectorAll('p')].find((p) => p.textContent?.includes('lead@example.com, who'));
    expect(line?.textContent).toContain("who isn't the sender of this email");
    expect(line?.className).toContain('text-amber-700');
  });

  it('no_reply_line_when_reply_to_is_null', async () => {
    await renderCard(invite({ replyTo: null }));
    expect(text()).not.toContain('Your reply will be sent');
    expect(button('Yes')).toBeDefined();
  });

  it('imported_outcome_confirms_and_mentions_calendar', async () => {
    await answerYes({ kind: 'imported', replySentTo: 'lead@example.com', replyError: null });
    expect(text()).toContain('You accepted this invitation.');
    expect(text()).toContain('Added to your calendar');
  });

  it('imported_with_reply_error_shows_amber_note_and_logs', async () => {
    await answerYes({ kind: 'imported', replySentTo: null, replyError: 'smtp down' });
    const note = [...container.querySelectorAll('p')].find((p) => p.textContent?.includes("couldn't be notified"));
    expect(note?.textContent).toBe(
      "Added to your calendar, but the organizer couldn't be notified. Answer again to retry.",
    );
    expect(note?.className).toContain('text-amber-700');
    expect(logs().some((l) => l.level === 'error' && l.source === 'sync' && l.message.includes('smtp down'))).toBe(
      true,
    );
  });

  it('removed_outcome_confirms_decline', async () => {
    api.rsvpCalendarInvite.mockResolvedValue({ kind: 'removed', replySentTo: 'lead@example.com', replyError: null });
    await renderCard(invite());
    await click('No');
    expect(text()).toContain('You declined this invitation.');
    expect(text()).not.toContain("couldn't be notified");
  });

  it('removed_with_reply_error_shows_amber_note', async () => {
    api.rsvpCalendarInvite.mockResolvedValue({ kind: 'removed', replySentTo: null, replyError: 'smtp down' });
    await renderCard(invite());
    await click('No');
    expect(text()).toContain(
      "Removed from your calendar, but the organizer couldn't be notified. Answer again to retry.",
    );
    expect(logs().some((l) => l.level === 'error' && l.message.includes('smtp down'))).toBe(true);
  });

  it('already_in_calendar_outcome_is_named', async () => {
    await answerYes({ kind: 'alreadyInCalendar' });
    expect(text()).toContain('Already in your calendar');
  });

  it('not_yet_in_calendar_shows_the_info_note', async () => {
    await answerYes({ kind: 'notYetInCalendar' });
    expect(text()).toContain("hasn't reached your calendar yet");
    expect(button('Yes')).toBeDefined();
  });

  it('cannot_import_shows_the_reason', async () => {
    await answerYes({ kind: 'cannotImport', reason: 'outlookRecurrence', reasonDetail: 'FREQ=WEEKLY' });
    expect(text()).toContain("Couldn't add it to your calendar: Outlook can't repeat this way (FREQ=WEEKLY)");
    expect(text()).not.toContain("Couldn't send your response");
  });

  it('change_reopens_the_buttons', async () => {
    await answerYes({ kind: 'responded', replySentTo: 'lead@example.com' });
    expect(text()).toContain('You accepted this invitation.');
    await click('Change');
    expect(button('Yes')).toBeDefined();
  });
});
