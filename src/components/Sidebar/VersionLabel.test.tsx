// The version line under the EmailOps wordmark. Release builds show the bare
// version; non-release builds include the commit short sha.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getBuildInfo: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { VersionLabel } from './VersionLabel';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderLabel() {
  await act(async () => {
    root.render(<VersionLabel />);
  });
}

describe('VersionLabel', () => {
  it('shows the bare version for a release build', async () => {
    vi.mocked(api.getBuildInfo).mockResolvedValue({ version: '0.6.2', commit: '05ae613', isRelease: true });
    await renderLabel();
    expect(container.textContent).toBe('v0.6.2');
  });

  it('includes the commit short sha for a non-release build', async () => {
    vi.mocked(api.getBuildInfo).mockResolvedValue({ version: '0.6.2', commit: '05ae613', isRelease: false });
    await renderLabel();
    expect(container.textContent).toBe('v0.6.2 (05ae613)');
  });

  it('renders nothing while build info is unavailable', async () => {
    vi.mocked(api.getBuildInfo).mockRejectedValue(new Error('command failed'));
    await renderLabel();
    expect(container.textContent).toBe('');
  });
});
