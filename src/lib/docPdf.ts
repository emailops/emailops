import type { Content, TDocumentDefinitions, TVirtualFileSystem } from 'pdfmake/interfaces';
import { displayValue } from '@/lib/sheetFormula';

/** A TipTap document node, as `editor.getJSON()` returns it. */
export interface DocNode {
  type: string;
  attrs?: Record<string, unknown>;
  content?: DocNode[];
  text?: string;
  marks?: { type: string; attrs?: Record<string, unknown> }[];
}

type Run = { text: string; bold?: boolean; italics?: boolean; decoration?: 'underline'; link?: string; color?: string };

/** Pure: a paragraph's text runs, with the marks pdfmake understands. */
function runs(nodes: DocNode[] = []): Run[] {
  return nodes.flatMap((node): Run[] => {
    if (node.type === 'hardBreak') return [{ text: '\n' }];
    if (node.type !== 'text' || !node.text) return [];
    const run: Run = { text: node.text };
    for (const mark of node.marks ?? []) {
      if (mark.type === 'bold') run.bold = true;
      if (mark.type === 'italic') run.italics = true;
      if (mark.type === 'underline') run.decoration = 'underline';
      if (mark.type === 'link' && typeof mark.attrs?.href === 'string') {
        run.link = mark.attrs.href;
        run.decoration = 'underline';
        run.color = '#1d4ed8';
      }
    }
    return [run];
  });
}

/** Pure: the blocks of a list item or table cell, as pdfmake content. */
function blocks(nodes: DocNode[] = []): Content[] {
  return nodes.flatMap((node): Content[] => {
    switch (node.type) {
      case 'paragraph':
        return [{ text: runs(node.content), style: 'p' }];
      case 'heading':
        return [{ text: runs(node.content), style: `h${Math.min(Number(node.attrs?.level) || 1, 3)}` }];
      case 'blockquote':
        return [{ stack: blocks(node.content), style: 'quote' }];
      case 'codeBlock':
        return [{ text: runs(node.content), style: 'code' }];
      case 'bulletList':
        return [{ ul: (node.content ?? []).map((item) => blocks(item.content)), style: 'list' }];
      case 'orderedList':
        return [{ ol: (node.content ?? []).map((item) => blocks(item.content)), style: 'list' }];
      case 'table': {
        const rows = (node.content ?? []).map((row) => (row.content ?? []).map((cell) => blocks(cell.content)));
        const columns = Math.max(0, ...rows.map((r) => r.length));
        const header = node.content?.[0]?.content?.every((cell) => cell.type === 'tableHeader') ?? false;
        return [
          {
            table: {
              headerRows: header ? 1 : 0,
              widths: Array(columns).fill('*'),
              body: rows.map((r) => [...r, ...Array(columns - r.length).fill('')]),
            },
            style: 'table',
          },
        ];
      }
      case 'image': {
        // Only images carried in the document itself: a remote one would be a
        // request to someone else's server.
        const src = node.attrs?.src;
        return typeof src === 'string' && src.startsWith('data:image/')
          ? [{ image: src, fit: [480, 640], style: 'image' }]
          : [];
      }
      case 'horizontalRule':
        return [{ canvas: [{ type: 'line', x1: 0, y1: 0, x2: 480, y2: 0, lineWidth: 0.5 }], style: 'rule' }];
      default:
        return [];
    }
  });
}

/** Pure: an EO Docs text document (`editor.getJSON()`) as pdfmake content. */
export function docContent(doc: DocNode): Content[] {
  return blocks(doc.content);
}

/** Pure: a sheet as one table of what its cells show (formulas worked out),
 *  without the empty rows and columns after the last value. */
export function sheetContent(values: string[][], locale: string): Content[] {
  const rows = values.reduce((n, row, r) => (row.some((v) => v) ? r + 1 : n), 0);
  const cols = values.reduce((n, row) => row.reduce((m, v, c) => (v ? Math.max(m, c + 1) : m), n), 0);
  if (rows === 0) return [];
  const body: Content[][] = values
    .slice(0, rows)
    .map((row, r) => row.slice(0, cols).map((_, c) => displayValue(values, r, c, locale)));
  // The first row is the header, as the sheet's filters take it.
  body[0] = body[0].map((text) => ({ text: String(text), bold: true }));
  return [{ table: { headerRows: 1, body }, style: 'table' }];
}

/** Pure: the whole PDF: the title, then the content. */
export function pdfDefinition(title: string, content: Content[]): TDocumentDefinitions {
  return {
    info: { title },
    pageMargins: [50, 50, 50, 50],
    content: [{ text: title, style: 'title' }, ...content],
    defaultStyle: { fontSize: 11, lineHeight: 1.25 },
    styles: {
      title: { fontSize: 20, bold: true, margin: [0, 0, 0, 14] },
      h1: { fontSize: 18, bold: true, margin: [0, 12, 0, 6] },
      h2: { fontSize: 15, bold: true, margin: [0, 10, 0, 5] },
      h3: { fontSize: 12, bold: true, margin: [0, 8, 0, 4] },
      p: { margin: [0, 0, 0, 6] },
      list: { margin: [0, 0, 0, 6] },
      quote: { italics: true, margin: [16, 0, 0, 6] },
      code: { fontSize: 9, margin: [0, 0, 0, 6] },
      table: { fontSize: 10, margin: [0, 4, 0, 10] },
      image: { margin: [0, 4, 0, 8] },
      rule: { margin: [0, 6, 0, 6] },
    },
  };
}

/** Pure: the file name a document's PDF is saved under. */
export function pdfFilename(title: string): string {
  const name = title.replace(/[/\\:*?"<>|]/g, '-').trim();
  return `${name || 'EO Docs'}.pdf`;
}

/** Lay out and write the PDF. pdfmake and its fonts (~2 MB) load on first use. */
export async function renderPdf(definition: TDocumentDefinitions): Promise<Uint8Array> {
  const [{ default: pdfMake }, fonts] = await Promise.all([
    import('pdfmake/build/pdfmake'),
    import('pdfmake/build/vfs_fonts'),
  ]);
  // The font bundle is a CommonJS module: its object is the default export.
  const vfs = fonts as unknown as { default?: TVirtualFileSystem } & TVirtualFileSystem;
  pdfMake.addVirtualFileSystem(vfs.default ?? vfs);
  const buffer = await pdfMake.createPdf(definition).getBuffer();
  return new Uint8Array(buffer);
}
