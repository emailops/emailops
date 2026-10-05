import { describe, expect, it } from 'vitest';
import { type DocNode, docContent, pdfDefinition, pdfFilename, renderPdf, sheetContent } from './docPdf';

const text = (t: string, marks?: { type: string; attrs?: Record<string, unknown> }[]) => ({
  type: 'text',
  text: t,
  marks,
});

describe('docContent', () => {
  it('turns headings and paragraphs into styled blocks', () => {
    const content = docContent({
      type: 'doc',
      content: [
        { type: 'heading', attrs: { level: 2 }, content: [text('Plan')] },
        { type: 'paragraph', content: [text('Hello '), text('world', [{ type: 'bold' }])] },
      ],
    });
    expect(content).toEqual([
      { text: [{ text: 'Plan' }], style: 'h2' },
      { text: [{ text: 'Hello ' }, { text: 'world', bold: true }], style: 'p' },
    ]);
  });

  it('keeps italic, underline and links', () => {
    const [p] = docContent({
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            text('a', [{ type: 'italic' }]),
            text('b', [{ type: 'underline' }]),
            text('c', [{ type: 'link', attrs: { href: 'https://example.com' } }]),
          ],
        },
      ],
    });
    expect(p).toEqual({
      text: [
        { text: 'a', italics: true },
        { text: 'b', decoration: 'underline' },
        { text: 'c', link: 'https://example.com', decoration: 'underline', color: '#1d4ed8' },
      ],
      style: 'p',
    });
  });

  it('turns bulleted and numbered lists into lists, nested ones included', () => {
    const item = (t: string, nested?: DocNode): DocNode => ({
      type: 'listItem',
      content: [{ type: 'paragraph', content: [text(t)] }, ...(nested ? [nested] : [])],
    });
    const content = docContent({
      type: 'doc',
      content: [
        { type: 'bulletList', content: [item('one', { type: 'orderedList', content: [item('inner')] }), item('two')] },
      ],
    });
    expect(content).toEqual([
      {
        ul: [
          [
            { text: [{ text: 'one' }], style: 'p' },
            { ol: [[{ text: [{ text: 'inner' }], style: 'p' }]], style: 'list' },
          ],
          [{ text: [{ text: 'two' }], style: 'p' }],
        ],
        style: 'list',
      },
    ]);
  });

  it('turns a table into a table with its header row', () => {
    const cell = (type: string, t: string) => ({ type, content: [{ type: 'paragraph', content: [text(t)] }] });
    const [table] = docContent({
      type: 'doc',
      content: [
        {
          type: 'table',
          content: [
            { type: 'tableRow', content: [cell('tableHeader', 'Item'), cell('tableHeader', 'Cost')] },
            { type: 'tableRow', content: [cell('tableCell', 'Desk'), cell('tableCell', '120')] },
          ],
        },
      ],
    });
    expect(table).toMatchObject({ table: { headerRows: 1, widths: ['*', '*'] }, style: 'table' });
    expect((table as { table: { body: unknown[][] } }).table.body[1][0]).toEqual([
      { text: [{ text: 'Desk' }], style: 'p' },
    ]);
  });

  it('keeps inline images and leaves out remote ones', () => {
    const content = docContent({
      type: 'doc',
      content: [
        { type: 'image', attrs: { src: 'data:image/png;base64,AAAA' } },
        { type: 'image', attrs: { src: 'https://tracker.example/p.png' } },
      ],
    });
    expect(content).toEqual([{ image: 'data:image/png;base64,AAAA', fit: [480, 640], style: 'image' }]);
  });
});

describe('sheetContent', () => {
  it('is one table of what the cells show, without the empty rows and columns after the last value', () => {
    const content = sheetContent(
      [
        ['Item', 'Cost', ''],
        ['Desk', '120', ''],
        ['Total', '=SUM(B2:B2)', ''],
        ['', '', ''],
      ],
      'en',
    );
    expect(content).toEqual([
      {
        table: {
          headerRows: 1,
          body: [
            [
              { text: 'Item', bold: true },
              { text: 'Cost', bold: true },
            ],
            ['Desk', '120'],
            ['Total', '120'],
          ],
        },
        style: 'table',
      },
    ]);
  });

  it('is nothing for an empty sheet', () => {
    expect(sheetContent([['', '']], 'en')).toEqual([]);
  });
});

describe('pdfFilename', () => {
  it('is the title as a file name', () => {
    expect(pdfFilename('Q3 budget')).toBe('Q3 budget.pdf');
    expect(pdfFilename('a/b: c?')).toBe('a-b- c-.pdf');
    expect(pdfFilename('   ')).toBe('EO Docs.pdf');
  });
});

describe('renderPdf', () => {
  it('lays out everything a document can hold', async () => {
    const para = (t: string) => ({ type: 'paragraph', content: [text(t)] });
    const content = docContent({
      type: 'doc',
      content: [
        { type: 'heading', attrs: { level: 1 }, content: [text('Plan')] },
        { type: 'bulletList', content: [{ type: 'listItem', content: [para('one')] }] },
        { type: 'orderedList', content: [{ type: 'listItem', content: [para('two')] }] },
        {
          type: 'table',
          content: [{ type: 'tableRow', content: [{ type: 'tableHeader', content: [para('h')] }] }],
        },
        {
          type: 'image',
          attrs: {
            src: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
          },
        },
        { type: 'horizontalRule' },
      ],
    });
    const bytes = await renderPdf(pdfDefinition('Plan', content));
    expect(new TextDecoder().decode(bytes.slice(0, 5))).toBe('%PDF-');
  });

  it('produces a PDF file', async () => {
    const bytes = await renderPdf(pdfDefinition('Plan', [{ text: 'Hello', style: 'p' }]));
    expect(new TextDecoder().decode(bytes.slice(0, 5))).toBe('%PDF-');
  });
});
