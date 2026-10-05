import { afterEach, describe, expect, it, vi } from 'vitest';
import { PRINT_ROOT_ID, printableHtml, printDocument } from './docPrint';

describe('printableHtml', () => {
  it('puts the title on top of the content', () => {
    const html = printableHtml('Budget', '<p>Total</p>');
    expect(html).toBe('<h1>Budget</h1><p>Total</p>');
  });

  it('escapes the title and drops anything that could run', () => {
    const html = printableHtml(
      '<b>Q3</b>',
      '<p onclick="x()">Hi</p><script>alert(1)</script><a href="javascript:x()">l</a>',
    );
    expect(html).toContain('<h1>&lt;b&gt;Q3&lt;/b&gt;</h1>');
    expect(html).not.toContain('script');
    expect(html).not.toContain('onclick');
    expect(html).not.toContain('javascript:');
  });

  it('keeps tables and images, which a document can hold', () => {
    const html = printableHtml(
      'T',
      '<table><tbody><tr><td>1</td></tr></tbody></table><img src="data:image/png;base64,AA==">',
    );
    expect(html).toContain('<td>1</td>');
    expect(html).toContain('<img src="data:image/png;base64,AA==">');
  });
});

describe('printDocument', () => {
  afterEach(() => {
    document.getElementById(PRINT_ROOT_ID)?.remove();
    vi.restoreAllMocks();
  });

  it('prints the document alone, named after it, and gives the window its title back', async () => {
    document.title = 'EmailOps';
    let printed = '';
    let titleWhilePrinting = '';
    vi.spyOn(window, 'print').mockImplementation(() => {
      printed = document.getElementById(PRINT_ROOT_ID)?.innerHTML ?? '';
      titleWhilePrinting = document.title;
    });

    await printDocument('Budget', '<p>Total</p>');

    expect(printed).toBe('<h1>Budget</h1><p>Total</p>');
    expect(titleWhilePrinting).toBe('Budget');
    expect(document.title).toBe('EmailOps');
  });
});
