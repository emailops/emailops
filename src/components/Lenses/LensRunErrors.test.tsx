// Each failed run in the history carries its own error detail, collapsed until
// asked for — it used to be one undated list under the whole table.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/lib/api', () => ({
  listLensRunFailures: vi.fn(),
}));

import * as api from '@/lib/api';
import type { LensRunHistoryEntry } from '@/types';
import { LensRunErrors } from './LensRunErrors';

const run: LensRunHistoryEntry = {
  id: 'run-1',
  kind: 'backfill',
  status: 'failed',
  startedAt: 1000,
  finishedAt: 1100,
  processed: 8,
  succeeded: 6,
  failed: 2,
  errorMessage: 'backfill aborted: model timeout',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.listLensRunFailures).mockResolvedValue([
    { emailId: 'e1', subject: 'Invoice 42', sender: 'Vendor', errorMessage: 'not a JSON object', extractedAt: 1050 },
  ]);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe('LensRunErrors', () => {
  it('starts collapsed and loads nothing until opened', () => {
    act(() => root.render(<LensRunErrors lensId="lens-1" run={run} />));
    const details = container.querySelector('details') as HTMLDetailsElement;
    expect(details.open).toBe(false);
    expect(api.listLensRunFailures).not.toHaveBeenCalled();
  });

  it('shows the run error and each failed email once opened', async () => {
    act(() => root.render(<LensRunErrors lensId="lens-1" run={run} />));
    const details = container.querySelector('details') as HTMLDetailsElement;
    await act(async () => {
      details.open = true;
      details.dispatchEvent(new Event('toggle'));
    });
    expect(api.listLensRunFailures).toHaveBeenCalledWith('lens-1', 'run-1');
    expect(container.textContent).toContain('backfill aborted: model timeout');
    expect(container.textContent).toContain('Invoice 42');
    expect(container.textContent).toContain('not a JSON object');
  });
});
