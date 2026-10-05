/**
 * Spreadsheet clipboard text, as Excel, Numbers, LibreOffice and Google
 * Sheets put it on the clipboard: rows separated by line breaks, cells by
 * tabs, and a cell holding a tab, line break or quote wrapped in quotes with
 * inner quotes doubled.
 */

/** Whether pasted text is a block of cells rather than text for one cell. */
export function isTablePaste(text: string): boolean {
  const body = text.replace(/\r?\n$/, '');
  return body.includes('\t') || body.includes('\n');
}

/** Pure: clipboard text as rows of cell values. */
export function parseClipboardTable(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = '';
  let i = 0;
  const src = text.replace(/\r\n?/g, '\n');
  while (i < src.length) {
    if (cell === '' && src[i] === '"') {
      // Quoted cell: runs to the closing quote; "" is a literal quote.
      let j = i + 1;
      let value = '';
      while (j < src.length) {
        if (src[j] === '"' && src[j + 1] === '"') {
          value += '"';
          j += 2;
        } else if (src[j] === '"') {
          break;
        } else {
          value += src[j];
          j += 1;
        }
      }
      const next = src[j + 1];
      if (j < src.length && (next === undefined || next === '\t' || next === '\n')) {
        cell = value;
        i = j + 1;
        continue;
      }
      // Not a well-formed quoted cell: read it as plain text.
    }
    const ch = src[i];
    if (ch === '\t') {
      row.push(cell);
      cell = '';
    } else if (ch === '\n') {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = '';
    } else {
      cell += ch;
    }
    i += 1;
  }
  if (cell !== '' || row.length > 0) {
    row.push(cell);
    rows.push(row);
  }
  return rows;
}
