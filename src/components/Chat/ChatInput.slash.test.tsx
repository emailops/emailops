// Typing `/` in the chat offers the user's enabled skills; picking one fills
// in `/name ` instead of sending the half-typed message.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SkillsOverview } from '@/lib/api';
import { useSkillsEnabledStore } from '@/stores/featureToggleStore';

const listSkills = vi.fn<() => Promise<SkillsOverview>>();
vi.mock('@/lib/api', async (orig) => ({
  ...(await orig<typeof import('@/lib/api')>()),
  listSkills: () => listSkills(),
}));

import { ChatInput } from './ChatInput';

let container: HTMLDivElement;
let root: Root;
const onSend = vi.fn();

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
  listSkills.mockResolvedValue({
    enabled: true,
    dir: '/d',
    skills: [
      { name: 'weekly-report', description: 'Weekly recap.', path: '', enabled: true },
      { name: 'weekly-digest', description: 'Off.', path: '', enabled: false },
      { name: 'trip-brief', description: 'Trips.', path: '', enabled: true },
    ],
    errors: [],
  });
  act(() => useSkillsEnabledStore.setState({ enabled: true }));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  act(() => useSkillsEnabledStore.setState({ enabled: false }));
});

async function renderInput() {
  await act(async () => {
    root.render(<ChatInput onSend={onSend} disabled={false} />);
  });
  const textarea = container.querySelector('textarea');
  if (!textarea) throw new Error('textarea not rendered');
  return textarea;
}

async function typeInto(textarea: HTMLTextAreaElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(textarea, text);
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function key(textarea: HTMLTextAreaElement, k: string) {
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
  });
}

const options = () =>
  Array.from(container.querySelectorAll('[data-testid^="slash-option-"]')).map((e) => e.textContent);

describe('ChatInput slash suggestions', () => {
  it('offers the enabled skills that match what is typed', async () => {
    const textarea = await renderInput();
    await typeInto(textarea, '/wee');
    expect(options()).toHaveLength(1);
    expect(options()[0]).toContain('/weekly-report');
    expect(options()[0]).toContain('Weekly recap.');
  });

  it('Enter picks the highlighted skill instead of sending', async () => {
    const textarea = await renderInput();
    await typeInto(textarea, '/');
    await key(textarea, 'ArrowDown');
    await key(textarea, 'Enter');
    expect(onSend).not.toHaveBeenCalled();
    expect(textarea.value).toBe('/weekly-report ');
    expect(options()).toHaveLength(0);
  });

  it('Escape closes the list and Enter then sends', async () => {
    const textarea = await renderInput();
    await typeInto(textarea, '/tri');
    await key(textarea, 'Escape');
    expect(options()).toHaveLength(0);
    await key(textarea, 'Enter');
    expect(onSend).toHaveBeenCalledWith('/tri');
  });

  it('stays out of the way while skills are off', async () => {
    act(() => useSkillsEnabledStore.setState({ enabled: false }));
    const textarea = await renderInput();
    await typeInto(textarea, '/wee');
    expect(options()).toHaveLength(0);
    expect(listSkills).not.toHaveBeenCalled();
  });
});
