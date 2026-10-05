// Importing Word and Excel files into EO Docs. Word is converted here
// (mammoth → the doc editor's schema → a Yjs update); spreadsheets go to the
// backend, which reads them with calamine.

import { describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

// The app gets mammoth's browser build (Vite honours its `browser` field);
// vitest resolves the Node build, which wants a path or a Buffer. Use the
// same build as the app.
vi.mock('mammoth', async () => {
  // @ts-expect-error -- the UMD browser build ships without type declarations
  const browser = await import('mammoth/mammoth.browser.js');
  return { ...(browser.default ?? browser), default: browser.default ?? browser };
});
vi.mock('@/lib/api', () => ({
  createSharedDoc: vi.fn(async (_a: string, kind: string, title: string) => ({ id: 'new-doc', kind, title })),
  applySharedDocUpdate: vi.fn(async () => {}),
  moveSharedDoc: vi.fn(async (_a: string, id: string, folderId: string) => ({ id, folderId })),
  importSpreadsheet: vi.fn(async () => [{ id: 's1' }, { id: 's2' }]),
}));

import * as api from '@/lib/api';
import { bytesFromBase64 } from '@/lib/yjsBytes';
import PLAN_DOCX_RAW from './fixtures/plan.docx.b64?raw';
import { docxToHtml, htmlToDocUpdate, importKind, importOfficeFile } from './officeImport';

/** The synthetic Word fixture (a heading, bold, a list and a table), standard base64. */
const PLAN_DOCX_B64 = PLAN_DOCX_RAW.trim();

function bodyOf(update: Uint8Array): string {
  const doc = new Y.Doc();
  Y.applyUpdate(doc, update);
  return doc.getXmlFragment('body').toString();
}

describe('importKind', () => {
  it('knows Word and spreadsheet files by extension', () => {
    expect(['a.docx', 'B.DOCX', 'c.xlsx', 'd.xlsm', 'e.xls', 'f.ods', 'g.pdf', 'h.doc'].map(importKind)).toEqual([
      'doc',
      'doc',
      'sheet',
      'sheet',
      'sheet',
      'sheet',
      null,
      null,
    ]);
  });
});

describe('Word import', () => {
  it('keeps headings, bold, lists and tables', async () => {
    // Through base64, as the app receives a file, so the buffer is this realm's.
    const bytes = bytesFromBase64(PLAN_DOCX_B64);
    const { html } = await docxToHtml(bytes.buffer.slice(0) as ArrayBuffer);
    const body = bodyOf(htmlToDocUpdate(html));
    expect(body).toContain('<heading level="1">Trip plan</heading>');
    expect(body).toContain('<bold>Tuesday</bold>');
    expect(body).toContain('<bulletlist>');
    expect(body).toContain('Print the badges');
    expect(body).toContain('<table>');
    expect(body).toContain('Flights');
  });

  it('drops images too large to travel by email and says how many', async () => {
    const big = `<p>x</p><img src="data:image/png;base64,${'A'.repeat(2_000_000)}"><img src="data:image/png;base64,AAAA">`;
    const { html, skippedImages } = await docxToHtml(new ArrayBuffer(0), async () => big);
    expect(skippedImages).toBe(1);
    expect(html).toContain('base64,AAAA');
  });

  it('creates the document with the converted content, in the open folder', async () => {
    const data = PLAN_DOCX_B64;
    const result = await importOfficeFile('acc-1', 'Trip plan.docx', data, 'folder-1');
    expect(api.createSharedDoc).toHaveBeenCalledWith('acc-1', 'doc', 'Trip plan');
    const [, docId, update] = vi.mocked(api.applySharedDocUpdate).mock.calls[0];
    expect(docId).toBe('new-doc');
    expect(bodyOf(bytesFromBase64(update))).toContain('Trip plan');
    expect(api.moveSharedDoc).toHaveBeenCalledWith('acc-1', 'new-doc', 'folder-1');
    expect(result.docs.map((d) => d.id)).toEqual(['new-doc']);
  });
});

describe('spreadsheet import', () => {
  it('hands the file to the backend', async () => {
    const result = await importOfficeFile('acc-1', 'budget.xlsx', 'QUJD', null);
    expect(api.importSpreadsheet).toHaveBeenCalledWith('acc-1', 'budget.xlsx', 'QUJD', null);
    expect(result.docs.map((d) => d.id)).toEqual(['s1', 's2']);
  });

  it('refuses a file it cannot import', async () => {
    await expect(importOfficeFile('acc-1', 'notes.pdf', 'QUJD', null)).rejects.toThrow();
  });
});
