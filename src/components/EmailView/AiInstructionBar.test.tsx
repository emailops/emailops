// The reply composer's free-text instruction for the AI draft: the user says
// what the reply should do ("accept, but propose Thursday") and (re)generates.

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
  act(() => {
    root.render(<AiInstructionBar onGenerate={onGenerate} isGenerating={false} hasDraft={false} {...props} />);
  });
  const input = container.querySelector('input') as HTMLInputElement;
  const button = container.querySelector('button') as HTMLButtonElement;
  return { onGenerate, input, button };
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
});
