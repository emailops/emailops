// Opening a draft is not an edit. The editor re-serialises what it loads
// (`<div>` lines become paragraphs, `<b>` becomes `<strong>`), and a composer
// that took that as a change saved the draft on open: on Gmail and Outlook the
// normalised copy was pushed upstream, and an IMAP draft stopped following the
// server. Only the user's own edits may reach `onChange`.

import type { Editor } from '@tiptap/react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({ currentPlatform: () => 'macos' }));

import { initI18n } from '@/i18n';
import { RichTextEditor } from './RichTextEditor';

/** HTML from another mail client, which the editor does not keep verbatim. */
const LOADED = '<div>Line one</div><div><b>Bold</b> line two</div>';

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

function render(onChange: (html: string) => void, disabled = false) {
  act(() => root.render(<RichTextEditor value={LOADED} onChange={onChange} disabled={disabled} />));
}

/** Tiptap hangs the editor off its contenteditable root. */
function editor(): Editor {
  const dom = container.querySelector('.ProseMirror') as (HTMLElement & { editor?: Editor }) | null;
  if (!dom?.editor) throw new Error('editor not found');
  return dom.editor;
}

describe('RichTextEditor loading existing content', () => {
  it('does not report the content it loaded as a change', () => {
    const onChange = vi.fn();
    render(onChange);

    expect(editor().getHTML()).not.toBe(LOADED); // it was re-serialised
    expect(onChange).not.toHaveBeenCalled();
  });

  it('does not report a change when it is disabled and enabled again', () => {
    const onChange = vi.fn();
    render(onChange);
    render(onChange, true);
    render(onChange, false);

    expect(onChange).not.toHaveBeenCalled();
  });

  it('reports an edit', () => {
    const onChange = vi.fn();
    render(onChange);

    act(() => {
      editor().commands.insertContent('!');
    });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0][0]).toContain('!');
  });
});
