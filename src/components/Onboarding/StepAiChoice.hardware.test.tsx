// The first-run wizard pairs "N GB+ RAM" with a disk figure for the model it
// means. The disk half was a hardcoded "~5 GB" that matched no model in the
// catalog; it now comes from the backend, like the RAM figure, and both
// describe the same (least demanding) chat model.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { StepAiChoice } from './StepAiChoice';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, string>) =>
      params
        ? `${key}(${Object.entries(params)
            .map(([k, v]) => `${k}=${v}`)
            .join(',')})`
        : key,
  }),
}));
vi.mock('@/stores/aiStore', () => ({ useAiStore: () => ({ setEnabled: vi.fn() }) }));
vi.mock('@/lib/api', () => ({
  detectAiCapability: vi.fn(async () => ({
    appleSilicon: true,
    localAiCapable: true,
    embeddedAiAvailable: true,
    totalRamGb: 16,
    minRamGbForLocalAi: 8,
    minDownloadBytesForLocalAi: 3_013_027_808,
    os: 'macos',
    arch: 'aarch64',
  })),
}));

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

describe('StepAiChoice hardware line', () => {
  it("shows the smallest chat model's own download size, not a fixed figure", async () => {
    await act(async () => {
      root.render(<StepAiChoice onNext={() => {}} />);
    });
    const line = [...container.querySelectorAll('p')]
      .map((p) => p.textContent ?? '')
      .find((s) => s.startsWith('auth:onboarding.aiChoice.useAiHardware'));
    expect(line).toContain('minRam=8');
    expect(line).toContain('minDisk=3.0');
  });
});
