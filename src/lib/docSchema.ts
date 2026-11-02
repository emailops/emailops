import Image from '@tiptap/extension-image';
import { Link } from '@tiptap/extension-link';
import { Table, TableCell, TableHeader, TableRow } from '@tiptap/extension-table';
import type { Extensions } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';

/**
 * Only images carried in the document get an address: a remote one would be a
 * request to someone else's server, made by opening a document anyone can send.
 */
const InlineImage = Image.extend({
  addAttributes() {
    const attributes = this.parent?.() ?? {};
    return {
      ...attributes,
      src: {
        ...attributes.src,
        renderHTML: ({ src }) => (typeof src === 'string' && src.startsWith('data:image/') ? { src } : {}),
      },
    };
  },
});

/** The field of the Yjs document an EO Docs text lives in. */
export const DOC_FIELD = 'body';

/**
 * A link without a stored `target`: how it opens is not part of the document.
 * The rendered one goes too (see `docSchemaExtensions`): tauri-plugin-shell
 * opens any click on a `target="_blank"` link by itself, past the confirmation.
 */
const DocLink = Link.extend({
  addAttributes() {
    const { target: _target, ...attributes } = this.parent?.() ?? {};
    return attributes;
  },
});

/**
 * The EO Docs text schema, without React or Yjs, so imports build documents
 * headlessly with exactly what the editor can show: text with headings,
 * marks, lists and links, tables, and images (inline data). Undo comes from
 * Yjs in the editor, so ProseMirror's own history is off.
 */
export const docSchemaExtensions: Extensions = [
  StarterKit.configure({ undoRedo: false, link: false }),
  DocLink.configure({ openOnClick: false, HTMLAttributes: { target: null } }),
  Table.configure({ resizable: false }),
  TableRow,
  TableHeader,
  TableCell,
  InlineImage.configure({ allowBase64: true }),
];
