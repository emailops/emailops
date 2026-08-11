// The full-screen image viewer's shell: the ways out of it, and what it does
// with a picture that will not load. The zoom and pan arithmetic it wraps is
// covered exhaustively in `lib/imageZoom.test.ts`.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { ImageLightbox } from './ImageLightbox';

const SRC = 'data:image/png;base64,iVBORw0KGgo=';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(props: Partial<Parameters<typeof ImageLightbox>[0]> = {}) {
  const onClose = vi.fn();
  act(() => {
    root.render(<ImageLightbox src={SRC} onClose={onClose} {...props} />);
  });
  return onClose;
}

function dialog(): HTMLElement {
  const el = container.querySelector('[role="dialog"]');
  if (!el) throw new Error('the viewer did not render');
  return el as HTMLElement;
}

describe('ImageLightbox', () => {
  it('shows the image it was given', () => {
    render({ alt: 'Q3 chart' });
    const img = container.querySelector('img');
    expect(img?.getAttribute('src')).toBe(SRC);
    expect(img?.getAttribute('alt')).toBe('Q3 chart');
  });

  it('captions the image with the alt text the sender wrote', () => {
    render({ alt: 'Q3 chart' });
    expect(container.textContent).toContain('Q3 chart');
  });

  it('closes on Escape', () => {
    const onClose = render();
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes when the close button is pressed', () => {
    const onClose = render();
    const button = container.querySelector('button');
    act(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on a click that lands on the backdrop', () => {
    const onClose = render();
    act(() => {
      dialog().dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // A drag that pans a zoomed image keeps the pointer capture, so its click
  // reports the image as the target. Dismissing there would make the picture
  // vanish at the end of every pan.
  it('stays open when the click comes from inside the image surface', () => {
    const onClose = render();
    const img = container.querySelector('img');
    act(() => {
      img?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it('opts out of the phone back-swipe so a pan is not read as navigation', () => {
    render();
    expect(dialog().hasAttribute('data-no-swipe')).toBe(true);
  });

  it('reports an image that cannot be decoded instead of showing a blank screen', () => {
    render({ alt: 'Q3 chart' });
    act(() => {
      container.querySelector('img')?.dispatchEvent(new Event('error'));
    });
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toContain('Failed to load image');
  });
});
