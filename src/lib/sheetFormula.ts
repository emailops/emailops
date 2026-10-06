/**
 * Formulas in EO Docs sheets: the basic aggregations over cell ranges, with
 * Excel's English names and their Spanish equivalents. A cell whose value
 * starts with "=" is a formula; it is stored as typed and shown as its
 * result. Ranges address the grid by position (A1, B2:B10), as in Excel.
 *
 * Pure: everything works on `values[row][col]` (the grid's raw values).
 */

export type CellResult = { value: number } | { text: string } | { error: string };

type Aggregate = (numbers: number[]) => CellResult;

const sum = (ns: number[]) => ns.reduce((a, b) => a + b, 0);

const FUNCTIONS: Record<string, Aggregate> = {
  SUM: (ns) => ({ value: sum(ns) }),
  AVERAGE: (ns) => (ns.length ? { value: sum(ns) / ns.length } : { error: '#DIV/0!' }),
  MIN: (ns) => ({ value: ns.length ? Math.min(...ns) : 0 }),
  MAX: (ns) => ({ value: ns.length ? Math.max(...ns) : 0 }),
  COUNT: (ns) => ({ value: ns.length }),
};

const ALIASES: Record<string, string> = { SUMA: 'SUM', PROMEDIO: 'AVERAGE', CONTAR: 'COUNT' };

/** Pure: a number as people type it in a cell — currency signs, spaces and
 *  thousands separators allowed, decimal comma or point. `null` otherwise. */
export function parseNumber(raw: string): number | null {
  let s = raw.trim().replace(/[\s €$£%]/g, '');
  if (!/^-?[\d.,]+$/.test(s) || !/\d/.test(s)) return null;
  const lastComma = s.lastIndexOf(',');
  const lastDot = s.lastIndexOf('.');
  if (lastComma >= 0 && lastDot >= 0) {
    // Both: the later one is the decimal separator.
    s = lastComma > lastDot ? s.replace(/\./g, '').replace(',', '.') : s.replace(/,/g, '');
  } else if (lastComma >= 0) {
    s = /^-?\d{1,3}(,\d{3})+$/.test(s) && s.split(',').length > 2 ? s.replace(/,/g, '') : s.replace(',', '.');
  } else if (/^-?\d{1,3}(\.\d{3})+$/.test(s)) {
    s = s.replace(/\./g, '');
  }
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
}

/** "B12" → [row 11, col 1]; `null` when it is not a cell reference. */
function parseRef(ref: string): [number, number] | null {
  const m = /^([A-Z]{1,3})(\d{1,6})$/.exec(ref.trim().toUpperCase());
  if (!m) return null;
  let col = 0;
  for (const ch of m[1]) col = col * 26 + (ch.charCodeAt(0) - 64);
  return [Number(m[2]) - 1, col - 1];
}

/** The cells an argument covers ("A1" or "A1:B3"), or `null` for a bad one. */
function cellsOf(arg: string): [number, number][] | null {
  const [from, to] = arg.split(':');
  const a = parseRef(from);
  const b = to === undefined ? a : parseRef(to);
  if (!a || !b) return null;
  const cells: [number, number][] = [];
  for (let r = Math.min(a[0], b[0]); r <= Math.max(a[0], b[0]); r++) {
    for (let c = Math.min(a[1], b[1]); c <= Math.max(a[1], b[1]); c++) cells.push([r, c]);
  }
  return cells;
}

/** Spreadsheet column name: 0 → A, 25 → Z, 26 → AA. */
export function columnName(index: number): string {
  let n = index + 1;
  let name = '';
  while (n > 0) {
    name = String.fromCharCode(65 + ((n - 1) % 26)) + name;
    n = Math.floor((n - 1) / 26);
  }
  return name;
}

/** A cell reference or a range ("B2", "B2:B9"), not part of a longer word. */
const REFERENCE = /(?<![A-Za-z0-9#])([A-Za-z]{1,3}\d{1,6})(?::([A-Za-z]{1,3}\d{1,6}))?(?![\w(])/g;

/**
 * Pure: a formula with its references moved for a row or column inserted
 * (`delta` 1) or deleted (`delta` -1) at `index`, as a spreadsheet does: what
 * is below or right of it moves, a range that spans it grows or shrinks, and a
 * reference to a deleted cell becomes `#REF!`.
 */
export function shiftFormula(formula: string, axis: 'row' | 'col', index: number, delta: 1 | -1): string {
  if (!formula.startsWith('=')) return formula;
  const at = axis === 'row' ? 0 : 1;
  return formula.replace(REFERENCE, (match, from: string, to: string | undefined) => {
    const a = parseRef(from);
    const b = to === undefined ? a : parseRef(to);
    if (!a || !b) return match;
    let start = Math.min(a[at], b[at]);
    let end = Math.max(a[at], b[at]);
    if (delta === 1) {
      if (start >= index) start++;
      if (end >= index) end++;
    } else {
      if (start === index && end === index) return '#REF!';
      if (start > index) start--;
      if (end >= index) end--;
    }
    const other = [Math.min(a[1 - at], b[1 - at]), Math.max(a[1 - at], b[1 - at])];
    const ref = (pos: number, rest: number) => {
      const [row, col] = axis === 'row' ? [pos, rest] : [rest, pos];
      return `${columnName(col)}${row + 1}`;
    };
    return to === undefined ? ref(start, other[0]) : `${ref(start, other[0])}:${ref(end, other[1])}`;
  });
}

function evaluate(values: string[][], row: number, col: number, visiting: Set<string>): CellResult {
  const raw = values[row]?.[col] ?? '';
  if (!raw.startsWith('=')) return { text: raw };
  const key = `${row}:${col}`;
  if (visiting.has(key)) return { error: '#CYCLE!' };
  const m = /^=\s*([A-Za-zÀ-ÿ]+)\s*\((.*)\)\s*$/.exec(raw);
  if (!m) return { error: '#NAME?' };
  const name = m[1].toUpperCase();
  const fn = FUNCTIONS[ALIASES[name] ?? name];
  if (!fn) return { error: '#NAME?' };
  visiting.add(key);
  try {
    const numbers: number[] = [];
    for (const arg of m[2]
      .split(/[,;]/)
      .map((a) => a.trim())
      .filter(Boolean)) {
      const cells = cellsOf(arg);
      if (!cells) return { error: '#REF!' };
      for (const [r, c] of cells) {
        const result = evaluate(values, r, c, visiting);
        if ('error' in result) return result;
        const n = 'value' in result ? result.value : parseNumber(result.text);
        if (n !== null) numbers.push(n);
      }
    }
    return fn(numbers);
  } finally {
    visiting.delete(key);
  }
}

/** Pure: what a cell holds once its formula, if any, is worked out. */
export function evaluateCell(values: string[][], row: number, col: number): CellResult {
  return evaluate(values, row, col, new Set());
}

/** Pure: a result number with up to two decimals, in the reader's locale. */
export function formatNumber(n: number, locale: string): string {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 2 }).format(n);
}

/** Pure: the text a cell shows — a formula's result, or the value as typed. */
export function displayValue(values: string[][], row: number, col: number, locale: string): string {
  const result = evaluateCell(values, row, col);
  if ('error' in result) return result.error;
  if ('value' in result) return formatNumber(result.value, locale);
  return result.text;
}
