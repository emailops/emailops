// The attachments list: a failed load shows its own state with Retry, apart
// from "nothing collected yet".

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts ? `${key}${JSON.stringify(opts)}` : key),
    i18n: { language: 'en' },
  }),
}));

import type { Attachment } from '@/types';
import { AttachmentList } from './AttachmentList';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const attachment = (id: string): Attachment => ({
  id,
  accountId: 'acc-1',
  emailId: `email-${id}`,
  ruleId: 'r1',
  ruleName: 'Invoices',
  gmailAttachmentId: '',
  filename: `${id}.pdf`,
  mimeType: 'application/pdf',
  fileSize: 1,
  filePath: `attachments/acc-1/${id}`,
  tags: ['invoice'],
  senderEmail: 'billing@vendor.example',
  subject: 'Invoice',
  emailTimestamp: 1_700_000_000,
  createdAt: 0,
});

let container: HTMLDivElement;
let root: Root;

function render(props: Partial<Parameters<typeof AttachmentList>[0]>) {
  act(() => {
    root.render(
      <AttachmentList
        attachments={[]}
        selectedAttachment={null}
        selectedTag={null}
        checkedIds={new Set()}
        isLoading={false}
        isLoadingMore={false}
        hasMore={false}
        listError={null}
        onRetry={vi.fn()}
        onSelectAttachment={vi.fn()}
        onToggleChecked={vi.fn()}
        onLoadMore={vi.fn()}
        onOpenRules={vi.fn()}
        {...props}
      />,
    );
  });
}

const button = (label: string) =>
  Array.from(container.querySelectorAll('button')).find((b) => b.textContent === label) ?? null;

beforeEach(() => {
  container = document.createElement('div');
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
});

describe('AttachmentList load error', () => {
  it('shows the error with Retry instead of the empty state', () => {
    const onRetry = vi.fn();
    render({ listError: 'db locked', onRetry });

    expect(container.textContent).toContain('attachments:list.loadError');
    expect(container.textContent).not.toContain('attachments:list.empty');
    act(() => button('attachments:list.retry')?.click());
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('keeps the rows already shown under the error', () => {
    render({ listError: 'db locked', attachments: [attachment('a1')] });

    expect(container.textContent).toContain('attachments:list.loadError');
    expect(container.textContent).toContain('a1.pdf');
  });

  it('shows no error without one', () => {
    render({ attachments: [attachment('a1')] });

    expect(button('attachments:list.retry')).toBeNull();
  });
});

describe('AttachmentList rows', () => {
  it('in All accounts each row shows its account and the rule that saved it', () => {
    render({
      attachments: [{ ...attachment('b1'), accountId: 'acc-2', ruleName: 'Sent invoices' }],
      getAccountBadge: (a) => ({ colorClass: 'bg-sky-500', label: a.accountId === 'acc-2' ? 'b@example.com' : '?' }),
    });

    const chip = container.querySelector('[data-testid="attachment-account-chip"]');
    expect(chip?.textContent).toBe('b@example.com');
    expect(chip?.getAttribute('title')).toBe('b@example.com');
    expect(container.textContent).toContain('Sent invoices');
  });

  it('shows no account chip for a single account', () => {
    render({ attachments: [attachment('a1')] });

    expect(container.querySelector('[data-testid="attachment-account-chip"]')).toBeNull();
    expect(container.textContent).toContain('Invoices');
  });

  it('with a tag selected and nothing listed, says so instead of offering a first rule', () => {
    render({ selectedTag: 'invoice' });

    expect(container.textContent).toContain('attachments:list.emptyForTag{"tag":"invoice"}');
    expect(button('attachments:list.createRule')).toBeNull();
  });
});
