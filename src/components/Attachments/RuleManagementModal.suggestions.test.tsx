// Suggested attachment rules inside RuleManagementModal: the user reviews a
// candidate in the regular (prefilled) rule form and confirms it by creating
// the rule, or dismisses it so it is never proposed again.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AttachmentRule, AttachmentRuleSuggestion } from '@/types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: vi.fn() }),
}));

vi.mock('@/lib/api', () => ({
  applyRuleRetroactively: vi.fn(async () => 3),
  countAttachmentsForRule: vi.fn(async () => 0),
}));

import * as api from '@/lib/api';
import { RuleManagementModal } from './RuleManagementModal';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const SUGGESTION: AttachmentRuleSuggestion = {
  id: 'sug-1',
  accountId: 'acc-1',
  name: 'Acme · invoice',
  senderEmailPattern: 'billing@acme.com',
  filenamePattern: 'Invoice_*.pdf',
  tags: ['invoice', 'acme'],
  emailCount: 4,
  firstSeen: 1_760_000_000,
  lastSeen: 1_770_000_000,
  sampleFilenames: ['Invoice_0042.pdf', 'Invoice_0041.pdf'],
  status: 'pending',
  createdAt: 1_770_000_000,
  updatedAt: 1_770_000_000,
};

function makeRule(name: string): AttachmentRule {
  return {
    id: 'rule-1',
    accountId: 'acc-1',
    name,
    senderEmailPattern: 'billing@acme.com',
    subjectPattern: null,
    filenamePattern: 'Invoice_*.pdf',
    tags: [],
    enabled: true,
    createdAt: 0,
    updatedAt: 0,
  };
}

let container: HTMLDivElement;
let root: Root;

const handlers = {
  onClose: vi.fn(),
  onCreateRule: vi.fn(async (name: string) => makeRule(name)),
  onUpdateRule: vi.fn(async (_id: string, name: string) => makeRule(name)),
  onDeleteRule: vi.fn(async () => {}),
  onRefreshAfterApply: vi.fn(),
  onRefreshSuggestions: vi.fn(),
  onDismissSuggestion: vi.fn(async () => {}),
  onAcceptSuggestion: vi.fn(async () => {}),
};

function render(suggestions: AttachmentRuleSuggestion[]) {
  act(() => {
    root.render(<RuleManagementModal rules={[]} accountId="acc-1" suggestions={suggestions} {...handlers} />);
  });
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes(label));
  if (!found) throw new Error(`button "${label}" not found`);
  return found;
}

function inputValues(): string[] {
  return Array.from(container.querySelectorAll('input[type="text"]')).map((i) => (i as HTMLInputElement).value);
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.click();
  });
}

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

describe('RuleManagementModal suggestions', () => {
  it('re-mines suggestions when opened', () => {
    render([]);
    expect(handlers.onRefreshSuggestions).toHaveBeenCalledTimes(1);
  });

  it('shows no suggestions section when there are none', () => {
    render([]);
    expect(container.textContent).not.toContain('attachments:suggestions.title');
  });

  it('lists each suggestion with its sender and filename patterns', () => {
    render([SUGGESTION]);

    expect(container.textContent).toContain('Acme · invoice');
    expect(container.textContent).toContain('billing@acme.com');
    expect(container.textContent).toContain('Invoice_*.pdf');
    expect(container.textContent).toContain('Invoice_0042.pdf');
  });

  it('dismisses a suggestion', async () => {
    render([SUGGESTION]);

    await click(button('attachments:suggestions.dismiss'));

    expect(handlers.onDismissSuggestion).toHaveBeenCalledWith('sug-1');
  });

  it('reviewing opens the rule form prefilled with the suggestion', async () => {
    render([SUGGESTION]);

    await click(button('attachments:suggestions.review'));

    expect(inputValues()).toEqual(['Acme · invoice', 'billing@acme.com', '', 'Invoice_*.pdf', 'invoice, acme']);
  });

  it('creating the reviewed rule applies it to existing mail and accepts the suggestion', async () => {
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('attachments:rules.createRule'));

    expect(handlers.onCreateRule).toHaveBeenCalledWith('Acme · invoice', 'billing@acme.com', null, 'Invoice_*.pdf', [
      'invoice',
      'acme',
    ]);
    expect(api.applyRuleRetroactively).toHaveBeenCalledWith('rule-1', 'acc-1');
    expect(handlers.onAcceptSuggestion).toHaveBeenCalledWith('sug-1');
  });

  it('cancelling the review leaves the suggestion pending', async () => {
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('common:actions.cancel'));

    expect(handlers.onAcceptSuggestion).not.toHaveBeenCalled();
    expect(handlers.onDismissSuggestion).not.toHaveBeenCalled();
  });

  it('a rule created from scratch does not accept any suggestion', async () => {
    render([SUGGESTION]);
    await click(button('attachments:list.createRule'));
    const nameInput = container.querySelector('input[type="text"]') as HTMLInputElement;
    const senderInput = container.querySelectorAll('input[type="text"]')[1] as HTMLInputElement;
    await act(async () => {
      for (const [el, v] of [
        [nameInput, 'Manual'],
        [senderInput, 'x@y.com'],
      ] as const) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
        setter?.call(el, v);
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }
    });

    await click(button('attachments:rules.createRule'));

    expect(handlers.onCreateRule).toHaveBeenCalled();
    expect(handlers.onAcceptSuggestion).not.toHaveBeenCalled();
  });
});
