// The panel chrome every Settings tab shares: a flex-column frame with a
// scrollable body, and optionally a footer that stays put while the body
// scrolls. The footer-outside-the-scroll-container property is the one worth
// pinning — putting it inside would scroll the save/test buttons out of view.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { SettingsPanel } from './SettingsPanel';

describe('SettingsPanel', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it('renders children inside the scroll container', () => {
    act(() => {
      root.render(
        <SettingsPanel>
          <p id="body">body</p>
        </SettingsPanel>,
      );
    });

    const scroll = container.querySelector('.overflow-y-auto');
    expect(scroll).not.toBeNull();
    expect(scroll?.querySelector('#body')).not.toBeNull();
  });

  it('keeps the footer outside the scroll container so it stays visible', () => {
    act(() => {
      root.render(
        <SettingsPanel footer={<div id="footer">save</div>}>
          <p>body</p>
        </SettingsPanel>,
      );
    });

    const scroll = container.querySelector('.overflow-y-auto');
    const footer = container.querySelector('#footer');
    expect(footer, 'footer should be mounted').not.toBeNull();
    expect(scroll?.contains(footer as Node), 'footer must not be inside the scroll container').toBe(false);
  });

  it('keeps the header outside the scroll container so error banners stay visible', () => {
    act(() => {
      root.render(
        <SettingsPanel header={<div id="banner">error</div>}>
          <p>body</p>
        </SettingsPanel>,
      );
    });

    const scroll = container.querySelector('.overflow-y-auto');
    const banner = container.querySelector('#banner');
    expect(banner, 'header should be mounted').not.toBeNull();
    expect(scroll?.contains(banner as Node), 'header must not be inside the scroll container').toBe(false);
  });

  it('is a stable component identity across re-renders', () => {
    act(() => {
      root.render(
        <SettingsPanel>
          <p>body</p>
        </SettingsPanel>,
      );
    });
    const before = container.querySelector('.overflow-y-auto');

    act(() => {
      root.render(
        <SettingsPanel>
          <p>body</p>
        </SettingsPanel>,
      );
    });

    expect(container.querySelector('.overflow-y-auto')).toBe(before);
  });
});
