// Dates on screen follow the UI language plus the OS region: with the UI in
// English, a US system writes 08/12/2026 and a UK one 12/08/2026. A user in
// the US read the list's fixed day-first date as the wrong day.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { initI18n, setOsRegion } from '@/i18n';
import { useFormatters } from './useFormatters';

const AUGUST_12 = new Date(2026, 7, 12, 9, 5).getTime() / 1000;
const NUMERIC_DATE: Intl.DateTimeFormatOptions = { day: '2-digit', month: '2-digit', year: 'numeric' };

function Dates() {
  const fmt = useFormatters();
  return (
    <>
      <span data-testid="received">{fmt.receivedAt(AUGUST_12)}</span>
      <span data-testid="date">{fmt.date(AUGUST_12, NUMERIC_DATE)}</span>
    </>
  );
}

let root: Root | null = null;
let host: HTMLDivElement | null = null;

function render(): { received: string; date: string } {
  host = document.createElement('div');
  document.body.appendChild(host);
  act(() => {
    root = createRoot(host as HTMLDivElement);
    root.render(<Dates />);
  });
  const text = (id: string) => host?.querySelector(`[data-testid="${id}"]`)?.textContent ?? '';
  return { received: text('received'), date: text('date') };
}

beforeAll(async () => {
  await initI18n('en');
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = null;
  host = null;
  setOsRegion(null);
});

describe('useFormatters', () => {
  it('writes the list date month first on a US system', () => {
    setOsRegion('US');
    expect(render().received).toBe('08/12/2026');
  });

  it('writes the list date day first on a UK system', () => {
    setOsRegion('GB');
    expect(render().received).toBe('12/08/2026');
  });

  it('formats the other dates in the same locale as the list', () => {
    setOsRegion('GB');
    expect(render().date).toBe('12/08/2026');
  });
});
