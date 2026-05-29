/**
 * Rich text editor used by all three compose surfaces (ComposeModal,
 * ComposeTabView, ReplyCompose).
 *
 * Built on Tiptap (ProseMirror). Supports:
 *   - bold / italic / underline / strike
 *   - bullet + ordered lists
 *   - block quote
 *   - links (with prompt-based UI)
 *   - inline images via paste / drag-drop (stored as data: URLs in the
 *     editor; converted to cid: + attachments by `prepareOutgoingHtml`
 *     just before send)
 *
 * The editor outputs HTML via `getHTML()`. The parent owns the string in
 * state and passes it back as `value`. We deliberately do NOT echo every
 * keystroke back as `value` from parent → editor (that fights ProseMirror's
 * selection); the editor is the source of truth while focused, and we only
 * reset its content when `value` changes externally (e.g. AI draft arrived).
 */

import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import Underline from '@tiptap/extension-underline';
import { type Editor, EditorContent, useEditor } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

export interface RichTextEditorProps {
  /** HTML content. The editor is updated when this changes externally. */
  value: string;
  onChange: (html: string) => void;
  placeholder?: string;
  disabled?: boolean;
  /** Extra classes for the editor's contenteditable surface. */
  contentClassName?: string;
}

export function RichTextEditor({
  value,
  onChange,
  placeholder,
  disabled = false,
  contentClassName,
}: RichTextEditorProps) {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // We want StarterKit defaults but explicit about a few things.
        heading: { levels: [1, 2, 3] },
      }),
      Underline,
      Link.configure({
        openOnClick: false,
        autolink: true,
        HTMLAttributes: { rel: 'noopener noreferrer' },
      }),
      // Inline images. Tiptap stores them as data: URLs until we extract
      // them at send time via `prepareOutgoingHtml`.
      Image.configure({
        allowBase64: true,
        HTMLAttributes: { class: 'max-w-full h-auto rounded' },
      }),
    ],
    content: value,
    editable: !disabled,
    onUpdate({ editor }) {
      onChange(editor.getHTML());
    },
    editorProps: {
      attributes: {
        class: ['prose prose-sm max-w-none focus:outline-none min-h-[120px] px-3 py-2', contentClassName ?? ''].join(
          ' ',
        ),
        ...(placeholder ? { 'data-placeholder': placeholder } : {}),
      },
      handlePaste(view, event) {
        const items = event.clipboardData?.items;
        if (!items) return false;
        for (const item of Array.from(items)) {
          if (item.kind === 'file' && item.type.startsWith('image/')) {
            const file = item.getAsFile();
            if (!file) continue;
            event.preventDefault();
            void readFileAsDataUrl(file).then((dataUrl) => {
              view.dispatch(
                view.state.tr.replaceSelectionWith(
                  view.state.schema.nodes.image.create({ src: dataUrl, alt: file.name }),
                ),
              );
            });
            return true;
          }
        }
        return false;
      },
      handleDrop(view, event) {
        const files = event.dataTransfer?.files;
        if (!files || files.length === 0) return false;
        const images = Array.from(files).filter((f) => f.type.startsWith('image/'));
        if (images.length === 0) return false;
        event.preventDefault();
        for (const file of images) {
          void readFileAsDataUrl(file).then((dataUrl) => {
            view.dispatch(
              view.state.tr.replaceSelectionWith(
                view.state.schema.nodes.image.create({ src: dataUrl, alt: file.name }),
              ),
            );
          });
        }
        return true;
      },
    },
  });

  // When the parent updates `value` externally (e.g. AI draft replaces the
  // placeholder), reset the editor content. We compare against the current
  // editor HTML to avoid loops with our own onUpdate callback.
  const lastExternalValue = useRef(value);
  useEffect(() => {
    if (!editor) return;
    if (value === lastExternalValue.current) return;
    lastExternalValue.current = value;
    if (editor.getHTML() !== value) {
      editor.commands.setContent(value, { emitUpdate: false });
    }
  }, [editor, value]);

  useEffect(() => {
    if (!editor) return;
    editor.setEditable(!disabled);
  }, [editor, disabled]);

  if (!editor) {
    return <div className="w-full rounded-lg border border-gray-300 bg-white min-h-[120px] animate-pulse" />;
  }

  return (
    <div className="rounded-lg border border-gray-300 bg-white focus-within:border-primary-500 focus-within:ring-2 focus-within:ring-primary-100">
      <Toolbar editor={editor} disabled={disabled} />
      <EditorContent editor={editor} />
    </div>
  );
}

interface ToolbarProps {
  editor: Editor;
  disabled: boolean;
}

function Toolbar({ editor, disabled }: ToolbarProps) {
  const { t } = useTranslation(['common']);
  // `window.prompt` is unreliable in the Tauri webview (and ugly in any
  // case), so the Link button toggles an inline popover instead.
  const [linkPopoverOpen, setLinkPopoverOpen] = useState(false);
  const [linkUrl, setLinkUrl] = useState('');
  const linkInputRef = useRef<HTMLInputElement | null>(null);

  // Focus the URL input when the popover opens. We avoid the `autoFocus`
  // attribute (a11y lint blocks it because it can disorient screen readers);
  // imperatively focusing only on the open transition keeps the lint happy
  // while preserving the expected UX.
  useEffect(() => {
    if (linkPopoverOpen) {
      linkInputRef.current?.focus();
    }
  }, [linkPopoverOpen]);

  const openLinkPopover = () => {
    const prev = (editor.getAttributes('link').href as string | undefined) ?? '';
    setLinkUrl(prev);
    setLinkPopoverOpen(true);
  };

  const applyLink = () => {
    const url = linkUrl.trim();
    if (url === '') {
      editor.chain().focus().extendMarkRange('link').unsetLink().run();
    } else {
      editor.chain().focus().extendMarkRange('link').setLink({ href: url }).run();
    }
    setLinkPopoverOpen(false);
  };

  const btn = (label: string, title: string, active: boolean, onClick: () => void, enabled = true) => (
    <button
      type="button"
      title={title}
      onClick={onClick}
      disabled={disabled || !enabled}
      className={`px-2 py-1 rounded text-xs font-medium border transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
        active
          ? 'bg-primary-100 text-primary-700 border-primary-200'
          : 'bg-white text-gray-700 border-transparent hover:bg-gray-100'
      }`}
    >
      {label}
    </button>
  );

  return (
    <div className="relative flex flex-wrap items-center gap-0.5 px-2 py-1.5 border-b border-gray-200 bg-gray-50 rounded-t-lg">
      {btn('B', 'Bold (⌘B)', editor.isActive('bold'), () => editor.chain().focus().toggleBold().run())}
      {btn('I', 'Italic (⌘I)', editor.isActive('italic'), () => editor.chain().focus().toggleItalic().run())}
      {btn('U', 'Underline (⌘U)', editor.isActive('underline'), () => editor.chain().focus().toggleUnderline().run())}
      {btn('S', 'Strikethrough', editor.isActive('strike'), () => editor.chain().focus().toggleStrike().run())}
      <span className="w-px h-4 bg-gray-300 mx-1" />
      {btn('• List', 'Bullet list', editor.isActive('bulletList'), () =>
        editor.chain().focus().toggleBulletList().run(),
      )}
      {btn('1. List', 'Numbered list', editor.isActive('orderedList'), () =>
        editor.chain().focus().toggleOrderedList().run(),
      )}
      {btn('❝', 'Quote', editor.isActive('blockquote'), () => editor.chain().focus().toggleBlockquote().run())}
      <span className="w-px h-4 bg-gray-300 mx-1" />
      {btn('Link', 'Insert link', editor.isActive('link') || linkPopoverOpen, openLinkPopover)}

      {linkPopoverOpen && (
        <div className="absolute top-full left-2 z-20 mt-1 flex items-center gap-1 rounded-lg border border-gray-200 bg-white p-1.5 shadow-lg">
          <input
            type="url"
            ref={linkInputRef}
            value={linkUrl}
            onChange={(e) => setLinkUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                applyLink();
              } else if (e.key === 'Escape') {
                e.preventDefault();
                setLinkPopoverOpen(false);
              }
            }}
            placeholder="https://example.com" // i18n-ignore: example URL the user replaces with their own link
            className="w-64 rounded border border-gray-300 px-2 py-1 text-xs outline-none focus:border-primary-500"
          />
          <button
            type="button"
            onClick={applyLink}
            className="rounded bg-primary-600 px-2 py-1 text-xs font-medium text-white hover:bg-primary-700"
          >
            Apply
          </button>
          {editor.isActive('link') && (
            <button
              type="button"
              onClick={() => {
                editor.chain().focus().extendMarkRange('link').unsetLink().run();
                setLinkPopoverOpen(false);
              }}
              className="rounded border border-gray-300 px-2 py-1 text-xs font-medium text-gray-700 hover:bg-gray-100"
            >
              Remove
            </button>
          )}
          <button
            type="button"
            onClick={() => setLinkPopoverOpen(false)}
            className="rounded px-1.5 py-1 text-xs text-gray-400 hover:bg-gray-100 hover:text-gray-700"
            title={t('common:actions.close')}
          >
            ×
          </button>
        </div>
      )}
    </div>
  );
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}
