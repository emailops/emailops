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

// A stable addLog, as the real store's is: effects depend on its identity.
const logs = vi.hoisted(() => ({ addLog: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof logs.addLog }) => unknown) => selector({ addLog: logs.addLog }),
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
  getAttachmentTagVocabulary: vi.fn(async () => ['invoice']),
  // The account picker is the shared Select: native on macOS.
  currentPlatform: vi.fn(() => 'macos'),
}));

import * as api from '@/lib/api';
import { useAttachmentStore } from '@/stores/attachmentStore';
import { type RuleAccount, RuleManagementModal } from './RuleManagementModal';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const SUGGESTION: AttachmentRuleSuggestion = {
  id: 'sug-1',
  accountId: 'acc-1',
  name: 'Acme · invoice',
  senderEmailPattern: 'billing@example.test',
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
    senderEmailPattern: 'billing@example.test',
    subjectPattern: null,
    filenamePattern: 'Invoice_*.pdf',
    tags: [],
    enabled: true,
    keepLatestPerThread: true,
    createdAt: 0,
    updatedAt: 0,
  };
}

let container: HTMLDivElement;
let root: Root;

const handlers = {
  onClose: vi.fn(),
  onCreateRule: vi.fn(async (_accountId: string, name: string) => makeRule(name)),
  onUpdateRule: vi.fn(async (_id: string, name: string) => makeRule(name)),
  onDeleteRule: vi.fn(async () => {}),
  onRefreshSuggestions: vi.fn(),
  onDismissSuggestion: vi.fn(async () => {}),
  onAcceptSuggestion: vi.fn(async () => {}),
  onRestoreSuggestion: vi.fn(async () => {}),
};

/** Fills the open form's name and sender, then creates the rule. */
async function fillAndCreate() {
  const [nameInput, senderInput] = Array.from(container.querySelectorAll<HTMLInputElement>('input[type="text"]'));
  await act(async () => {
    for (const [el, v] of [
      [nameInput, 'Manual'],
      [senderInput, 'sender@example.test'],
    ] as const) {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setter?.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }
  });
  await click(button('attachments:rules.createRule'));
}

const ONE_ACCOUNT: RuleAccount[] = [{ id: 'acc-1', email: 'b@example.com', colorClass: 'bg-blue-500' }];

function render(
  suggestions: AttachmentRuleSuggestion[],
  rules: AttachmentRule[] = [],
  extra: {
    dismissed?: AttachmentRuleSuggestion[];
    loading?: boolean;
    accounts?: RuleAccount[];
    defaultAccountId?: string;
  } = {},
) {
  act(() => {
    root.render(
      <RuleManagementModal
        rules={rules}
        accounts={extra.accounts ?? ONE_ACCOUNT}
        defaultAccountId={extra.defaultAccountId ?? 'acc-1'}
        suggestions={suggestions}
        suggestionsLoading={extra.loading ?? false}
        dismissedSuggestions={extra.dismissed ?? []}
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
  vi.mocked(api.getAttachmentTagVocabulary).mockReset().mockResolvedValue(['invoice']);
  for (const h of Object.values(handlers)) h.mockReset();
  handlers.onCreateRule.mockImplementation(async (_accountId: string, name: string) => makeRule(name));
  handlers.onUpdateRule.mockImplementation(async (_id: string, name: string) => makeRule(name));
  handlers.onDeleteRule.mockResolvedValue(undefined);
  handlers.onDismissSuggestion.mockResolvedValue(undefined);
  handlers.onAcceptSuggestion.mockResolvedValue(undefined);
  handlers.onRestoreSuggestion.mockResolvedValue(undefined);
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
    expect(container.textContent).toContain('billing@example.test');
    expect(container.textContent).toContain('Invoice_*.pdf');
    expect(container.textContent).toContain('Invoice_0042.pdf');
  });

  it('explains how the filename pattern was derived', () => {
    render([SUGGESTION]);

    expect(container.textContent).toContain('attachments:suggestions.patternHelp');
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
    expect(inputValues()).toEqual(['Acme · invoice', 'billing@example.test', '', 'Invoice_*.pdf', '']);
  });

  it('creating the reviewed rule applies it to existing mail and accepts the suggestion', async () => {
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('attachments:rules.createRule'));

    expect(handlers.onCreateRule).toHaveBeenCalledWith(
      'acc-1',
      'Acme · invoice',
      'billing@example.test',
      null,
      'Invoice_*.pdf',
      [],
      true,
    );
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
    expect(container.textContent).toContain('attachments:rules.applyDone{"count":20}');
    expect(container.textContent).toContain('attachments:rules.applyDoneNew{"count":0}');
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

  // --- Undo and dismissed suggestions ---

  it('dismissing a suggestion offers to undo it', async () => {
    render([SUGGESTION]);

    await click(button('attachments:suggestions.dismiss'));
    await click(button('attachments:suggestions.undo'));

    expect(handlers.onRestoreSuggestion).toHaveBeenCalledWith('sug-1');
  });

  it('dismissed suggestions can be listed and restored', async () => {
    const dismissed = { ...SUGGESTION, id: 'sug-9', name: 'Globex · receipt', status: 'dismissed' as const };
    render([], [], { dismissed: [dismissed] });

    await click(button('attachments:suggestions.showDismissed'));
    expect(container.textContent).toContain('Globex · receipt');
    await click(button('attachments:suggestions.restore'));

    expect(handlers.onRestoreSuggestion).toHaveBeenCalledWith('sug-9');
  });

  it('says it is looking for suggestions while the re-mine runs', () => {
    render([], [], { loading: true });

    expect(container.textContent).toContain('attachments:suggestions.searching');
  });

  // --- Accessibility ---

  it('is a modal dialog named by its title', () => {
    render([]);

    const dialog = container.querySelector('[role="dialog"]');
    expect(dialog?.getAttribute('aria-modal')).toBe('true');
    const titleId = dialog?.getAttribute('aria-labelledby');
    expect(titleId && document.getElementById(titleId)?.textContent).toBe('attachments:rules.modalTitle');
  });

  it('moves focus into the dialog when it opens', () => {
    render([]);

    expect(container.querySelector('[role="dialog"]')?.contains(document.activeElement)).toBe(true);
  });

  it('Escape closes the dialog', async () => {
    render([]);

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });

    expect(handlers.onClose).toHaveBeenCalled();
  });

  it('icon-only buttons have accessible names', () => {
    render([], [makeRule('Acme')]);

    const unnamed = Array.from(container.querySelectorAll('button')).filter(
      (b) => !b.textContent?.trim() && !b.getAttribute('aria-label'),
    );
    expect(unnamed).toEqual([]);
  });

  it('every rule form field is labelled', async () => {
    render([]);
    await click(button('attachments:list.createRule'));

    const fields = Array.from(container.querySelectorAll('input'));
    for (const field of fields) {
      const labelled = (field.id && container.querySelector(`label[for="${field.id}"]`)) || field.closest('label');
      expect(labelled, field.outerHTML).toBeTruthy();
    }
  });

  // --- Form validation and failures ---

  async function typeInto(el: HTMLInputElement, value: string) {
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setter?.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
  }

  function textInputs(): HTMLInputElement[] {
    return Array.from(container.querySelectorAll<HTMLInputElement>('input[type="text"]'));
  }

  it('a rule needs a name', async () => {
    render([]);
    await click(button('attachments:list.createRule'));

    await click(button('attachments:rules.createRule'));

    expect(container.textContent).toContain('attachments:rules.nameRequired');
    expect(handlers.onCreateRule).not.toHaveBeenCalled();
  });

  it('a rule needs at least one pattern', async () => {
    render([]);
    await click(button('attachments:list.createRule'));
    await typeInto(textInputs()[0], 'Only a name');

    await click(button('attachments:rules.createRule'));

    expect(container.textContent).toContain('attachments:rules.patternRequired');
    expect(handlers.onCreateRule).not.toHaveBeenCalled();
  });

  it('a failed save keeps the form open with the error', async () => {
    handlers.onCreateRule.mockRejectedValueOnce(new Error('duplicate rule'));
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));

    await click(button('attachments:rules.createRule'));

    expect(container.textContent).toContain('duplicate rule');
    expect(inputValues()[0]).toBe('Acme · invoice');
    expect(handlers.onAcceptSuggestion).not.toHaveBeenCalled();
  });

  it('unticking "apply to existing emails" creates the rule without scanning', async () => {
    render([SUGGESTION]);
    await click(button('attachments:suggestions.review'));
    const checkbox = Array.from(container.querySelectorAll('label'))
      .find((l) => l.textContent?.includes('attachments:rules.applyToExistingLabel'))
      ?.querySelector<HTMLInputElement>('input[type="checkbox"]');
    await act(async () => {
      checkbox?.click();
    });

    await click(button('attachments:rules.createRule'));

    expect(handlers.onCreateRule).toHaveBeenCalled();
    expect(api.applyRuleRetroactively).not.toHaveBeenCalled();
  });

  it('a failed enable/disable is shown', async () => {
    handlers.onUpdateRule.mockRejectedValueOnce(new Error('db locked'));
    render([], [makeRule('Acme')]);

    await click(button('attachments:rules.disable'));

    expect(container.textContent).toContain('db locked');
  });

  it('a failed restore is shown', async () => {
    handlers.onRestoreSuggestion.mockRejectedValueOnce(new Error('restore failed'));
    const dismissed = { ...SUGGESTION, id: 'sug-9', status: 'dismissed' as const };
    render([], [], { dismissed: [dismissed] });

    await click(button('attachments:suggestions.showDismissed'));
    await click(button('attachments:suggestions.restore'));

    expect(container.textContent).toContain('restore failed');
  });

  it('clicking outside the dialog closes it', async () => {
    render([]);
    const backdrop = container.firstElementChild as HTMLElement;

    await act(async () => {
      backdrop.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    });

    expect(handlers.onClose).toHaveBeenCalled();
  });

  it('clicking inside the dialog does not close it', async () => {
    render([]);
    const dialog = container.querySelector('[role="dialog"]') as HTMLElement;

    await act(async () => {
      dialog.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    });

    expect(handlers.onClose).not.toHaveBeenCalled();
  });

  // --- Deleting a rule ---

  it('deleting asks for confirmation, saying how many saved files go too', async () => {
    vi.mocked(api.countAttachmentsForRule).mockResolvedValueOnce(4);
    render([], [makeRule('Acme')]);

    await click(button('common:actions.delete'));

    expect(container.textContent).toContain('attachments:rules.deleteWithFiles{"count":4}');
    expect(handlers.onDeleteRule).not.toHaveBeenCalled();
  });

  it('confirming deletes the rule', async () => {
    vi.mocked(api.countAttachmentsForRule).mockResolvedValueOnce(0);
    render([], [makeRule('Acme')]);
    await click(button('common:actions.delete'));

    await click(button('attachments:rules.deleteRule'));

    expect(handlers.onDeleteRule).toHaveBeenCalledWith('rule-1');
    expect(container.textContent).not.toContain('attachments:rules.deleteConfirm');
  });

  it('cancelling keeps the rule', async () => {
    render([], [makeRule('Acme')]);
    await click(button('common:actions.delete'));

    await click(button('common:actions.cancel'));

    expect(handlers.onDeleteRule).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain('attachments:rules.deleteConfirm');
  });

  it('a failed delete is shown and the rule stays', async () => {
    handlers.onDeleteRule.mockRejectedValueOnce(new Error('in use'));
    render([], [makeRule('Acme')]);
    await click(button('common:actions.delete'));

    await click(button('attachments:rules.deleteRule'));

    expect(container.textContent).toContain('in use');
  });

  it('when the file count cannot be read, deleting is still possible', async () => {
    vi.mocked(api.countAttachmentsForRule).mockRejectedValueOnce(new Error('db locked'));
    render([], [makeRule('Acme')]);

    await click(button('common:actions.delete'));

    expect(container.textContent).toContain('attachments:rules.deleteNoFiles');
    expect(button('attachments:rules.deleteRule').disabled).toBe(false);
  });

  it("shows the rule's tags", () => {
    render([], [{ ...makeRule('Acme'), tags: ['facturas', 'acme'] }]);

    expect(container.textContent).toContain('facturas');
    expect(container.textContent).toContain('acme');
  });
});

describe('RuleManagementModal tags', () => {
  /** Opens a new rule's form and its tag list, after the tags load. */
  async function openTagList(): Promise<string[]> {
    await act(async () => {});
    await click(button('attachments:list.createRule'));
    const input = container.querySelector<HTMLInputElement>('input[role="combobox"]');
    if (!input) throw new Error('no tag input');
    await act(async () => {
      input.focus();
      input.dispatchEvent(new FocusEvent('focus', { bubbles: true }));
    });
    return Array.from(container.querySelectorAll('[role="option"]')).map((o) => o.textContent ?? '');
  }

  it('offers the tags of every account, not only this one', async () => {
    vi.mocked(api.getAttachmentTagVocabulary).mockResolvedValue(['factura', 'receipt']);
    render([], [{ ...makeRule('Acme'), tags: ['acme'] }]);

    expect(await openTagList()).toEqual(['acme', 'factura', 'receipt']);
  });

  it("offers one spelling of a tag this account's rules spell differently", async () => {
    vi.mocked(api.getAttachmentTagVocabulary).mockResolvedValue(['Invoice']);
    render([], [{ ...makeRule('Acme'), tags: ['invoice'] }]);

    expect(await openTagList()).toEqual(['Invoice']);
  });

  it("logs a failed load and still offers this account's rule tags", async () => {
    vi.mocked(api.getAttachmentTagVocabulary).mockRejectedValue(new Error('db locked'));
    render([], [{ ...makeRule('Acme'), tags: ['acme'] }]);

    expect(await openTagList()).toEqual(['acme']);
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('db locked'));
  });
});

describe('RuleManagementModal account', () => {
  it('names the account its rules belong to in the header', () => {
    render([]);
    expect(container.textContent).toContain('attachments:rules.accountHeader{"email":"b@example.com"}');
  });

  it('with one account offers no account to pick and creates the rule there', async () => {
    render([]);
    await click(button('attachments:list.createRule'));

    expect(container.querySelector('[data-testid="rule-account-select"]')).toBeNull();
    await fillAndCreate();
    expect(handlers.onCreateRule.mock.calls[0][0]).toBe('acc-1');
  });
});

describe('RuleManagementModal keep latest version', () => {
  const keepLatest = () => container.querySelector<HTMLInputElement>('[data-testid="rule-keep-latest"]');

  it('a new rule keeps only the latest version of each file in a thread unless unchecked', async () => {
    render([]);
    await click(button('attachments:list.createRule'));
    expect(keepLatest()?.checked).toBe(true);

    await fillAndCreate();
    expect((handlers.onCreateRule.mock.calls[0] as unknown[])[6]).toBe(true);

    await click(button('attachments:list.createRule'));
    await act(async () => keepLatest()?.click());
    await fillAndCreate();
    expect((handlers.onCreateRule.mock.calls[1] as unknown[])[6]).toBe(false);
  });

  it('editing a rule shows its setting and saves the change', async () => {
    render([], [{ ...makeRule('Acme'), keepLatestPerThread: false }]);
    await click(button('common:actions.edit'));
    expect(keepLatest()?.checked).toBe(false);

    await act(async () => keepLatest()?.click());
    await click(button('attachments:rules.updateRule'));

    expect((handlers.onUpdateRule.mock.calls[0] as unknown[])[7]).toBe(true);
  });

  it('turning a rule off or on keeps its setting', async () => {
    render([], [{ ...makeRule('Acme'), keepLatestPerThread: true }]);

    await click(button('attachments:rules.disable'));

    expect((handlers.onUpdateRule.mock.calls[0] as unknown[])[6]).toBe(false);
    expect((handlers.onUpdateRule.mock.calls[0] as unknown[])[7]).toBe(true);
  });
});

describe('RuleManagementModal in All accounts', () => {
  const ACCOUNTS: RuleAccount[] = [
    { id: 'acc-1', email: 'a@example.com', colorClass: 'bg-blue-500' },
    { id: 'acc-2', email: 'b@example.com', colorClass: 'bg-emerald-500' },
  ];
  const ruleOf = (id: string, accountId: string, name: string): AttachmentRule => ({
    ...makeRule(name),
    id,
    accountId,
  });

  function accountSelect(): HTMLSelectElement {
    const select = container.querySelector<HTMLSelectElement>('[data-testid="rule-account-select"]');
    if (!select) throw new Error('no account select');
    return select;
  }

  it('says it lists every account and names the account of each rule', () => {
    render([], [ruleOf('ra', 'acc-1', 'A'), ruleOf('rb', 'acc-2', 'B')], { accounts: ACCOUNTS });

    expect(container.textContent).toContain('attachments:rules.allAccountsHeader');
    const chips = Array.from(container.querySelectorAll('[data-testid="rule-account-chip"]')).map((c) => c.textContent);
    expect(chips).toEqual(['a@example.com', 'b@example.com']);
  });

  it('names the account of each suggestion', () => {
    render([{ ...SUGGESTION, accountId: 'acc-2' }], [], { accounts: ACCOUNTS });

    const chips = Array.from(container.querySelectorAll('[data-testid="rule-account-chip"]')).map((c) => c.textContent);
    expect(chips).toEqual(['b@example.com']);
  });

  it('creates a new rule in the default account unless another is picked', async () => {
    render([], [], { accounts: ACCOUNTS, defaultAccountId: 'acc-2' });
    await click(button('attachments:list.createRule'));
    expect(accountSelect().value).toBe('acc-2');

    await act(async () => {
      accountSelect().value = 'acc-1';
      accountSelect().dispatchEvent(new Event('change', { bubbles: true }));
    });
    await fillAndCreate();

    expect(handlers.onCreateRule.mock.calls[0][0]).toBe('acc-1');
  });

  it("reviewing a suggestion creates the rule in the suggestion's account", async () => {
    render([{ ...SUGGESTION, accountId: 'acc-2' }], [], { accounts: ACCOUNTS });
    await click(button('attachments:suggestions.review'));
    await click(button('attachments:rules.createRule'));

    expect(handlers.onCreateRule.mock.calls[0][0]).toBe('acc-2');
  });

  it('scans existing mail and counts files in the account of the rule', async () => {
    render([], [ruleOf('rb', 'acc-2', 'B')], { accounts: ACCOUNTS });

    await click(button('attachments:rules.applyToExisting'));
    await click(button('common:actions.delete'));

    expect(vi.mocked(api.applyRuleRetroactively).mock.calls[0].slice(0, 2)).toEqual(['rb', 'acc-2']);
    expect(api.countAttachmentsForRule).toHaveBeenCalledWith('acc-2', 'rb');
  });
});
