// Regression: the Add IMAP Account form had ONE field for TWO concepts — the
// account's own address and the SASL login sent to the server. That field was
// `type="email"` and `required`, so an account whose server login is a bare
// name (`alex`, not `alex@example.de`) could not be added at all: "Test
// Connection" succeeded (it is a `type="button"`), and then submitting failed
// silent native validation.
//
// Address and login are now separate inputs. The address is validated and
// becomes `accounts.email`; the login is free text and defaults to the address.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AddImapAccountModal } from './AddImapAccountModal';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const api = vi.hoisted(() => ({
  addImapAccount: vi.fn(),
  testImapConnection: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  api.addImapAccount.mockReset().mockResolvedValue({ id: 'acct-1', email: 'alex@example.de' });
  api.testImapConnection.mockReset().mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function field(id: string): HTMLInputElement {
  const input = container.querySelector<HTMLInputElement>(`#${id}`);
  if (!input) throw new Error(`input #${id} not rendered`);
  return input;
}

function typeInto(input: HTMLInputElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  act(() => {
    setValue?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

/** Render and fill every field the form needs. `username` is left untouched when omitted. */
function fillForm({ email, username }: { email: string; username?: string }) {
  act(() => {
    root.render(<AddImapAccountModal onSuccess={() => {}} onCancel={() => {}} />);
  });
  typeInto(field('add-imap-email'), email);
  if (username !== undefined) typeInto(field('add-imap-username'), username);
  typeInto(field('add-imap-password'), 'app-password');
  typeInto(field('add-imap-host'), 'imap.example.com');
  typeInto(field('add-imap-smtp-host'), 'smtp.example.com');
}

function clickButton(label: string) {
  const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes(label));
  if (!button) throw new Error(`button "${label}" not rendered`);
  return act(async () => {
    button.click();
  });
}

describe('AddImapAccountModal address and login', () => {
  it('does not block submission when the server login is not an email address', () => {
    fillForm({ email: 'alex@example.de', username: 'alex' });

    const form = container.querySelector('form');
    expect(form).not.toBeNull();
    expect(field('add-imap-username').type).toBe('text');
    expect(field('add-imap-username').checkValidity()).toBe(true);
    expect(form?.checkValidity()).toBe(true);
  });

  it('sends the typed address as email and the typed login as username', async () => {
    fillForm({ email: 'alex@example.de', username: 'alex' });

    await clickButton('Test Connection');
    await clickButton('Add Account');

    expect(api.testImapConnection).toHaveBeenCalledWith(expect.objectContaining({ username: 'alex' }));
    expect(api.addImapAccount).toHaveBeenCalledWith(
      expect.objectContaining({ email: 'alex@example.de', username: 'alex' }),
    );
  });

  it('falls back to the address as the login when the username is left blank', async () => {
    fillForm({ email: 'alex@example.de' });

    await clickButton('Test Connection');
    await clickButton('Add Account');

    expect(api.addImapAccount).toHaveBeenCalledWith(
      expect.objectContaining({ email: 'alex@example.de', username: 'alex@example.de' }),
    );
  });
});
