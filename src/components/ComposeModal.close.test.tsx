// The autosaved draft keeps the text of a message, not the files attached in
// this composer, which live here until the message is sent. Every way of
// closing it therefore asks first while it holds files, so a stray Escape or
// click outside cannot throw them away.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => <div data-testid="body">{value}</div>,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'req'),
}));

import type { Account } from '@/types';
import { ComposeModal } from './ComposeModal';

const account = { id: 'a1', email: 'me@example.com', name: 'Me', enabled: true } as Account;

let container: HTMLDivElement;
let root: Root;
const onClose = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  await act(async () => {
    root.render(<ComposeModal accounts={[account]} defaultAccountId="a1" onClose={onClose} />);
  });
}

async function attachFile() {
  const input = document.querySelector<HTMLInputElement>('input[type="file"]');
  if (!input) throw new Error('file input not rendered');
  const file = new File(['hello'], 'contract.pdf', { type: 'application/pdf' });
  Object.defineProperty(input, 'files', { value: [file], configurable: true });
  await act(async () => {
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await vi.waitFor(() => expect(document.body.textContent).toContain('contract.pdf'));
  await vi.waitFor(() => expect(document.body.textContent).not.toContain('compose:readingFiles'));
}

const pressEscape = () =>
  act(async () => {
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  });
const clickBackdrop = () => act(async () => document.querySelector<HTMLElement>('.bg-black\\/40')?.click());
const confirmation = () => document.querySelector<HTMLElement>('[data-testid="modal-overlay"]');

function button(label: string, scope: ParentNode = document.body): HTMLButtonElement {
  const found = [...scope.querySelectorAll('button')].find((b) => b.textContent === label);
  if (!found) throw new Error(`${label} not rendered`);
  return found;
}

describe('closing the composer', () => {
  it('closes at once when it holds no files: the draft has everything', async () => {
    await mount();
    await pressEscape();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it.each([
    ['Escape', pressEscape],
    ['a click outside', clickBackdrop],
    ['Cancel', () => act(async () => button('common:actions.cancel').click())],
  ])('asks before %s drops the attached files', async (_how, close) => {
    await mount();
    await attachFile();

    await close();

    expect(onClose).not.toHaveBeenCalled();
    expect(confirmation()?.textContent).toContain('compose:closeWithFiles.title');
  });

  it('keeps writing when the user says so, with the files still attached', async () => {
    await mount();
    await attachFile();
    await pressEscape();

    await act(async () => button('compose:closeWithFiles.keep').click());

    expect(confirmation()).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('contract.pdf');
  });

  it('closes when the user confirms', async () => {
    await mount();
    await attachFile();
    await pressEscape();

    await act(async () => button('common:actions.close', confirmation() as HTMLElement).click());

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Escape on the confirmation goes back to the composer, and no further', async () => {
    await mount();
    await attachFile();
    await pressEscape();

    await pressEscape();

    expect(confirmation()).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
  });
});
