// The "New event" dialog's dropdowns once rendered as dark #333 pills on its
// white panel, because the shared <Select> defaulted to a dark-chrome palette.
// There is one palette now; this keeps the dialog's start / end / repeats
// controls on it in the owned-popup (Linux) branch.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '00:00', date: () => '', dateTime: () => '' }),
}));
vi.mock('@/lib/api', () => ({
  createCalendarEvent: vi.fn(),
  searchRecipients: vi.fn(async () => []),
  // Owned-popup branch — the one Linux takes, and the one that renders a
  // <button> trigger rather than a native <select>.
  currentPlatform: vi.fn(() => ''),
}));

import { NewEventDialog } from './NewEventDialog';

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
  vi.clearAllMocks();
});

function renderDialog() {
  const start = Math.floor(new Date(2026, 7, 18, 5, 0).getTime() / 1000);
  act(() => {
    root.render(
      <NewEventDialog
        choices={[
          {
            account: {
              id: 'acct-1',
              provider: 'gmail',
              email: 'a@mail.example',
              name: '',
              createdAt: 0,
              sortOrder: 0,
              enabled: true,
              syncFromTimestamp: null,
              color: 'blue',
            },
            plan: { kind: 'dialog' },
          },
        ]}
        initialStart={start}
        initialEnd={start + 3600}
        onClose={() => {}}
        onCreated={() => {}}
        onAuthError={() => {}}
      />,
    );
  });
}

describe('NewEventDialog select styling', () => {
  it('renders every dropdown on the inbox palette', () => {
    renderDialog();

    const triggers = Array.from(container.querySelectorAll('[aria-haspopup="listbox"]'));
    // start time, end time, repeats
    expect(triggers).toHaveLength(3);

    for (const trigger of triggers) {
      const label = trigger.getAttribute('aria-label');
      expect(trigger.className, `${label} should use the inbox trigger`).toContain('bg-white');
      expect(trigger.className, `${label} should not use a dark trigger`).not.toContain('bg-[#333]');
    }
  });
});
