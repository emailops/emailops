import { describe, expect, it, vi } from 'vitest';
import { CUDA_DOWNLOAD_URL, type GpuRuntimeNoticeHost, showGpuRuntimeNotice } from './gpuRuntimeNotice';

function host(notice: Promise<'cudaRuntimeNotFound' | null>): GpuRuntimeNoticeHost {
  return {
    getNotice: () => notice,
    addToast: vi.fn(() => 1),
    addLog: vi.fn(),
    t: (key: string) => key,
    openUrl: vi.fn(),
  };
}

describe('showGpuRuntimeNotice', () => {
  it('shows a sticky toast with the fix and logs it when the CUDA runtime is missing', async () => {
    const h = host(Promise.resolve('cudaRuntimeNotFound'));
    await showGpuRuntimeNotice(h);

    expect(h.addToast).toHaveBeenCalledTimes(1);
    const toast = vi.mocked(h.addToast).mock.calls[0][0];
    expect(toast.message).toBe('notifications:gpu.cudaRuntimeNotFound');
    expect(toast.sticky).toBe(true);
    expect(toast.actionLabel).toBe('notifications:gpu.downloadCuda');
    toast.onAction?.();
    expect(h.openUrl).toHaveBeenCalledWith(CUDA_DOWNLOAD_URL);
    expect(h.addLog).toHaveBeenCalledWith('error', 'ai', 'notifications:gpu.cudaRuntimeNotFound');
  });

  it('does nothing when there is no notice', async () => {
    const h = host(Promise.resolve(null));
    await showGpuRuntimeNotice(h);

    expect(h.addToast).not.toHaveBeenCalled();
    expect(h.addLog).not.toHaveBeenCalled();
  });

  it('logs the failure and shows no toast when the check itself fails', async () => {
    const h = host(Promise.reject(new Error('ipc down')));
    await showGpuRuntimeNotice(h);

    expect(h.addToast).not.toHaveBeenCalled();
    expect(h.addLog).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('ipc down'));
  });
});
