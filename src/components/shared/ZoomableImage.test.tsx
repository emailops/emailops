// How the viewer decides what size to draw an image at. The zoom and pan
// arithmetic behind it is covered in `lib/imageZoom.test.ts`; what is left here
// is the wiring that feeds it real measurements — and one race that made every
// inline email image render stretched to the shape of the screen.
//
// jsdom neither lays out nor decodes anything, so both measurements have to be
// stubbed: elements report a zero client box, and images a zero natural size.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { ZoomableImage } from './ZoomableImage';

const SRC = 'data:image/png;base64,iVBORw0KGgo=';
const OTHER_SRC = 'data:image/png;base64,iVBORw0KGgoAAA=';

/** A landscape image against a square surface: fitted by width, letterboxed. */
const NATURAL = { width: 1400, height: 900 };
const SURFACE = 500;
const FITTED_WIDTH = SURFACE;
const FITTED_HEIGHT = (NATURAL.height * SURFACE) / NATURAL.width;

let container: HTMLDivElement;
let root: Root;
let restore: Array<() => void>;

/** Pretend the browser laid the surface out and finished decoding the image —
 *  including having done so *before* React could attach its load handler, which
 *  is what a `data:` URI does in practice. */
function stubMeasurements() {
  const original = {
    clientWidth: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth'),
    clientHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight'),
  };
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => SURFACE });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => SURFACE });
  Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', {
    configurable: true,
    get: () => NATURAL.width,
  });
  Object.defineProperty(HTMLImageElement.prototype, 'naturalHeight', {
    configurable: true,
    get: () => NATURAL.height,
  });
  restore.push(() => {
    if (original.clientWidth) Object.defineProperty(HTMLElement.prototype, 'clientWidth', original.clientWidth);
    if (original.clientHeight) Object.defineProperty(HTMLElement.prototype, 'clientHeight', original.clientHeight);
    Reflect.deleteProperty(HTMLImageElement.prototype, 'naturalWidth');
    Reflect.deleteProperty(HTMLImageElement.prototype, 'naturalHeight');
  });
}

function image(): HTMLImageElement {
  const img = container.querySelector('img');
  if (!img) throw new Error('the image did not render');
  return img;
}

beforeEach(() => {
  restore = [];
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  for (const undo of restore) undo();
});

describe('ZoomableImage', () => {
  // The regression: the reset that greets a new src used to live in an effect,
  // which runs after paint. A data: image is already decoded by then, so the
  // reset landed *after* the real size and replaced it with "unknown" — and an
  // unknown size falls back to the surface, stretching the picture to fill the
  // screen for as long as the viewer stayed open.
  it('draws an image that decoded before mount at its fitted size, not stretched to the surface', () => {
    stubMeasurements();
    act(() => {
      root.render(<ZoomableImage src={SRC} alt="chart" />);
    });
    expect(Number.parseFloat(image().style.width)).toBeCloseTo(FITTED_WIDTH);
    expect(Number.parseFloat(image().style.height)).toBeCloseTo(FITTED_HEIGHT);
  });

  it('re-fits when a different image is shown', () => {
    stubMeasurements();
    act(() => {
      root.render(<ZoomableImage src={SRC} alt="chart" />);
    });
    act(() => {
      root.render(<ZoomableImage src={OTHER_SRC} alt="other" />);
    });
    expect(image().getAttribute('src')).toBe(OTHER_SRC);
    expect(Number.parseFloat(image().style.height)).toBeCloseTo(FITTED_HEIGHT);
  });

  it('starts fitted and centred', () => {
    stubMeasurements();
    act(() => {
      root.render(<ZoomableImage src={SRC} alt="chart" />);
    });
    expect(image().style.transform).toBe('translate(0px, 0px) scale(1)');
  });

  // Before the surface has a size there is nothing to fit against; the CSS
  // fallback is what keeps that first frame inside the screen.
  it('constrains an unmeasured image with max-width and max-height', () => {
    act(() => {
      root.render(<ZoomableImage src={SRC} alt="chart" />);
    });
    expect(image().style.maxWidth).toBe('100%');
    expect(image().style.maxHeight).toBe('100%');
  });

  it('claims the gesture from the browser so a pinch zooms the image, not the page', () => {
    act(() => {
      root.render(<ZoomableImage src={SRC} alt="chart" />);
    });
    const surface = image().parentElement;
    expect(surface?.style.touchAction).toBe('none');
  });
});
