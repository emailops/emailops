import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { DraftContextChip } from './DraftContextChip';

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

function render(props: Partial<Parameters<typeof DraftContextChip>[0]> = {}) {
  const onToggle = vi.fn();
  act(() =>
    root.render(
      <DraftContextChip
        subject="Re: Q3 report"
        scope="threadAndReply"
        active
        research={false}
        onToggle={onToggle}
        {...props}
      />,
    ),
  );
  return onToggle;
}

const button = () => container.querySelector('button') as HTMLButtonElement;

describe('DraftContextChip', () => {
  it('names the draft and what travels with it', () => {
    render();
    expect(container.textContent).toContain('Re: Q3 report');
    expect(container.textContent).toContain('panel.draft.threadAndReply');
  });

  it('says a new email is a new email', () => {
    render({ subject: 'Q3 report', scope: 'newEmail' });
    expect(container.textContent).toContain('panel.draft.newEmail');
  });

  it('says when there is no subject', () => {
    render({ subject: '  ' });
    expect(container.textContent).toContain('panel.context.noSubject');
  });

  it('switches the draft off when pressed while on', () => {
    const onToggle = render({ active: true });
    act(() => button().click());
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  it('stays on screen when off, offering to restore it', () => {
    const onToggle = render({ active: false });
    expect(button().getAttribute('aria-pressed')).toBe('false');
    expect(container.textContent).toContain('panel.draft.restore');
    act(() => button().click());
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it('warns that a research turn does not see the draft', () => {
    render({ research: true });
    expect(container.textContent).toContain('panel.draft.research');
  });
});
