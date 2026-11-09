// A row for a message the user sent names its recipients ("To: Ana, Luis +1")
// instead of repeating the user's own name, in every list and both layouts.
// The decisions themselves (who is "own", order, dedupe, names) are unit-tested
// on `rowParty`; this file checks the row wires and words them.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { useAccountStore } from '@/stores/accountStore';
import { useFolderStore } from '@/stores/folderStore';
import { useSelectionStore } from '@/stores/selectionStore';
import type { Account, Email } from '@/types';
import { EmailRow } from './EmailRow';

const OWNER = 'me@example.com';

const sent: Email = {
  id: 'email-1',
  accountId: 'acct-1',
  threadId: 'thread-1',
  messageId: null,
  subject: 'Proposal',
  sender: 'Sam Rivera',
  senderEmail: OWNER,
  recipients: ['Ana <ana@example.com>', 'luis@example.org'],
  cc: ['carla@example.net'],
  body: 'body',
  snippet: 'snippet',
  timestamp: 1_700_000_000,
  isRead: true,
  triageStatus: null,
  category: 'primary',
  mailbox: 'sent',
  isSent: true,
  isStarred: false,
};

const received: Email = { ...sent, sender: 'Ana', senderEmail: 'ana@example.com', mailbox: 'inbox', isSent: false };

/** How luis@example.org signs when he writes to acct-1. */
const nameOf = (accountId: string, address: string) =>
  accountId === 'acct-1' && address === 'luis@example.org' ? 'Luis Gil' : undefined;

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('es');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function renderRow(
  email: Email,
  opts: { compact?: boolean; onAddSenderFilter?: (a: string) => void; onHide?: (a: string) => void } = {},
) {
  act(() => {
    root.render(
      <EmailRow
        email={email}
        isSelected={false}
        onClick={() => {}}
        compact={opts.compact ?? false}
        ownerEmail={OWNER}
        nameOf={nameOf}
        onAddSenderFilter={opts.onAddSenderFilter ?? (() => {})}
        onHideSenderFromFilters={opts.onHide ?? (() => {})}
      />,
    );
  });
}

function party() {
  const cell = container.querySelector<HTMLElement>('[data-testid="row-party"]');
  if (!cell) throw new Error('party cell not rendered');
  return {
    cell,
    names: cell.querySelector('[data-testid="row-party-names"]')?.textContent ?? cell.textContent,
    more: cell.querySelector<HTMLElement>('[data-testid="row-party-more"]'),
  };
}

function openMenu(): HTMLElement {
  const button = container.querySelector<HTMLButtonElement>('button[aria-haspopup], button[title="Más acciones"]');
  if (!button) throw new Error('kebab button not rendered');
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
  const dropdown = document.body.querySelector<HTMLElement>('div.fixed');
  if (!dropdown) throw new Error('dropdown not rendered');
  return dropdown;
}

describe('EmailRow recipients of an own message', () => {
  it('names two recipients and counts the rest in the split row', () => {
    renderRow(sent);
    const { cell, names, more } = party();
    expect(names).toBe('Para: Ana, Luis Gil');
    expect(more?.textContent).toBe('+1');
    expect(cell.querySelector('.sr-only')?.textContent).toBe(' y 1 más');
  });

  it('names one recipient in the compact row', () => {
    renderRow(sent, { compact: true });
    const { names, more } = party();
    expect(names).toBe('Para: Ana');
    expect(more?.textContent).toBe('+2');
  });

  it.each([
    ['split', false],
    ['compact', true],
  ])('lists every recipient, To and Cc apart, in the %s tooltip', (_layout, compact) => {
    renderRow(sent, { compact });
    expect(party().cell.title).toBe('Para: Ana <ana@example.com>, Luis Gil <luis@example.org>\nCc: carla@example.net');
  });

  it('calls the user "ti", last', () => {
    renderRow({ ...sent, recipients: [OWNER, 'ana@example.com'], cc: [] });
    const { cell, names, more } = party();
    expect(names).toBe('Para: ana@example.com, ti');
    expect(more).toBeNull();
    expect(cell.title).toBe('Para: ti <me@example.com>, ana@example.com');
  });

  it('says so when no recipient is visible', () => {
    renderRow({ ...sent, recipients: ['undisclosed-recipients:;'], cc: [] });
    expect(party().names).toBe('Para: (sin destinatarios)');
  });

  it('draws the first recipient in the avatar', () => {
    renderRow(sent);
    expect(container.querySelector('[data-testid="row-avatar"]')?.textContent).toBe('A');
  });

  it('points the menu at the first recipient, without blocking them', () => {
    const onAdd = vi.fn();
    const onHide = vi.fn();
    renderRow(sent, { onAddSenderFilter: onAdd, onHide });
    const menu = openMenu();

    expect(menu.querySelector('[data-testid="menu-sender"]')?.textContent).toBe('ana@example.com');
    expect(menu.querySelector('[data-testid="menu-block-sender"]')).toBeNull();
    const add = [...menu.querySelectorAll('button')].find((b) =>
      b.textContent?.includes('Añadir destinatario como filtro inteligente'),
    );
    act(() => add?.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onAdd).toHaveBeenCalledWith('ana@example.com');

    const hide = [...openMenu().querySelectorAll('button')].find((b) =>
      b.textContent?.includes('Ocultar de los filtros inteligentes'),
    );
    act(() => hide?.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onHide).toHaveBeenCalledWith('ana@example.com');
  });

  it('reads "Para: ti" on a note to self and drops the sender items from the menu', () => {
    renderRow({ ...sent, mailbox: 'inbox', recipients: [OWNER], cc: [] });
    expect(party().names).toBe('Para: ti');
    expect(openMenu().querySelector('[data-testid="menu-sender"]')).toBeNull();
  });
});

describe('EmailRow of a received message', () => {
  it.each([
    ['split', false],
    ['compact', true],
  ])('names the sender, with a tooltip, in the %s row', (_layout, compact) => {
    renderRow(received, { compact });
    const { cell } = party();
    expect(cell.textContent).toBe('Ana');
    expect(cell.title).toBe('Ana');
  });

  it('keeps the sender of an Inbox message that forges the account address', () => {
    renderRow({ ...sent, mailbox: 'inbox', isSent: false });
    expect(party().cell.textContent).toBe('Sam Rivera');
  });

  it('names the recipients of the same message when the Sent view lists it', () => {
    act(() => {
      root.render(
        <EmailRow
          email={{ ...sent, mailbox: 'inbox', isSent: false }}
          isSelected={false}
          onClick={() => {}}
          ownerEmail={OWNER}
          nameOf={nameOf}
          inSentView
        />,
      );
    });
    expect(party().names).toBe('Para: Ana, Luis Gil');
  });

  it('keeps the sender items of the menu', () => {
    renderRow(received);
    const menu = openMenu();
    expect(menu.querySelector('[data-testid="menu-sender"]')?.textContent).toBe('ana@example.com');
    expect(menu.textContent).toContain('Añadir remitente como filtro inteligente');
    expect(menu.querySelector('[data-testid="menu-block-sender"]')).not.toBeNull();
  });
});

describe('EmailRow drag preview of an own message', () => {
  beforeEach(() => {
    // Dragging is offered for IMAP inbox and custom-folder messages.
    useAccountStore.setState({ accounts: [{ id: 'acct-1', email: OWNER, provider: 'imap' } as Account] } as never);
    useFolderStore.setState({ folders: [], accountId: 'acct-1' });
    useSelectionStore.getState().clear();
  });

  it('carries the same label as the row', () => {
    renderRow({ ...sent, mailbox: 'folder:Work' });
    const row = container.querySelector<HTMLElement>('[draggable="true"]');
    if (!row) throw new Error('row is not draggable');
    const setDragImage = vi.fn();
    const event = new Event('dragstart', { bubbles: true }) as Event & { dataTransfer: unknown };
    event.dataTransfer = { setData: () => {}, setDragImage, effectAllowed: 'none' };
    act(() => {
      row.dispatchEvent(event);
    });
    expect((setDragImage.mock.calls[0][0] as HTMLElement).textContent).toContain('Para: Ana, Luis Gil +1');
  });
});
