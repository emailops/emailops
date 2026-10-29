import { describe, expect, test } from 'vitest';
import {
  canZoomIn,
  canZoomOut,
  effectiveScale,
  FIT_WIDTH,
  fitWidthScale,
  MAX_ZOOM,
  MIN_ZOOM,
  type PdfZoom,
  zoomIn,
  zoomKeyAction,
  zoomOut,
} from './pdfZoom';

describe('effectiveScale', () => {
  test('fit mode follows the fit scale, a fixed zoom ignores it', () => {
    expect(effectiveScale(FIT_WIDTH, 1.3)).toBe(1.3);
    expect(effectiveScale(2, 1.3)).toBe(2);
  });
});

describe('zoomIn / zoomOut', () => {
  test.each<[PdfZoom, number, number]>([
    [1, 1, 1.25],
    [1.25, 1, 1.5],
    [2, 1, 3],
    // From fit width, step to the next level above wherever fit landed.
    [FIT_WIDTH, 1.1, 1.25],
    [FIT_WIDTH, 0.6, 0.75],
  ])('zoomIn(%s, fit %s) = %s', (zoom, fit, expected) => {
    expect(zoomIn(zoom, fit)).toBe(expected);
  });

  test.each<[PdfZoom, number, number]>([
    [1, 1, 0.75],
    [3, 1, 2],
    [FIT_WIDTH, 1.1, 1],
    [FIT_WIDTH, 2.5, 2],
  ])('zoomOut(%s, fit %s) = %s', (zoom, fit, expected) => {
    expect(zoomOut(zoom, fit)).toBe(expected);
  });

  test('stays at the limits', () => {
    expect(zoomIn(MAX_ZOOM, 1)).toBe(MAX_ZOOM);
    expect(zoomOut(MIN_ZOOM, 1)).toBe(MIN_ZOOM);
  });

  test('canZoomIn / canZoomOut are false only at the limits', () => {
    expect(canZoomIn(MAX_ZOOM, 1)).toBe(false);
    expect(canZoomOut(MIN_ZOOM, 1)).toBe(false);
    expect(canZoomIn(2, 1)).toBe(true);
    expect(canZoomOut(0.75, 1)).toBe(true);
    expect(canZoomIn(FIT_WIDTH, MAX_ZOOM)).toBe(false);
    expect(canZoomOut(FIT_WIDTH, MIN_ZOOM)).toBe(false);
  });
});

describe('fitWidthScale', () => {
  test('scales the page width to the available width', () => {
    expect(fitWidthScale(600, 1224, 24)).toBe(2);
  });

  test('clamps to the zoom range for very narrow or very wide panes', () => {
    expect(fitWidthScale(600, 100, 24)).toBe(MIN_ZOOM);
    expect(fitWidthScale(600, 5000, 24)).toBe(MAX_ZOOM);
  });

  test('falls back to 100% before the pane or the page has a width', () => {
    expect(fitWidthScale(600, 0, 24)).toBe(1);
    expect(fitWidthScale(0, 800, 24)).toBe(1);
  });
});

describe('zoomKeyAction', () => {
  const key = (k: string, mods: { metaKey?: boolean; ctrlKey?: boolean } = { metaKey: true }) => ({
    key: k,
    metaKey: false,
    ctrlKey: false,
    ...mods,
  });

  test.each([
    ['=', 'in'],
    ['+', 'in'],
    ['-', 'out'],
    ['0', 'fit'],
  ])('Cmd+%s → %s', (k, action) => {
    expect(zoomKeyAction(key(k))).toBe(action);
  });

  test('Ctrl works as well as Cmd', () => {
    expect(zoomKeyAction(key('=', { ctrlKey: true }))).toBe('in');
  });

  test('ignores keys without a modifier and unrelated shortcuts', () => {
    expect(zoomKeyAction(key('=', {}))).toBeNull();
    expect(zoomKeyAction(key('c'))).toBeNull();
  });
});
