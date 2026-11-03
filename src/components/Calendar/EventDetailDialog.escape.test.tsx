// The event detail closes like every other dialog: on Escape, and only when
// nothing is open on top of it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '10:00', date: () => '28/07/2026', dateTime: () => '28/07/2026 10:00' }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));
vi.mock('@/lib/api', () => ({ deleteCalendarEvent: vi.fn(), currentPlatform: vi.fn(() => '') }));

import { Modal } from '@/components/common/Modal';
import type { CalendarEvent } from '@/types';
import { EventDetailDialog } from './EventDetailDialog';

const EVENT = {
  id: 'acc:cal:e1',
  accountId: 'acc',
  calendarId: 'cal',
  title: 'Standup',
  description: '',
  location: '',
  startTime: 1_785_312_000,
  endTime: 1_785_315_600,
  isAllDay: false,
  organizer: '',
  attendees: [],
  meetingLink: null,
  htmlLink: null,
  status: 'confirmed',
} as unknown as CalendarEvent;

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

const pressEscape = () =>
  act(async () => {
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  });

describe('EventDetailDialog — Escape', () => {
  it('closes, though the window consumes the key', async () => {
    // The app-wide shortcut handler registers first and consumes every Escape
    // that reaches the window.
    const consume = (e: KeyboardEvent) => e.preventDefault();
    window.addEventListener('keydown', consume);
    const onClose = vi.fn();
    await act(async () =>
      root.render(
        <EventDetailDialog
          event={EVENT}
          accountId="acc"
          provider="gmail"
          onClose={onClose}
          onDeleted={() => {}}
          onAuthError={() => {}}
        />,
      ),
    );

    await pressEscape();
    window.removeEventListener('keydown', consume);

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('leaves Escape to a dialog open on top of it', async () => {
    const onClose = vi.fn();
    const closeTop = vi.fn();
    await act(async () =>
      root.render(
        <>
          <EventDetailDialog
            event={EVENT}
            accountId="acc"
            provider="gmail"
            onClose={onClose}
            onDeleted={() => {}}
            onAuthError={() => {}}
          />
          <Modal open onClose={closeTop} title="On top">
            <p>Body</p>
          </Modal>
        </>,
      ),
    );

    await pressEscape();

    expect(closeTop).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
  });
});
