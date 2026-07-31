// Custom dropdown replacing native <select> app-wide. WebKitGTK on Linux
// renders a native <select>'s option popup via the GTK theme, not page CSS,
// so every native select in this app showed a light popup even on dark
// surfaces — this component owns its own popup markup so it can be styled.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Select } from './Select';

let container: HTMLDivElement;
let root: Root;

const FRUIT_OPTIONS = [
  { value: 'apple', label: 'Apple' },
  { value: 'banana', label: 'Banana' },
  { value: 'cherry', label: 'Cherry' },
];

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

function clickOption(label: string) {
  const option = Array.from(container.querySelectorAll('[role="option"]')).find((el) =>
    el.textContent?.includes(label),
  );
  act(() => {
    option?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

describe('Select', () => {
  it('renders the label of the current value', () => {
    act(() => {
      root.render(<Select value="banana" options={FRUIT_OPTIONS} onChange={vi.fn()} ariaLabel="Fruit" />);
    });
    expect(container.textContent).toContain('Banana');
  });

  it('opens the option list on click and lists all options', () => {
    act(() => {
      root.render(<Select value="apple" options={FRUIT_OPTIONS} onChange={vi.fn()} ariaLabel="Fruit" />);
    });
    clickTrigger();
    expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    expect(container.textContent).toContain('Cherry');
  });

  it('calls onChange with the picked value and closes the popup', () => {
    const onChange = vi.fn();
    act(() => {
      root.render(<Select value="apple" options={FRUIT_OPTIONS} onChange={onChange} ariaLabel="Fruit" />);
    });
    clickTrigger();
    clickOption('Cherry');
    expect(onChange).toHaveBeenCalledWith('cherry');
    expect(container.querySelector('[role="listbox"]')).toBeNull();
  });

  it('closes without calling onChange on an outside click', () => {
    const onChange = vi.fn();
    act(() => {
      root.render(<Select value="apple" options={FRUIT_OPTIONS} onChange={onChange} ariaLabel="Fruit" />);
    });
    clickTrigger();
    act(() => {
      document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    });
    expect(container.querySelector('[role="listbox"]')).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('closes on Escape', () => {
    act(() => {
      root.render(<Select value="apple" options={FRUIT_OPTIONS} onChange={vi.fn()} ariaLabel="Fruit" />);
    });
    clickTrigger();
    act(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(container.querySelector('[role="listbox"]')).toBeNull();
  });

  it('does not open when disabled', () => {
    act(() => {
      root.render(<Select value="apple" options={FRUIT_OPTIONS} onChange={vi.fn()} ariaLabel="Fruit" disabled />);
    });
    clickTrigger();
    expect(container.querySelector('[role="listbox"]')).toBeNull();
  });

  it('shows the placeholder when value matches no option', () => {
    act(() => {
      root.render(
        <Select value="" options={FRUIT_OPTIONS} onChange={vi.fn()} ariaLabel="Fruit" placeholder="Choose a fruit" />,
      );
    });
    expect(container.textContent).toContain('Choose a fruit');
  });
});
