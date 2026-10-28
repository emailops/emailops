// The PDF preview renders pages with pdf.js and owns its zoom: a toolbar
// (out, level, in, fit width), the Cmd/Ctrl zoom shortcuts, and fit-to-width
// by default that follows the pane as it resizes.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts ? `${key} ${JSON.stringify(opts)}` : key),
    i18n: { language: 'en' },
  }),
}));

const addLog = vi.fn();
vi.mock('@/stores/logStore', () => ({
  useLogStore: (sel: (s: { addLog: typeof addLog }) => unknown) => sel({ addLog }),
}));

vi.mock('pdfjs-dist/legacy/build/pdf.worker.min.mjs?url', () => ({ default: 'pdf.worker.js' }));

const PAGE_WIDTH = 600;
const render = vi.fn((_params: { canvas: unknown; viewport: { width: number } }) => ({
  promise: Promise.resolve(),
  cancel: vi.fn(),
}));
const page = {
  getViewport: ({ scale }: { scale: number }) => ({ width: PAGE_WIDTH * scale, height: 800 * scale, scale }),
  render,
};
const getDocument = vi.fn();
vi.mock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
  GlobalWorkerOptions: {},
  getDocument: (params: unknown) => getDocument(params),
}));

let resize: (width: number) => void = () => {};
class FakeResizeObserver {
  constructor(private cb: ResizeObserverCallback) {
    resize = (width) => this.cb([{ contentRect: { width } } as ResizeObserverEntry], this as never);
  }
  observe() {}
  disconnect() {}
}

import { PdfViewer } from './PdfViewer';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  render.mockClear();
  addLog.mockClear();
  getDocument.mockReset();
  getDocument.mockReturnValue({
    promise: Promise.resolve({ numPages: 2, getPage: () => Promise.resolve(page), destroy: vi.fn() }),
    destroy: vi.fn(),
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

// "%PDF" in base64.
const DATA_URL = 'data:application/pdf;base64,JVBERg==';

const settle = () => act(async () => new Promise((r) => setTimeout(r, 10)));

async function mount(paneWidth = 1224) {
  await act(async () => root.render(<PdfViewer dataUrl={DATA_URL} title="doc.pdf" />));
  await act(async () => resize(paneWidth));
  await settle();
}

const level = () => container.querySelector('[data-testid="pdf-zoom-level"]')?.textContent;
const button = (label: string) => container.querySelector(`button[aria-label="${label}"]`) as HTMLButtonElement;
const lastRenderWidth = () => {
  const calls = render.mock.calls;
  return calls[calls.length - 1]?.[0].viewport.width;
};
const press = async (key: string) => {
  await act(async () => {
    container
      .querySelector('[data-testid="pdf-viewer"]')
      ?.dispatchEvent(new KeyboardEvent('keydown', { key, metaKey: true, bubbles: true, cancelable: true }));
  });
  await settle();
};
const click = async (label: string) => {
  await act(async () => button(label).click());
  await settle();
};

describe('PdfViewer', () => {
  it('loads the decoded bytes, not the data URL', async () => {
    await mount();
    const params = getDocument.mock.calls[0][0] as { data: Uint8Array; url?: string };
    expect(params.url).toBeUndefined();
    expect([...params.data]).toEqual([0x25, 0x50, 0x44, 0x46]);
  });

  it('opens fitted to the pane width and renders every page at that scale', async () => {
    await mount(1224); // (1224 - 24 padding) / 600 = 200%
    expect(level()).toBe('200%');
    expect(container.querySelectorAll('canvas')).toHaveLength(2);
    expect(lastRenderWidth()).toBe(1200);
  });

  it('re-fits as the pane resizes while in fit mode', async () => {
    await mount(1224);
    await act(async () => resize(624));
    await settle();
    expect(level()).toBe('100%');
    expect(lastRenderWidth()).toBe(600);
  });

  it('steps through the zoom levels with the toolbar and re-renders at the new scale', async () => {
    await mount(1224);
    await click('attachments:viewer.zoomOut');
    expect(level()).toBe('150%');
    expect(lastRenderWidth()).toBe(900);

    // A fixed zoom no longer follows the pane.
    await act(async () => resize(624));
    await settle();
    expect(level()).toBe('150%');

    await click('attachments:viewer.fitWidth');
    expect(level()).toBe('100%');
  });

  it('disables zoom in at the maximum level', async () => {
    await mount(1224);
    await press('=');
    expect(level()).toBe('300%');
    expect(button('attachments:viewer.zoomIn').disabled).toBe(true);
    expect(button('attachments:viewer.zoomOut').disabled).toBe(false);
  });

  it('handles the Cmd/Ctrl zoom shortcuts', async () => {
    await mount(1224);
    await press('-');
    await press('-');
    expect(level()).toBe('125%');
    await press('0');
    expect(level()).toBe('200%');
  });

  it('shows and logs a load failure', async () => {
    getDocument.mockReturnValue({ promise: Promise.reject(new Error('bad xref')), destroy: vi.fn() });
    await mount();
    expect(container.textContent).toContain('attachments:viewer.previewLoadFailed');
    expect(addLog).toHaveBeenCalledWith('error', 'system', expect.stringContaining('bad xref'));
  });
});
