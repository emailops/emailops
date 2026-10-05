import { describe, expect, it } from 'vitest';
import { isTablePaste, parseClipboardTable } from './clipboardTable';

describe('parseClipboardTable', () => {
  it('reads rows and tab-separated cells, as Excel copies them', () => {
    expect(parseClipboardTable('Item\tPrice (€)\r\nDesk\t10,00 €\r\nLamp\t\r\n')).toEqual([
      ['Item', 'Price (€)'],
      ['Desk', '10,00 €'],
      ['Lamp', ''],
    ]);
  });

  it('keeps empty rows inside the block and drops only the trailing line break', () => {
    expect(parseClipboardTable('a\tb\n\t\nc\td\n')).toEqual([
      ['a', 'b'],
      ['', ''],
      ['c', 'd'],
    ]);
  });

  it('unquotes cells that hold quotes, tabs or line breaks', () => {
    expect(parseClipboardTable('"two\nlines"\t"say ""hi"""\n"tab\there"\tplain')).toEqual([
      ['two\nlines', 'say "hi"'],
      ['tab\there', 'plain'],
    ]);
  });

  it('leaves a quote in the middle of a cell alone', () => {
    expect(parseClipboardTable('12" screen\tok')).toEqual([['12" screen', 'ok']]);
  });
});

describe('isTablePaste', () => {
  it('is a table when the text spans cells or rows', () => {
    expect(isTablePaste('a\tb')).toBe(true);
    expect(isTablePaste('a\nb')).toBe(true);
    expect(isTablePaste('just text')).toBe(false);
    expect(isTablePaste('one cell\n')).toBe(false);
  });
});
