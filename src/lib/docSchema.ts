import Image from '@tiptap/extension-image';
import { Table, TableCell, TableHeader, TableRow } from '@tiptap/extension-table';
import type { Extensions } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';

/** The field of the Yjs document an EO Docs text lives in. */
export const DOC_FIELD = 'body';

/**
 * The EO Docs text schema, without React or Yjs, so imports build documents
 * headlessly with exactly what the editor can show: text with headings,
 * marks, lists and links, tables, and images (inline data). Undo comes from
 * Yjs in the editor, so ProseMirror's own history is off.
 */
export const docSchemaExtensions: Extensions = [
  StarterKit.configure({ undoRedo: false, link: { openOnClick: false } }),
  Table.configure({ resizable: false }),
  TableRow,
  TableHeader,
  TableCell,
  Image.configure({ allowBase64: true }),
];
