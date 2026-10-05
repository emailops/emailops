import { Editor } from '@tiptap/react';
import { describe, expect, it } from 'vitest';
import { composeEditorExtensions } from './composeEditorExtensions';

function createEditor(html: string): Editor {
  return new Editor({ element: null, extensions: composeEditorExtensions, content: html });
}

/** Load HTML into the compose schema and read it back, as opening + saving a draft does. */
function roundTrip(html: string): string {
  const editor = createEditor(html);
  const out = editor.getHTML();
  editor.destroy();
  return out;
}

function parse(html: string): HTMLElement {
  return new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html').body;
}

function attrs(el: Element | null): Record<string, string> {
  if (!el) throw new Error('element not found');
  return Object.fromEntries(Array.from(el.attributes).map((a) => [a.name, a.value]));
}

const TABLE =
  '<table width="600" border="1" cellpadding="4" cellspacing="0" bgcolor="#ffffff" align="center" style="border-collapse: collapse;">' +
  '<tbody>' +
  '<tr bgcolor="#eeeeee" style="height: 30px;"><th colspan="2" align="left">Plan</th></tr>' +
  '<tr><td rowspan="2" width="200" valign="top" bgcolor="#f0f0f0" style="padding: 4px; border: 1px solid rgb(204, 204, 204);">Alpha</td><td>Beta</td></tr>' +
  '<tr><td>Gamma</td></tr>' +
  '</tbody></table>';

describe('composeEditorExtensions — tables', () => {
  it('keeps the table structure and its layout attributes', () => {
    const body = parse(roundTrip(TABLE));

    expect(attrs(body.querySelector('table'))).toEqual({
      width: '600',
      border: '1',
      cellpadding: '4',
      cellspacing: '0',
      bgcolor: '#ffffff',
      align: 'center',
      style: 'border-collapse: collapse;',
    });
    expect(body.querySelectorAll('tr')).toHaveLength(3);
    expect(attrs(body.querySelector('tr'))).toEqual({ bgcolor: '#eeeeee', style: 'height: 30px;' });
    expect(attrs(body.querySelector('th'))).toEqual({ colspan: '2', rowspan: '1', align: 'left' });
    expect(attrs(body.querySelector('td'))).toEqual({
      colspan: '1',
      rowspan: '2',
      width: '200',
      valign: 'top',
      bgcolor: '#f0f0f0',
      style: 'padding: 4px; border: 1px solid rgb(204, 204, 204);',
    });
    expect(body.textContent).toBe('PlanAlphaBetaGamma');
  });

  it('adds no editor-only markup to the table', () => {
    const out = roundTrip('<table><tbody><tr><td>One</td><td>Two</td></tr></tbody></table>');
    expect(out).toBe(
      '<table><tbody><tr><td colspan="1" rowspan="1"><p>One</p></td><td colspan="1" rowspan="1"><p>Two</p></td></tr></tbody></table>',
    );
  });

  it('keeps the table when text inside a cell is edited', () => {
    const editor = createEditor(TABLE);
    let betaEnd = -1;
    editor.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === 'Beta') betaEnd = pos + node.nodeSize;
    });
    editor.commands.insertContentAt(betaEnd, ' edited');
    const body = parse(editor.getHTML());
    editor.destroy();

    expect(body.querySelectorAll('td')[1].textContent).toBe('Beta edited');
    expect(body.querySelector('table')?.getAttribute('width')).toBe('600');
    expect(body.querySelector('td')?.getAttribute('style')).toBe('padding: 4px; border: 1px solid rgb(204, 204, 204);');
    expect(body.querySelectorAll('tr')).toHaveLength(3);
  });

  it('keeps a table nested in a cell', () => {
    const body = parse(
      roundTrip(
        '<table><tbody><tr><td><table bgcolor="#eeeeee"><tbody><tr><td>Inner</td></tr></tbody></table></td></tr></tbody></table>',
      ),
    );
    expect(body.querySelector('td table')?.getAttribute('bgcolor')).toBe('#eeeeee');
    expect(body.querySelector('td table td')?.textContent).toBe('Inner');
  });
});

describe('composeEditorExtensions — inline styles', () => {
  it('keeps colour, background colour, font size and font family on text', () => {
    const html =
      '<p><span style="color: rgb(255, 0, 0);">red</span> <span style="background-color: rgb(255, 255, 0);">marked</span> ' +
      '<span style="font-size: 18px; font-family: Georgia, serif;">big</span></p>';
    expect(roundTrip(html)).toBe(html);
  });

  it('keeps a styled span together with bold and links', () => {
    const html =
      '<p><a rel="noopener noreferrer" href="https://example.com"><strong><span style="color: rgb(0, 128, 0);">go</span></strong></a></p>';
    const body = parse(roundTrip(html));
    expect(body.querySelector('a strong span, a span strong, strong a span')).not.toBeNull();
    expect(body.querySelector('span')?.getAttribute('style')).toBe('color: rgb(0, 128, 0);');
  });

  it('keeps alignment and other styles on paragraphs and headings', () => {
    const html =
      '<p style="text-align: center;">centred</p><h2 style="text-align: right; line-height: 1.5;">right</h2><p align="right">legacy</p>';
    expect(roundTrip(html)).toBe(html);
  });

  it('keeps styles on lists and quotes', () => {
    const html =
      '<ul style="list-style-type: square;"><li><p>one</p></li></ul><blockquote style="margin-left: 40px;"><p>quoted</p></blockquote>';
    expect(roundTrip(html)).toBe(html);
  });

  it('gives text in nested styled spans the outer and the inner styles', () => {
    expect(
      roundTrip('<p><span style="color: red;">a <span style="font-size: 18px;">b <span>c</span></span></span></p>'),
    ).toBe('<p><span style="color: red;">a </span><span style="color: red; font-size: 18px;">b c</span></p>');
  });

  it('rewrites styles in the normalised form of the CSS object model', () => {
    expect(roundTrip('<p style="text-align:center"><span style="color:#ff0000">x</span></p>')).toBe(
      '<p style="text-align: center;"><span style="color: rgb(255, 0, 0);">x</span></p>',
    );
  });

  it('turns a <font> tag into the equivalent styled span', () => {
    expect(roundTrip('<p><font color="#ff0000" face="georgia, serif" size="4">old</font></p>')).toBe(
      '<p><span style="color: rgb(255, 0, 0); font-family: georgia, serif; font-size: large;">old</span></p>',
    );
  });

  it('keeps the style of a text-only <div> as a paragraph', () => {
    expect(roundTrip('<div style="text-align: center;">centred</div>')).toBe(
      '<p style="text-align: center;">centred</p>',
    );
  });
});

describe('composeEditorExtensions — a draft written in a webmail client', () => {
  it('keeps the table, the alignment and the colours', () => {
    const draft =
      '<div dir="ltr"><div>Hi Alex,</div><div><br></div>' +
      '<div style="text-align:center"><b>Quarterly plan</b></div>' +
      '<table cellspacing="0" cellpadding="0" dir="ltr" border="1" style="table-layout:fixed;font-size:10pt;font-family:Arial;width:0px;border-collapse:collapse;border:none">' +
      '<colgroup><col width="100"><col width="100"></colgroup><tbody>' +
      '<tr style="height:21px"><td style="border:1px solid rgb(204,204,204);background-color:rgb(255,242,204);font-weight:bold">Item</td>' +
      '<td style="border:1px solid rgb(204,204,204);text-align:right">Cost</td></tr>' +
      '<tr style="height:21px"><td style="border:1px solid rgb(204,204,204)">Hosting</td>' +
      '<td style="border:1px solid rgb(204,204,204);text-align:right"><span style="background-color:rgb(255,255,0)">120 EUR</span></td></tr>' +
      '</tbody></table>' +
      '<div><font color="#0000ff">Regards,</font></div><div>Sam (sam@example.com)</div></div>';
    const body = parse(roundTrip(draft));

    const table = body.querySelector('table');
    expect(table?.style.tableLayout).toBe('fixed');
    expect(table?.style.fontSize).toBe('10pt');
    expect(table?.style.fontFamily).toBe('Arial');
    expect(table?.style.borderCollapse).toBe('collapse');
    expect(table?.getAttribute('border')).toBe('1');
    expect(table?.getAttribute('cellpadding')).toBe('0');
    expect(body.querySelectorAll('tr')).toHaveLength(2);
    expect(body.querySelector('tr')?.style.height).toBe('21px');
    expect(Array.from(body.querySelectorAll('colgroup > col')).map((col) => col.getAttribute('width'))).toEqual([
      '100',
      '100',
    ]);
    const cells = body.querySelectorAll('td');
    expect(cells).toHaveLength(4);
    expect(cells[0].style.border).toBe('1px solid rgb(204, 204, 204)');
    expect(cells[0].style.backgroundColor).toBe('rgb(255, 242, 204)');
    expect(cells[0].style.fontWeight).toBe('bold');
    expect(cells[1].style.textAlign).toBe('right');
    expect(cells[3].querySelector('span')?.style.backgroundColor).toBe('rgb(255, 255, 0)');
    expect(cells[3].textContent).toBe('120 EUR');
    const heading = body.querySelector('p[style] strong');
    expect(heading?.textContent).toBe('Quarterly plan');
    expect(heading?.closest('p')?.style.textAlign).toBe('center');
    const regards = Array.from(body.querySelectorAll('span')).find((span) => span.textContent === 'Regards,');
    expect(regards?.style.color).toBe('rgb(0, 0, 255)');
    expect(body.firstElementChild?.outerHTML).toBe('<p>Hi Alex,</p>');
  });
});

const SECTIONED_TABLE =
  '<table width="300"><caption style="caption-side: bottom;" align="left">Totals</caption>' +
  '<colgroup><col width="80"><col span="2" style="width: 110px;"></colgroup>' +
  '<thead><tr><th>Item</th><th>Net</th><th>Gross</th></tr></thead>' +
  '<tbody><tr><td>Hosting</td><td>100</td><td>121</td></tr><tr><td>Domain</td><td>10</td><td>12</td></tr></tbody>' +
  '<tfoot><tr><td>Sum</td><td>110</td><td>133</td></tr></tfoot></table>';

function rowTexts(section: Element | null): string[] {
  if (!section) throw new Error('section not found');
  return Array.from(section.querySelectorAll('tr')).map((row) => row.textContent ?? '');
}

describe('composeEditorExtensions — table sections, caption and column widths', () => {
  it('keeps thead, tbody and tfoot with their rows', () => {
    const table = parse(roundTrip(SECTIONED_TABLE)).querySelector('table');

    expect(Array.from(table?.children ?? []).map((el) => el.tagName)).toEqual([
      'CAPTION',
      'COLGROUP',
      'THEAD',
      'TBODY',
      'TFOOT',
    ]);
    expect(rowTexts(table?.querySelector('thead') ?? null)).toEqual(['ItemNetGross']);
    expect(rowTexts(table?.querySelector('tbody') ?? null)).toEqual(['Hosting100121', 'Domain1012']);
    expect(rowTexts(table?.querySelector('tfoot') ?? null)).toEqual(['Sum110133']);
    expect(table?.querySelector('thead th')?.textContent).toBe('Item');
  });

  it('keeps the caption with its text, style and alignment', () => {
    const caption = parse(roundTrip(SECTIONED_TABLE)).querySelector('caption');
    expect(caption?.textContent).toBe('Totals');
    expect(attrs(caption)).toEqual({ style: 'caption-side: bottom;', align: 'left' });
  });

  it('keeps the column widths of colgroup', () => {
    const cols = parse(roundTrip(SECTIONED_TABLE)).querySelectorAll('colgroup > col');
    expect(Array.from(cols).map(attrs)).toEqual([{ width: '80' }, { span: '2', style: 'width: 110px;' }]);
  });

  it('keeps sections, caption and widths when text inside a cell is edited', () => {
    const editor = createEditor(SECTIONED_TABLE);
    let end = -1;
    editor.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === 'Hosting') end = pos + node.nodeSize;
    });
    editor.commands.insertContentAt(end, ' plan');
    const table = parse(editor.getHTML()).querySelector('table');
    editor.destroy();

    expect(rowTexts(table?.querySelector('tbody') ?? null)).toEqual(['Hosting plan100121', 'Domain1012']);
    expect(rowTexts(table?.querySelector('thead') ?? null)).toEqual(['ItemNetGross']);
    expect(rowTexts(table?.querySelector('tfoot') ?? null)).toEqual(['Sum110133']);
    expect(table?.querySelector('caption')?.textContent).toBe('Totals');
    expect(table?.querySelectorAll('colgroup > col')).toHaveLength(2);
    expect(table?.getAttribute('width')).toBe('300');
  });

  it('puts the footer after the body even when the source has it first', () => {
    const table = parse(
      roundTrip(
        '<table><thead><tr><th>H</th></tr></thead><tfoot><tr><td>F</td></tr></tfoot><tbody><tr><td>B</td></tr></tbody></table>',
      ),
    ).querySelector('table');
    expect(Array.from(table?.children ?? []).map((el) => el.tagName)).toEqual(['THEAD', 'TBODY', 'TFOOT']);
  });

  it('writes a table that is all header rows without an empty tbody', () => {
    expect(roundTrip('<table><thead><tr><th>Only</th></tr></thead></table>')).toBe(
      '<table><thead><tr><th colspan="1" rowspan="1"><p>Only</p></th></tr></thead></table>',
    );
  });

  it('reduces markup inside a caption to its text', () => {
    const caption = parse(
      roundTrip('<table><caption><b>Q3</b> totals</caption><tbody><tr><td>x</td></tr></tbody></table>'),
    ).querySelector('caption');
    expect(caption?.innerHTML).toBe('Q3 totals');
  });

  it('drops a colgroup that no longer describes the columns', () => {
    const editor = createEditor(SECTIONED_TABLE);
    let inCell = -1;
    editor.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === 'Hosting') inCell = pos;
    });
    editor.commands.setTextSelection(inCell);
    expect(editor.commands.addColumnAfter()).toBe(true);
    const table = parse(editor.getHTML()).querySelector('table');
    editor.destroy();

    expect(table?.querySelectorAll('thead th')).toHaveLength(4);
    expect(table?.querySelector('colgroup')).toBeNull();
    expect(table?.querySelector('caption')?.textContent).toBe('Totals');
  });

  it('shows the caption in the editor, above the rows and not editable', () => {
    const editor = new Editor({
      element: document.createElement('div'),
      extensions: composeEditorExtensions,
      content: SECTIONED_TABLE,
    });
    const caption = editor.view.dom.querySelector('table > caption');
    const text = caption?.textContent;
    const editable = caption?.getAttribute('contenteditable');
    const headerCells = editor.view.dom.querySelectorAll('table th').length;
    editor.destroy();

    expect(text).toBe('Totals');
    expect(editable).toBe('false');
    expect(headerCells).toBe(3);
  });
});

describe('composeEditorExtensions — legacy inline and block tags', () => {
  it('keeps subscript, superscript and small text', () => {
    const html = '<p>H<sub>2</sub>O x<sup>2</sup> <small>fine print</small></p>';
    expect(roundTrip(html)).toBe(html);
  });

  it('keeps them together with other formatting', () => {
    const body = parse(roundTrip('<p><b>E = mc<sup>2</sup></b> <span style="color: red;"><sub>low</sub></span></p>'));
    expect(body.querySelector('strong sup, sup strong')?.textContent).toBe('2');
    expect(body.querySelector('sub')?.textContent).toBe('low');
    expect(body.querySelector('span')?.getAttribute('style')).toBe('color: red;');
  });

  it('keeps <center> around its text, as a centred block', () => {
    expect(roundTrip('<center>mid</center><p>after</p>')).toBe('<center><p>mid</p></center><p>after</p>');
  });

  it('keeps <center> around a table', () => {
    const body = parse(roundTrip('<center><table><tbody><tr><td>Cell</td></tr></tbody></table></center>'));
    expect(body.querySelector('center > table td')?.textContent).toBe('Cell');
  });
});

describe('composeEditorExtensions — known limits', () => {
  it('drops the attributes of thead, tbody and tfoot themselves', () => {
    const body = parse(
      roundTrip('<table><thead bgcolor="#eeeeee" style="color: red;"><tr><th>H</th></tr></thead></table>'),
    );
    expect(attrs(body.querySelector('thead'))).toEqual({});
  });

  it('drops the wrapper of a <div> that holds other blocks', () => {
    expect(roundTrip('<div style="padding: 8px"><p>one</p><p>two</p></div>')).toBe('<p>one</p><p>two</p>');
  });
});

describe('composeEditorExtensions — signature block', () => {
  it('keeps the signature container through a round trip', () => {
    const html =
      '<p>Hello</p><p></p><div data-emailops-signature=""><p>Ana <strong>Lopez</strong></p><p>Example Ltd</p></div>';
    expect(roundTrip(html)).toBe(html);
  });

  it('keeps a signature whose content is bare text', () => {
    const body = parse(roundTrip('<p>Hi</p><div data-emailops-signature="">Ana</div>'));
    expect(body.querySelector('[data-emailops-signature]')?.textContent).toBe('Ana');
  });

  it('keeps a pasted logo in the signature', () => {
    const body = parse(
      roundTrip('<div data-emailops-signature=""><p><img src="data:image/png;base64,AAAA" alt="logo"></p></div>'),
    );
    expect(body.querySelector('[data-emailops-signature] img')?.getAttribute('src')).toBe('data:image/png;base64,AAAA');
  });
});
