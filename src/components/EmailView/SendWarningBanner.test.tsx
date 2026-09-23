// Shown instead of sending when the reply looks unfinished (mentions an
// attachment but has none, or still holds an AI [placeholder]).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, string>) => (opts?.items ? `${key}:${opts.items}` : key),
    i18n: { language: 'en' },
  }),
}));

import { SendWarningBanner } from './SendWarningBanner';

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
});

function buttons() {
  return Array.from(container.querySelectorAll('button'));
}

describe('SendWarningBanner', () => {
  it('lists every warning', () => {
    act(() => {
      root.render(
        <SendWarningBanner
          warnings={[
            { kind: 'missingAttachment' },
            { kind: 'unfilledPlaceholder', text: '[fecha]' },
            { kind: 'unfilledPlaceholder', text: '[nombre]' },
          ]}
          onSendAnyway={vi.fn()}
          onReview={vi.fn()}
        />,
      );
    });
    expect(container.textContent).toContain('compose:sendWarnings.missingAttachment');
    expect(container.textContent).toContain('compose:sendWarnings.unfilledPlaceholder:[fecha], [nombre]');
  });

  it('sends anyway or goes back to review', () => {
    const onSendAnyway = vi.fn();
    const onReview = vi.fn();
    act(() => {
      root.render(
        <SendWarningBanner
          warnings={[{ kind: 'missingAttachment' }]}
          onSendAnyway={onSendAnyway}
          onReview={onReview}
        />,
      );
    });
    const [review, sendAnyway] = buttons();
    act(() => review.click());
    expect(onReview).toHaveBeenCalledOnce();
    act(() => sendAnyway.click());
    expect(onSendAnyway).toHaveBeenCalledOnce();
  });
});
