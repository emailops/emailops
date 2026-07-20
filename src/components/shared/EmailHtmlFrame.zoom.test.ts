// Pinch-zoom support inside the email body iframe. macOS touchpad pinches
// arrive in two dialects: Chromium/Firefox synthesize a `wheel` event with
// `ctrlKey: true`, while WebKit (the Tauri webview on macOS) fires proprietary
// `gesturestart`/`gesturechange` events carrying an absolute `scale`. The
// bridge script handles both, applies the zoom to the document root, and
// re-posts the (scaled) auto-height so the parent iframe grows with the
// content. We exercise the real BRIDGE_SCRIPT under jsdom, reading the zoom
// back from the `data-email-zoom` attribute the script maintains.

import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { BRIDGE_SCRIPT } from './EmailHtmlFrame';

function runBridge() {
  new Function(BRIDGE_SCRIPT)();
}

function currentZoom(): number {
  const raw = document.documentElement.getAttribute('data-email-zoom');
  return raw === null ? 1 : Number(raw);
}

function pinch(deltaY: number, ctrlKey = true): WheelEvent {
  const ev = new WheelEvent('wheel', { deltaY, ctrlKey, cancelable: true, bubbles: true });
  window.dispatchEvent(ev);
  return ev;
}

function gesture(type: 'gesturestart' | 'gesturechange' | 'gestureend', scale: number) {
  const ev = new Event(type, { cancelable: true, bubbles: true }) as Event & { scale: number };
  ev.scale = scale;
  window.dispatchEvent(ev);
}

function resetZoom() {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: '0', metaKey: true, cancelable: true }));
}

interface FrameMessage {
  __emailFrame?: boolean;
  type?: string;
  height?: number;
}

function lastHeightMessage(calls: unknown[][]): number | null {
  const heights = calls
    .map((call) => call[0] as FrameMessage)
    .filter((msg) => msg?.__emailFrame && msg.type === 'height' && typeof msg.height === 'number')
    .map((msg) => msg.height as number);
  return heights.length > 0 ? heights[heights.length - 1] : null;
}

describe('EmailHtmlFrame bridge pinch-zoom', () => {
  // The bridge registers window-level listeners it never removes, so run it
  // exactly once for the whole file and reset zoom between tests.
  beforeAll(() => {
    runBridge();
  });

  beforeEach(() => {
    vi.restoreAllMocks();
    resetZoom();
  });

  it('ctrl+wheel up zooms in and consumes the event', () => {
    const ev = pinch(-100);
    expect(currentZoom()).toBeGreaterThan(1);
    expect(ev.defaultPrevented).toBe(true);
  });

  it('ctrl+wheel down zooms out below 1', () => {
    pinch(100);
    expect(currentZoom()).toBeLessThan(1);
  });

  it('a plain scroll wheel without ctrl does not zoom or consume the event', () => {
    const ev = pinch(-100, false);
    expect(currentZoom()).toBe(1);
    expect(ev.defaultPrevented).toBe(false);
  });

  it('clamps zoom to at most 3x', () => {
    for (let i = 0; i < 20; i++) pinch(-500);
    expect(currentZoom()).toBe(3);
  });

  it('clamps zoom to at least 0.5x', () => {
    for (let i = 0; i < 20; i++) pinch(500);
    expect(currentZoom()).toBe(0.5);
  });

  it('WebKit gesture events apply the scale relative to the zoom at gesture start', () => {
    gesture('gesturestart', 1);
    gesture('gesturechange', 2);
    gesture('gestureend', 2);
    expect(currentZoom()).toBe(2);

    // A second gesture compounds on the zoom left by the first one.
    gesture('gesturestart', 1);
    gesture('gesturechange', 0.75);
    gesture('gestureend', 0.75);
    expect(currentZoom()).toBe(1.5);
  });

  it('cmd+0 resets zoom to 1', () => {
    pinch(-300);
    expect(currentZoom()).toBeGreaterThan(1);
    resetZoom();
    expect(currentZoom()).toBe(1);
  });

  it('re-posts the auto-height scaled by the zoom factor so the iframe grows with the content', () => {
    Object.defineProperty(document.body, 'scrollHeight', { configurable: true, value: 1000 });
    try {
      const spy = vi.spyOn(window, 'postMessage');
      pinch(-100);
      const zoom = currentZoom();
      expect(zoom).toBeGreaterThan(1);
      expect(lastHeightMessage(spy.mock.calls)).toBe(Math.round(1000 * zoom));
    } finally {
      Reflect.deleteProperty(document.body, 'scrollHeight');
    }
  });
});
