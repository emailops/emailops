import { describe, expect, it } from 'vitest';
import { displayValue, evaluateCell, formatNumber, parseNumber } from './sheetFormula';

describe('parseNumber', () => {
  it('reads numbers as people type them, currency and thousands included', () => {
    expect(parseNumber('420')).toBe(420);
    expect(parseNumber('69,00 €')).toBe(69);
    expect(parseNumber('1.287,81 €')).toBe(1287.81);
    expect(parseNumber('$1,234.50')).toBe(1234.5);
    expect(parseNumber('1.234')).toBe(1234);
    expect(parseNumber('0.5')).toBe(0.5);
    expect(parseNumber('-12,5')).toBe(-12.5);
    expect(parseNumber('Alta')).toBeNull();
    expect(parseNumber('')).toBeNull();
    expect(parseNumber('2026-11-03')).toBeNull();
  });
});

// values[row][col]; A1 is values[0][0].
const SHEET = [
  ['Item', 'Price', 'Qty'],
  ['Desk', '69,00 €', '2'],
  ['Lamp', '24,90 €', ''],
  ['Chair', 'n/a', '3'],
  ['Total', '=SUM(B2:B4)', '=SUMA(C2:C4)'],
];

const at = (values: string[][], ref: string) => {
  const col = ref.charCodeAt(0) - 65;
  const row = Number(ref.slice(1)) - 1;
  return evaluateCell(values, row, col);
};

describe('evaluateCell', () => {
  it('adds the numbers of a range and skips text and blanks', () => {
    expect(at(SHEET, 'B5')).toEqual({ value: 93.9 });
    expect(at(SHEET, 'C5')).toEqual({ value: 5 });
  });

  it('knows the basic aggregations, in English and Spanish', () => {
    const v = (f: string) => evaluateCell([['1', '2', '4'], [f]], 1, 0);
    expect(v('=AVERAGE(A1:C1)')).toEqual({ value: 7 / 3 });
    expect(v('=promedio(A1:C1)')).toEqual({ value: 7 / 3 });
    expect(v('=MIN(A1:C1)')).toEqual({ value: 1 });
    expect(v('=MAX(A1:C1)')).toEqual({ value: 4 });
    expect(v('=COUNT(A1:C1)')).toEqual({ value: 3 });
    expect(v('=CONTAR(A1:C1)')).toEqual({ value: 3 });
    expect(v('=SUM(A1,C1)')).toEqual({ value: 5 });
    expect(v('=SUM(A1:A1;C1)')).toEqual({ value: 5 });
  });

  it('follows formulas that use other formulas', () => {
    const values = [
      ['1', '2'],
      ['=SUM(A1:B1)', '=SUM(A2,A1)'],
    ];
    expect(evaluateCell(values, 1, 1)).toEqual({ value: 4 });
  });

  it('reports a loop, an unknown function and a bad reference instead of hanging or guessing', () => {
    expect(evaluateCell([['=SUM(A1)']], 0, 0)).toEqual({ error: '#CYCLE!' });
    expect(evaluateCell([['=SUM(A2)'], ['=SUM(A1)']], 0, 0)).toEqual({ error: '#CYCLE!' });
    expect(evaluateCell([['=MEDIAN(A1)']], 0, 0)).toEqual({ error: '#NAME?' });
    expect(evaluateCell([['=SUM(ZZ)']], 0, 0)).toEqual({ error: '#REF!' });
    expect(evaluateCell([['=1+1']], 0, 0)).toEqual({ error: '#NAME?' });
  });

  it('an empty range averages to an error, as in Excel', () => {
    expect(evaluateCell([['', '=AVERAGE(A1)']], 0, 1)).toEqual({ error: '#DIV/0!' });
  });

  it('a plain value is itself', () => {
    expect(evaluateCell(SHEET, 1, 1)).toEqual({ text: '69,00 €' });
  });
});

describe('displayValue', () => {
  it('shows a formula as its result and anything else as typed', () => {
    expect(displayValue(SHEET, 4, 1, 'es')).toBe('93,9');
    expect(displayValue(SHEET, 4, 1, 'en')).toBe('93.9');
    expect(displayValue(SHEET, 0, 0, 'es')).toBe('Item');
    expect(displayValue([['=MEDIAN(A1)']], 0, 0, 'es')).toBe('#NAME?');
  });
});

describe('formatNumber', () => {
  it('keeps up to two decimals with the locale separators', () => {
    expect(formatNumber(1287.8149, 'es')).toBe('1287,81');
    expect(formatNumber(1234567.5, 'en')).toBe('1,234,567.5');
  });
});
