import { useTranslation } from 'react-i18next';
import type { CatalogModel, ModelDownloadProgress } from '@/types';
import { formatBytes, formatProgress } from './helpers';

interface ModelRowProps {
  model: CatalogModel;
  isSelected: boolean;
  downloadProgress: ModelDownloadProgress | null;
  onSelect: () => void;
  onDownload: () => void;
  onCancel: () => void;
  onDelete: () => void;
}

export function ModelRow({
  model,
  isSelected,
  downloadProgress,
  onSelect,
  onDownload,
  onCancel,
  onDelete,
}: ModelRowProps) {
  const { t } = useTranslation(['common', 'settings']);
  const isDownloading = downloadProgress !== null && downloadProgress.status === 'downloading';
  const isVerifying = downloadProgress?.status === 'verifying';
  const pct =
    downloadProgress && downloadProgress.totalBytes > 0
      ? Math.round((downloadProgress.downloadedBytes / downloadProgress.totalBytes) * 100)
      : 0;

  return (
    <div
      className={`p-3 rounded border transition-colors ${
        isSelected ? 'border-primary-600 bg-primary-900/20' : 'border-gray-700 bg-[#2a2a2b]'
      }`}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-start gap-2 min-w-0">
          {/* Selection radio — only for downloaded models */}
          {model.isLocal && (
            <button onClick={onSelect} className="mt-0.5 flex-shrink-0" title={t('settings:ai.useThisModelTitle')}>
              <div
                className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                  isSelected ? 'border-primary-500' : 'border-gray-500'
                }`}
              >
                {isSelected && <div className="w-2 h-2 rounded-full bg-primary-500" />}
              </div>
            </button>
          )}
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 flex-wrap">
              <span className="text-sm font-medium text-gray-200">{model.displayName}</span>
              {model.recommended && (
                <span className="text-xs px-1.5 py-0.5 bg-primary-900/50 text-primary-400 rounded border border-primary-800">
                  Recommended
                </span>
              )}
              {model.isLocal && (
                <span className="text-xs px-1.5 py-0.5 bg-green-900/40 text-green-400 rounded border border-green-800">
                  Downloaded
                </span>
              )}
            </div>
            <div className="text-xs text-gray-500 mt-0.5">
              {model.minRamGb}+ GB RAM · {formatBytes(model.sizeBytes)} · {model.license}
              {model.supportsTools && ' · tool-calling'}
            </div>
          </div>
        </div>

        {/* Action buttons */}
        <div className="flex items-center gap-1 flex-shrink-0">
          {model.isLocal ? (
            <button
              onClick={onDelete}
              className="px-2 py-1 text-xs text-red-400 hover:text-red-300 hover:bg-red-900/20 rounded transition-colors"
              title={t('settings:ai.deleteModelFileTitle')}
            >
              Delete
            </button>
          ) : isDownloading || isVerifying ? (
            <button
              onClick={onCancel}
              className="px-2 py-1 text-xs text-gray-400 hover:text-gray-200 hover:bg-gray-700 rounded transition-colors"
            >
              Cancel
            </button>
          ) : (
            <button
              onClick={onDownload}
              className="px-2 py-1 text-xs bg-primary-700 hover:bg-primary-600 text-white rounded transition-colors"
            >
              Download
            </button>
          )}
        </div>
      </div>

      {/* Progress bar */}
      {(isDownloading || isVerifying) && (
        <div className="mt-2">
          <div className="flex items-center justify-between text-xs text-gray-400 mb-1">
            <span>
              {isVerifying
                ? 'Verifying…'
                : formatProgress(downloadProgress!.downloadedBytes, downloadProgress!.totalBytes)}
            </span>
            {isDownloading && <span>{pct}%</span>}
          </div>
          <div className="h-1.5 bg-gray-700 rounded-full overflow-hidden">
            <div
              className={`h-full rounded-full transition-all ${isVerifying ? 'bg-yellow-500' : 'bg-primary-500'}`}
              style={{ width: isVerifying ? '100%' : `${pct}%` }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
