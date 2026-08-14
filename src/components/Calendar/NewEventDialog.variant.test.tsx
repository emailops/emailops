// The "New event" dialog is a light surface (white panel, gray-300 inputs), but
// the shared <Select> defaults to variant="dark" — the app's dark chrome. Every
// Select in this dialog must opt into the light variant, otherwise the start /
// end / repeats controls render as dark #333 pills on white — which is what
// shipped, spotted while setting up the OAuth demo recording. Both the native
// (macOS/Windows) and owned-popup (Linux) branches are affected, since both
// apply VARIANT_CLASSES[variant].trigger to the trigger element.

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
        accountId="acct-1"
        isGmail
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
  it('renders every dropdown on the light variant, not the dark chrome one', () => {
    renderDialog();

    const triggers = Array.from(container.querySelectorAll('[aria-haspopup="listbox"]'));
    // start time, end time, repeats
    expect(triggers).toHaveLength(3);

    for (const trigger of triggers) {
      const label = trigger.getAttribute('aria-label');
      expect(trigger.className, `${label} should use the light trigger`).toContain('bg-white');
      expect(trigger.className, `${label} should not use the dark chrome trigger`).not.toContain('bg-[#333]');
    }
  });
});
