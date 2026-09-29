// Suggested attachment rules inside RuleManagementModal: the user reviews a
// candidate in the regular (prefilled) rule form and confirms it by creating
// the rule, or dismisses it so it is never proposed again.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AttachmentRule, AttachmentRuleSuggestion } from '@/types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts ? `${key}${JSON.stringify(opts)}` : key),
    i18n: { language: 'en' },
  }),
}));

vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: vi.fn() }),
}));

const events = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void> }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => void) => {
    events.handlers[name] = handler;
    return () => {};
  }),
}));

vi.mock('@/lib/api', () => ({
  applyRuleRetroactively: vi.fn(async () => {}),
  countAttachmentsForRule: vi.fn(async () => 0),
}));

import * as api from '@/lib/api';
import { useAttachmentStore } from '@/stores/attachmentStore';
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
  onRefreshSuggestions: vi.fn(),
  onDismissSuggestion: vi.fn(async () => {}),
  onAcceptSuggestion: vi.fn(async () => {}),
};

function render(suggestions: AttachmentRuleSuggestion[], rules: AttachmentRule[] = []) {
  act(() => {
    root.render(
      <RuleManagementModal
        rules={rules}
        accountId="acc-1"
        suggestions={suggestions}
        existingTags={['invoice']}
        {...handlers}
      />,
    );
  });
}

/** Progress of the rule's current scan, as the app-wide listener stores it. */
async function reportProgress(ruleId: string, processed: number, total: number, saved: number) {
  await act(async () => {
    const store = useAttachmentStore.getState();
    const runId = store.ruleApplies[ruleId]?.runId ?? '';
    store.reportRuleApplyProgress({ ruleId, runId, processed, total, saved });
  });
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button')).find(
    (b) => b.textContent?.includes(label) || b.getAttribute('title') === label,
  );
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
  // clearAllMocks keeps queued `mockReturnValueOnce` values: reset the mocks a
  // test may leave unconsumed so no test depends on the order they run in.
  vi.mocked(api.applyRuleRetroactively).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.countAttachmentsForRule).mockReset().mockResolvedValue(0);
  for (const h of Object.values(handlers)) h.mockReset();
  handlers.onCreateRule.mockImplementation(async (name: string) => makeRule(name));
  handlers.onUpdateRule.mockImplementation(async (_id: string, name: string) => makeRule(name));
  handlers.onDeleteRule.mockResolvedValue(undefined);
  handlers.onDismissSuggestion.mockResolvedValue(undefined);
  handlers.onAcceptSuggestion.mockResolvedValue(undefined);
  useAttachmentStore.setState({ ruleApplies: {} });
  Element.prototype.scrollIntoView = vi.fn();
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

  it('heads the existing rules with their own title, below the suggestions', () => {
    render([SUGGESTION], [makeRule('Acme invoices')]);

    const text = container.textContent ?? '';
    expect(text).toContain('attachments:rules.existingTitle');
    expect(text.indexOf('attachments:suggestions.title')).toBeLessThan(text.indexOf('attachments:rules.existingTitle'));
    expect(text.indexOf('attachments:rules.existingTitle')).toBeLessThan(text.indexOf('Acme invoices'));
  });

  it('shows no existing-rules title when there are no rules', () => {
    render([SUGGESTION]);
    expect(container.textContent).not.toContain('attachments:rules.existingTitle');
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

    // Tags stay empty: the user picks their own.
    expect(inputValues()).toEqual(['Acme · invoice', 'billing@acme.com', '', 'Invoice_*.pdf', '']);
  });

  it('creating the reviewed rule applies it to existing mail and accepts the suggestion', async () => {
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('attachments:rules.createRule'));

    expect(handlers.onCreateRule).toHaveBeenCalledWith('Acme · invoice', 'billing@acme.com', null, 'Invoice_*.pdf', []);
    expect(api.applyRuleRetroactively).toHaveBeenCalledWith('rule-1', 'acc-1', expect.any(String));
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

  it('creating a rule closes the form without waiting for the scan of existing mail', async () => {
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('attachments:rules.createRule'));

    expect(container.textContent).not.toContain('attachments:rules.newTitle');
    expect(api.applyRuleRetroactively).toHaveBeenCalled();
  });

  it('the rule card shows the scan progress reported by the backend', async () => {
    render([], [makeRule('Acme')]);
    await click(button('attachments:rules.applyToExisting'));

    await reportProgress('rule-1', 1, 4, 1);

    const bar = container.querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('25');
  });

  it('the rule card reports how many attachments the rule holds once the scan ends', async () => {
    render([], [makeRule('Acme')]);
    await click(button('attachments:rules.applyToExisting'));

    await act(async () => {
      const store = useAttachmentStore.getState();
      store.finishRuleApply('rule-1', store.ruleApplies['rule-1'].runId, 0, 20);
    });

    expect(container.querySelector('[role="progressbar"]')).toBeNull();
    expect(container.textContent).toContain('attachments:rules.applyDone{"count":20,"new":0}');
  });

  it('a scan that cannot be queued is shown as failed', async () => {
    vi.mocked(api.applyRuleRetroactively).mockRejectedValueOnce(new Error('queue closed'));
    render([], [makeRule('Acme')]);

    await click(button('attachments:rules.applyToExisting'));

    expect(container.textContent).toContain('attachments:rules.applyFailed');
  });

  it('reviewing a suggestion scrolls the prefilled form into view', async () => {
    render([SUGGESTION]);

    await click(button('attachments:suggestions.review'));

    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
  });

  it('editing a rule scrolls the form into view', async () => {
    render([], [makeRule('Acme')]);

    await click(button('common:actions.edit'));

    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
  });

  it('saving an edited rule closes the form and re-applies it in the background', async () => {
    render([], [makeRule('Acme')]);
    await click(button('common:actions.edit'));

    await click(button('attachments:rules.updateRule'));

    expect(handlers.onUpdateRule).toHaveBeenCalled();
    expect(container.textContent).not.toContain('attachments:rules.editTitle');
    expect(api.applyRuleRetroactively).toHaveBeenCalledWith('rule-1', 'acc-1', expect.any(String));
  });

  it('re-enabling a disabled rule scans the mail that arrived while it was off', async () => {
    render([], [{ ...makeRule('Acme'), enabled: false }]);

    await click(button('attachments:rules.enable'));

    expect(api.applyRuleRetroactively).toHaveBeenCalledWith('rule-1', 'acc-1', expect.any(String));
  });

  it('disabling a rule does not scan', async () => {
    handlers.onUpdateRule.mockImplementationOnce(async (_id: string, name: string) => ({
      ...makeRule(name),
      enabled: false,
    }));
    render([], [makeRule('Acme')]);

    await click(button('attachments:rules.disable'));

    expect(api.applyRuleRetroactively).not.toHaveBeenCalled();
  });

  it('a failed accept after the rule was created still closes the form and scans', async () => {
    handlers.onAcceptSuggestion.mockRejectedValueOnce(new Error('suggestion gone'));
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('attachments:rules.createRule'));

    expect(inputValues()).toEqual([]);
    expect(api.applyRuleRetroactively).toHaveBeenCalledWith('rule-1', 'acc-1', expect.any(String));
  });

  it('a failed dismiss is shown in the modal', async () => {
    handlers.onDismissSuggestion.mockRejectedValueOnce(new Error('db locked'));
    render([SUGGESTION]);

    await click(button('attachments:suggestions.dismiss'));

    expect(container.textContent).toContain('db locked');
  });

  it('dismiss is disabled while it is in flight', async () => {
    handlers.onDismissSuggestion.mockReturnValueOnce(new Promise(() => {}));
    render([SUGGESTION]);

    await click(button('attachments:suggestions.dismiss'));

    expect(button('attachments:suggestions.dismiss').disabled).toBe(true);
  });

  it('a scan still running shows its progress when the modal is reopened', async () => {
    render([], [makeRule('Acme')]);
    await click(button('attachments:rules.applyToExisting'));
    await reportProgress('rule-1', 1, 4, 1);

    act(() => root.unmount());
    root = createRoot(container);
    render([], [makeRule('Acme')]);

    expect(container.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('25');
    expect(button('attachments:rules.applyToExisting').disabled).toBe(true);
  });
});
