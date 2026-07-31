// Custom dropdown replacing a native <select> for the language picker.
// WebKitGTK on Linux renders a native <select>'s option popup via the GTK
// theme, not page CSS, so it shows up light even inside this app's dark
// surfaces — this component owns its own popup markup so it can be styled.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LanguageSelect } from './LanguageSelect';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function clickTrigger() {
  const button = container.querySelector('button[aria-haspopup="listbox"]');
  act(() => {
    button?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

function clickOption(nativeName: string) {
  const option = Array.from(container.querySelectorAll('[role="option"]')).find((el) =>
    el.textContent?.includes(nativeName),
  );
  act(() => {
    option?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

describe('LanguageSelect', () => {
  it('renders the current language as the trigger label', () => {
    act(() => {
      root.render(<LanguageSelect value="es" onChange={vi.fn()} ariaLabel="Language" />);
    });
    expect(container.textContent).toContain('Español');
  });

  it('opens the option list on click and lists all supported languages', () => {
    act(() => {
      root.render(<LanguageSelect value="en" onChange={vi.fn()} ariaLabel="Language" />);
    });
    clickTrigger();
    expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    expect(container.textContent).toContain('Français');
    expect(container.textContent).toContain('Deutsch');
  });

  it('calls onChange with the picked language and closes the popup', () => {
    const onChange = vi.fn();
    act(() => {
      root.render(<LanguageSelect value="en" onChange={onChange} ariaLabel="Language" />);
    });
    clickTrigger();
    clickOption('Français');
    expect(onChange).toHaveBeenCalledWith('fr');
    expect(container.querySelector('[role="listbox"]')).toBeNull();
  });

  it('closes without calling onChange on an outside click', () => {
    const onChange = vi.fn();
    act(() => {
      root.render(<LanguageSelect value="en" onChange={onChange} ariaLabel="Language" />);
    });
    clickTrigger();
    expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    act(() => {
      document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    });
    expect(container.querySelector('[role="listbox"]')).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('closes on Escape', () => {
    act(() => {
      root.render(<LanguageSelect value="en" onChange={vi.fn()} ariaLabel="Language" />);
    });
    clickTrigger();
    expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    act(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(container.querySelector('[role="listbox"]')).toBeNull();
  });

  it('does not open when disabled', () => {
    act(() => {
      root.render(<LanguageSelect value="en" onChange={vi.fn()} ariaLabel="Language" disabled />);
    });
    clickTrigger();
    expect(container.querySelector('[role="listbox"]')).toBeNull();
  });
});
