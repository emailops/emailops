// App-level error banner. Sync errors are account-scoped; with multiple
// accounts configured the banner must say WHICH account failed, otherwise
// "Sync error: IMAP connect failed …" is unactionable.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { initI18n } from '@/i18n';
import { ErrorBanner } from './ErrorBanner';

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

describe('ErrorBanner', () => {
  it('shows which account the error belongs to when accountEmail is given', () => {
    act(() => {
      root.render(
        <ErrorBanner
          message="Sync error: IMAP connect failed: Temporary authentication failure."
          accountEmail="work@example.com"
          onDismiss={() => {}}
        />,
      );
    });
    expect(container.textContent).toContain('work@example.com');
    expect(container.textContent).toContain('Sync error: IMAP connect failed');
  });

  it('renders the bare message when no account is associated', () => {
    act(() => {
      root.render(<ErrorBanner message="Failed to load accounts" onDismiss={() => {}} />);
    });
    expect(container.textContent).toContain('Failed to load accounts');
    expect(container.textContent).not.toContain('@');
  });

  it('renders nothing without a message', () => {
    act(() => {
      root.render(<ErrorBanner message={null} accountEmail="work@example.com" onDismiss={() => {}} />);
    });
    expect(container.textContent).toBe('');
  });
});
