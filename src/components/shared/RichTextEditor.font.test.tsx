// The compose toolbar offers font, size and colour pickers and a Clear
// formatting button, and every control names the formatting of the text at
// the cursor or in the selection, following it as it moves. Choosing a font in
// the picker applies it to the selection; what the other font, size and colour
// commands, and clearing the formatting, do to the document is covered on the
// schema side (`composeEditorExtensions.test.ts`).

import type { Editor } from '@tiptap/react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({ currentPlatform: () => 'macos' }));

import { initI18n } from '@/i18n';
import { RichTextEditor } from './RichTextEditor';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
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

function render(value: string) {
  act(() => root.render(<RichTextEditor value={value} onChange={() => {}} />));
}

function picker(label: string): HTMLSelectElement {
  const select = container.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`);
  if (!select) throw new Error(`${label} picker not found`);
  return select;
}

const fontPicker = () => picker('Font');
const sizePicker = () => picker('Font size');
const colorPicker = () => picker('Text colour');

function shownLabel(select: HTMLSelectElement): string | null | undefined {
  return select.selectedOptions[0]?.textContent;
}

/** Tiptap hangs the editor off its contenteditable root. */
function editor(): Editor {
  const dom = container.querySelector('.ProseMirror') as (HTMLElement & { editor?: Editor }) | null;
  if (!dom?.editor) throw new Error('editor not found');
  return dom.editor;
}

/** Put the cursor inside the first occurrence of `text`. */
function moveCursorInto(text: string) {
  let target = -1;
  editor().state.doc.descendants((node, pos) => {
    if (target === -1 && node.isText && node.text?.includes(text)) target = pos + node.text.indexOf(text) + 1;
  });
  act(() => {
    editor().commands.setTextSelection(target);
  });
}

/** Select the first occurrence of `text`. */
function selectText(text: string) {
  let from = -1;
  editor().state.doc.descendants((node, pos) => {
    if (from === -1 && node.isText && node.text?.includes(text)) from = pos + node.text.indexOf(text);
  });
  act(() => {
    editor().commands.setTextSelection({ from, to: from + text.length });
  });
}

/** Pick the option labelled `label`, as the user does from the dropdown. */
function choose(select: HTMLSelectElement, label: string) {
  const option = Array.from(select.options).find((o) => o.textContent === label);
  if (!option) throw new Error(`option ${label} not found`);
  act(() => {
    select.value = option.value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

function boldButton(): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>('button[title^="Bold"]');
  if (!button) throw new Error('bold button not found');
  return button;
}

describe('RichTextEditor font picker', () => {
  it('offers the default font and a set of email-safe fonts', () => {
    render('<p>Hello</p>');

    const labels = Array.from(fontPicker().options).map((option) => option.textContent);
    expect(labels[0]).toBe('Font');
    expect(labels).toContain('Arial');
    expect(labels).toContain('Georgia');
    expect(labels).toContain('Times New Roman');
  });

  it('shows the default font for unstyled text', () => {
    render('<p>Hello</p>');

    expect(fontPicker().selectedOptions[0]?.textContent).toBe('Font');
  });

  it('names the font of the text at the cursor', () => {
    render('<p><span style="font-family: Georgia, serif">Hello</span></p>');

    expect(fontPicker().selectedOptions[0]?.textContent).toBe('Georgia');
  });

  it('offers a default size and a set of sizes', () => {
    render('<p>Hello</p>');

    const labels = Array.from(sizePicker().options).map((option) => option.textContent);
    expect(labels[0]).toBe('Size');
    expect(labels).toContain('12');
    expect(labels).toContain('24');
  });

  it('names the size of the text at the cursor', () => {
    render('<p><span style="font-size: 18px">Hello</span></p>');

    expect(shownLabel(sizePicker())).toBe('18');
  });

  it('follows the cursor into differently formatted text', () => {
    render('<p>plain <strong><span style="font-family: Georgia, serif; font-size: 24px">styled</span></strong></p>');
    moveCursorInto('plain');
    expect(shownLabel(fontPicker())).toBe('Font');
    expect(shownLabel(sizePicker())).toBe('Size');
    expect(boldButton().className).not.toContain('bg-primary-100');

    moveCursorInto('styled');

    expect(shownLabel(fontPicker())).toBe('Georgia');
    expect(shownLabel(sizePicker())).toBe('24');
    expect(boldButton().className).toContain('bg-primary-100');
  });

  it('names the font of the selected text', () => {
    render('<p>plain <span style="font-family: Georgia, serif">Hello world</span></p>');
    moveCursorInto('plain');
    expect(shownLabel(fontPicker())).toBe('Font');

    selectText('world');

    expect(shownLabel(fontPicker())).toBe('Georgia');
  });

  it('applies the chosen font to the selected text only', () => {
    render('<p>Hello world</p>');
    selectText('world');

    choose(fontPicker(), 'Georgia');

    const html = document.createElement('div');
    html.innerHTML = editor().getHTML();
    expect(html.textContent).toBe('Hello world');
    const styled = Array.from(html.querySelectorAll('span'), (span) => [span.textContent, span.style.fontFamily]);
    expect(styled).toEqual([['world', 'Georgia, serif']]);
  });
});

describe('RichTextEditor colour picker', () => {
  it('offers the default colour and a palette', () => {
    render('<p>Hello</p>');

    const labels = Array.from(colorPicker().options).map((option) => option.textContent);
    expect(labels[0]).toBe('Colour');
    expect(labels).toContain('Red');
    expect(labels).toContain('Blue');
  });

  it('names the colour of the text at the cursor', () => {
    render('<p>plain <span style="color: rgb(220, 38, 38)">red</span></p>');
    moveCursorInto('plain');
    expect(shownLabel(colorPicker())).toBe('Colour');

    moveCursorInto('red');

    expect(shownLabel(colorPicker())).toBe('Red');
  });

  it('colours the selected text with the chosen colour', () => {
    render('<p>Hello</p>');
    act(() => {
      editor().commands.selectAll();
    });
    const select = colorPicker();
    const blue = Array.from(select.options).find((option) => option.textContent === 'Blue');
    if (!blue) throw new Error('Blue not offered');

    act(() => {
      select.value = blue.value;
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });

    const span = new DOMParser().parseFromString(editor().getHTML(), 'text/html').querySelector('span');
    expect(span?.style.color).toBe(blue.value);
    expect(span?.textContent).toBe('Hello');
  });
});

describe('RichTextEditor clear formatting', () => {
  it('clears the formatting of the selected text', () => {
    // Ends in a paragraph: after a heading the editor appends an empty one.
    render('<h2 style="color: red"><strong>Hello</strong></h2><p><em>world</em></p>');
    act(() => {
      editor().commands.selectAll();
    });
    const button = container.querySelector<HTMLButtonElement>('button[title="Clear formatting"]');
    if (!button) throw new Error('Clear formatting button not found');

    act(() => button.click());

    expect(editor().getHTML()).toBe('<p>Hello</p><p>world</p>');
  });
});

describe('RichTextEditor compact toolbar', () => {
  it.each(['Bullet list', 'Numbered list', 'Insert link', 'Clear formatting'])(
    'shows %s as an icon named by its tooltip',
    (title) => {
      render('<p>Hello</p>');

      const button = container.querySelector<HTMLButtonElement>(`button[title="${title}"]`);

      expect(button?.textContent).toBe('');
      expect(button?.querySelector('svg')).not.toBeNull();
    },
  );
});
