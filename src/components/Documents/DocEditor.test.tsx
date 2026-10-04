// The doc editor is bound to the shared Y.Doc: text another person wrote
// appears in the open editor, and typing here becomes a Yjs change.

import type { Editor } from '@tiptap/react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { DOC_FIELD, DocEditor } from './DocEditor';

/** A paragraph written by someone else, as a Yjs update. */
function remoteParagraph(base: Y.Doc, text: string): Uint8Array {
  const other = new Y.Doc();
  Y.applyUpdate(other, Y.encodeStateAsUpdate(base));
  const p = new Y.XmlElement('paragraph');
  p.insert(0, [new Y.XmlText(text)]);
  other.getXmlFragment(DOC_FIELD).insert(other.getXmlFragment(DOC_FIELD).length, [p]);
  return Y.encodeStateAsUpdate(other, Y.encodeStateVector(base));
}

describe('DocEditor', () => {
  let container: HTMLDivElement;
  let root: Root;
  let editor: Editor | null;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    editor = null;
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('shows what another person wrote and records local typing in the doc', async () => {
    const doc = new Y.Doc();
    await act(async () => {
      root.render(<DocEditor doc={doc} editable onEditor={(e) => (editor = e)} />);
    });

    await act(async () => Y.applyUpdate(doc, remoteParagraph(doc, 'From a colleague')));
    expect(container.textContent).toContain('From a colleague');

    await act(async () => {
      editor?.commands.insertContentAt(editor.state.doc.content.size, '<p>Mine</p>');
    });
    expect(doc.getXmlFragment(DOC_FIELD).toString()).toContain('Mine');
  });

  it('a read-only editor has no toolbar and cannot be typed in', async () => {
    const doc = new Y.Doc();
    await act(async () => {
      root.render(<DocEditor doc={doc} editable={false} onEditor={(e) => (editor = e)} />);
    });
    expect(container.querySelector('[aria-label="documents:toolbar.bold"]')).toBeNull();
    expect(editor?.isEditable).toBe(false);
  });
});
