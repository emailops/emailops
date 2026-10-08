import type { GpuRuntimeNotice } from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Toast } from '@/stores/toastStore';

/** Where NVIDIA publishes the CUDA Toolkit installer (it sets `CUDA_PATH`). */
export const CUDA_DOWNLOAD_URL = 'https://developer.nvidia.com/cuda-downloads';

/** The exact `notifications` keys this module resolves — a literal union keeps
 *  `t` compatible with i18next's key-typed translator (as in `appUpdate.ts`). */
type GpuNoticeKey = 'notifications:gpu.cudaRuntimeNotFound' | 'notifications:gpu.downloadCuda';

export interface GpuRuntimeNoticeHost {
  getNotice: () => Promise<GpuRuntimeNotice | null>;
  addToast: (toast: Omit<Toast, 'id'>) => number;
  addLog: (level: 'error', source: 'ai', message: string) => void;
  t: (key: GpuNoticeKey) => string;
  openUrl: (url: string) => void;
}

/** Ask the backend whether the GPU runtime needs the user's attention (e.g. the
 *  CUDA build cannot find the CUDA Toolkit and runs on the CPU) and, if so,
 *  show the fix in a sticky toast and the Output panel. */
export async function showGpuRuntimeNotice(host: GpuRuntimeNoticeHost): Promise<void> {
  let notice: GpuRuntimeNotice | null;
  try {
    notice = await host.getNotice();
  } catch (err) {
    host.addLog('error', 'ai', `Could not check the GPU runtime: ${errorText(err)}`);
    return;
  }
  if (notice !== 'cudaRuntimeNotFound') return;

  const message = host.t('notifications:gpu.cudaRuntimeNotFound');
  host.addToast({
    message,
    actionLabel: host.t('notifications:gpu.downloadCuda'),
    onAction: () => host.openUrl(CUDA_DOWNLOAD_URL),
    sticky: true,
  });
  host.addLog('error', 'ai', message);
}
