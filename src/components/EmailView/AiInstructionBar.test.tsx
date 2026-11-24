// The composers' free-text instructions for the AI: the user says what the
// email should do ("accept, but propose Thursday") and (re)generates, or
// polishes the draft already written. The instructions are never sent.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

import { AiInstructionBar } from './AiInstructionBar';

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

function render(props: Partial<Parameters<typeof AiInstructionBar>[0]> = {}) {
  const onGenerate = vi.fn();
  const onPolish = vi.fn();
  act(() => {
    root.render(
      <AiInstructionBar onGenerate={onGenerate} onPolish={onPolish} isGenerating={false} hasDraft={false} {...props} />,
    );
  });
  const input = container.querySelector('input') as HTMLInputElement;
  const button = container.querySelector('[data-testid="ai-generate"]') as HTMLButtonElement;
  const polish = container.querySelector('[data-testid="ai-polish"]') as HTMLButtonElement;
  return { onGenerate, onPolish, input, button, polish };
}

function type(input: HTMLInputElement, value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('AiInstructionBar', () => {
  it('generates with the trimmed instruction', () => {
    const { onGenerate, input, button } = render();
    type(input, '  accept, but propose Thursday  ');
    act(() => button.click());
    expect(onGenerate).toHaveBeenCalledWith('accept, but propose Thursday');
  });

  it('generates on Enter', () => {
    const { onGenerate, input } = render();
    type(input, 'say no politely');
    act(() => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(onGenerate).toHaveBeenCalledWith('say no politely');
  });

  it('generates without an instruction when the box is empty', () => {
    const { onGenerate, button } = render();
    act(() => button.click());
    expect(onGenerate).toHaveBeenCalledWith('');
  });

  it('is disabled while a draft is generating', () => {
    const { onGenerate, input, button } = render({ isGenerating: true });
    expect(button.disabled).toBe(true);
    expect(input.disabled).toBe(true);
    act(() => button.click());
    expect(onGenerate).not.toHaveBeenCalled();
  });

  it('offers to regenerate once a draft exists', () => {
    const { button } = render({ hasDraft: true });
    expect(button.textContent).toContain('compose:aiDraft.regenerate');
  });

  it('polishes the draft with the trimmed instruction', () => {
    const { onPolish, onGenerate, input, polish } = render({ hasDraft: true });
    type(input, '  make it more formal  ');
    act(() => polish.click());
    expect(onPolish).toHaveBeenCalledWith('make it more formal');
    expect(onGenerate).not.toHaveBeenCalled();
  });

  it('polishes without an instruction when the box is empty', () => {
    const { onPolish, polish } = render({ hasDraft: true });
    act(() => polish.click());
    expect(onPolish).toHaveBeenCalledWith('');
  });

  it('has nothing to polish before there is a draft, and says so', () => {
    const { onPolish, polish } = render({ hasDraft: false });
    expect(polish.disabled).toBe(true);
    expect(polish.title).toBe('compose:aiDraft.polishNeedsDraft');
    act(() => polish.click());
    expect(onPolish).not.toHaveBeenCalled();
  });

  it('does not generate when the composer lacks what a draft needs, and says what', () => {
    const { onGenerate, input, button } = render({ canGenerate: false, generateTitle: 'needs a subject' });
    expect(button.disabled).toBe(true);
    expect(button.title).toBe('needs a subject');
    type(input, 'say yes');
    act(() => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(onGenerate).not.toHaveBeenCalled();
  });

  it('locks both actions while either runs, and shows which one is running', () => {
    const { onPolish, input, button, polish } = render({ hasDraft: true });
    type(input, 'warmer');
    act(() => polish.click());
    act(() => {
      root.render(<AiInstructionBar onGenerate={vi.fn()} onPolish={onPolish} isGenerating={true} hasDraft={true} />);
    });
    expect(polish.disabled && button.disabled).toBe(true);
    expect(polish.textContent).toContain('compose:aiDraft.polishing');
    // The instruction is kept for a retry.
    expect(input.value).toBe('warmer');
  });

  it('shows why the last request failed', () => {
    const { input } = render({ hasDraft: true, error: 'The AI took too long' });
    expect(container.textContent).toContain('The AI took too long');
    expect(input.disabled).toBe(false);
  });
});
