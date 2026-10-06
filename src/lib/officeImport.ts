import { generateJSON, getSchema } from '@tiptap/core';
import { prosemirrorJSONToYXmlFragment } from '@tiptap/y-tiptap';
import * as Y from 'yjs';
import * as api from '@/lib/api';
import { DOC_FIELD, docSchemaExtensions } from '@/lib/docSchema';
import { bytesFromBase64, bytesToBase64 } from '@/lib/yjsBytes';
import type { SharedDoc } from '@/types';

/** Images larger than this are left out: a document travels by email whole. */
const MAX_IMAGE_CHARS = 1_000_000;

export type ImportKind = 'doc' | 'sheet';

/** What EO Docs makes of a file, by its extension; `null` when it cannot import it. */
export function importKind(filename: string): ImportKind | null {
  const ext = filename.toLowerCase().split('.').pop() ?? '';
  if (ext === 'docx') return 'doc';
  if (['xlsx', 'xlsm', 'xls', 'ods'].includes(ext)) return 'sheet';
  return null;
}

async function mammothHtml(data: ArrayBuffer): Promise<string> {
  const mammoth = await import('mammoth');
  const result = await mammoth.convertToHtml({ arrayBuffer: data });
  return result.value;
}

/** A .docx as HTML in the doc editor's terms, without images too large to mail. */
export async function docxToHtml(
  data: ArrayBuffer,
  convert: (data: ArrayBuffer) => Promise<string> = mammothHtml,
): Promise<{ html: string; skippedImages: number }> {
  const parsed = new DOMParser().parseFromString(await convert(data), 'text/html');
  let skippedImages = 0;
  for (const img of Array.from(parsed.querySelectorAll('img'))) {
    if ((img.getAttribute('src') ?? '').length > MAX_IMAGE_CHARS) {
      img.remove();
      skippedImages += 1;
    }
  }
  return { html: parsed.body.innerHTML, skippedImages };
}

/** HTML as the Yjs update of a new EO Docs text. */
export function htmlToDocUpdate(html: string): Uint8Array {
  const json = generateJSON(html, docSchemaExtensions);
  const ydoc = new Y.Doc();
  prosemirrorJSONToYXmlFragment(getSchema(docSchemaExtensions), json, ydoc.getXmlFragment(DOC_FIELD));
  return Y.encodeStateAsUpdate(ydoc);
}

function titleOf(filename: string): string {
  const stem = filename.replace(/\.[^.]+$/, '').trim();
  return stem || filename;
}

export interface ImportResult {
  docs: SharedDoc[];
  /** Images left out of a Word file for being too large to mail. */
  skippedImages: number;
}

/** Import a Word or spreadsheet file (standard base64) into EO Docs. */
export async function importOfficeFile(
  accountId: string,
  filename: string,
  data: string,
  folderId: string | null,
): Promise<ImportResult> {
  const kind = importKind(filename);
  if (kind === 'sheet') {
    return { docs: await api.importSpreadsheet(accountId, filename, data, folderId), skippedImages: 0 };
  }
  if (kind !== 'doc') throw new Error(`EO Docs cannot import ${filename}`);
  const bytes = bytesFromBase64(data);
  const { html, skippedImages } = await docxToHtml(
    bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length) as ArrayBuffer,
  );
  const update = htmlToDocUpdate(html);
  let doc = await api.createSharedDoc(accountId, 'doc', titleOf(filename));
  await api.applySharedDocUpdate(accountId, doc.id, bytesToBase64(update));
  if (folderId) doc = await api.moveSharedDoc(accountId, doc.id, folderId);
  return { docs: [doc], skippedImages };
}
