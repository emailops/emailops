import Collaboration from '@tiptap/extension-collaboration';
import { type Editor, EditorContent, useEditor } from '@tiptap/react';
import { redo, undo } from '@tiptap/y-tiptap';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import type * as Y from 'yjs';
import { DOC_FIELD, docSchemaExtensions } from '@/lib/docSchema';

export { DOC_FIELD };

interface DocEditorProps {
  doc: Y.Doc;
  editable: boolean;
  /** Hands the editor up, so sharing can take an HTML copy of the text. */
  onEditor: (editor: Editor | null) => void;
}

interface ToolbarButtonProps {
  label: string;
  text: string;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
}

function ToolbarButton({ label, text, active, disabled = false, onClick }: ToolbarButtonProps) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onMouseDown={(e) => e.preventDefault()}
      onClick={onClick}
      className={`px-2 py-1 rounded text-xs disabled:opacity-40 ${active ? 'bg-gray-600 text-white' : 'text-gray-300 hover:bg-gray-700'}`}
    >
      {text}
    </button>
  );
}

/**
 * Rich text bound to the shared `Y.Doc`: every keystroke is a Yjs change, so
 * edits from other people merge into the text as it is open. Undo is Yjs's
 * (it only undoes this person's own changes), hence no ProseMirror history.
 * It answers the toolbar, Cmd/Ctrl+Z and the Edit menu, which reaches the
 * page as a `historyUndo`/`historyRedo` input event rather than a key press.
 */
export function DocEditor({ doc, editable, onEditor }: DocEditorProps) {
  const { t } = useTranslation(['documents']);
  const editor = useEditor(
    {
      editable,
      // The toolbar shows the marks at the caret, so it follows every change.
      shouldRerenderOnTransaction: true,
      extensions: [...docSchemaExtensions, Collaboration.configure({ document: doc, field: DOC_FIELD })],
      editorProps: {
        handleDOMEvents: {
          beforeinput: (view, event) => {
            const type = (event as InputEvent).inputType;
            if (type !== 'historyUndo' && type !== 'historyRedo') return false;
            event.preventDefault();
            if (type === 'historyUndo') undo(view.state);
            else redo(view.state);
            return true;
          },
        },
        attributes: {
          class: 'eo-doc max-w-none min-h-[60vh] text-sm text-gray-200 focus:outline-none px-6 py-4',
          'data-testid': 'shared-doc-editor',
        },
      },
    },
    [doc],
  );

  useEffect(() => {
    editor?.setEditable(editable);
  }, [editor, editable]);

  useEffect(() => {
    onEditor(editor);
    return () => onEditor(null);
  }, [editor, onEditor]);

  if (!editor) return null;
  const run = (fn: (e: Editor) => void) => () => fn(editor);

  return (
    <div className="flex flex-col flex-1 min-h-0">
      {editable && (
        <div className="flex gap-1 px-4 py-2 border-b border-gray-700">
          <ToolbarButton
            label={t('documents:toolbar.undo')}
            text="↶"
            active={false}
            disabled={!editor.can().undo()}
            onClick={run((e) => e.chain().focus().undo().run())}
          />
          <ToolbarButton
            label={t('documents:toolbar.redo')}
            text="↷"
            active={false}
            disabled={!editor.can().redo()}
            onClick={run((e) => e.chain().focus().redo().run())}
          />
          <span className="mx-1 w-px bg-gray-700" />
          <ToolbarButton
            label={t('documents:toolbar.bold')}
            text="B"
            active={editor.isActive('bold')}
            onClick={run((e) => e.chain().focus().toggleBold().run())}
          />
          <ToolbarButton
            label={t('documents:toolbar.italic')}
            text="I"
            active={editor.isActive('italic')}
            onClick={run((e) => e.chain().focus().toggleItalic().run())}
          />
          <ToolbarButton
            label={t('documents:toolbar.underline')}
            text="U"
            active={editor.isActive('underline')}
            onClick={run((e) => e.chain().focus().toggleUnderline().run())}
          />
          <ToolbarButton
            label={t('documents:toolbar.heading')}
            text="H"
            active={editor.isActive('heading', { level: 2 })}
            onClick={run((e) => e.chain().focus().toggleHeading({ level: 2 }).run())}
          />
          <ToolbarButton
            label={t('documents:toolbar.bulletList')}
            text="•"
            active={editor.isActive('bulletList')}
            onClick={run((e) => e.chain().focus().toggleBulletList().run())}
          />
          <ToolbarButton
            label={t('documents:toolbar.orderedList')}
            text="1."
            active={editor.isActive('orderedList')}
            onClick={run((e) => e.chain().focus().toggleOrderedList().run())}
          />
        </div>
      )}
      <div className="flex-1 overflow-y-auto">
        <EditorContent editor={editor} />
      </div>
    </div>
  );
}
