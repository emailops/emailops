import type { TFunction } from 'i18next';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { useToastStore } from '@/stores/toastStore';
import type { BulkDownloadSummary, DownloadSkipReason } from '@/types';

/** Skipped ids named in the log; the rest are only counted. */
const MAX_LOGGED_SKIPPED_IDS = 20;

/**
 * One Logs entry per bulk download: success when everything was copied,
 * otherwise an error with the count per reason and the first skipped ids.
 * Ids only — never paths or file names.
 */
function downloadLogEntry(
  summary: BulkDownloadSummary,
  t: TFunction<['common', 'attachments']>,
): { level: 'success' | 'error'; message: string } {
  const total = summary.copied + summary.skipped.length;
  if (summary.skipped.length === 0) {
    return {
      level: 'success',
      message: t('attachments:toolbar.downloadSummaryLog', { count: total, copied: summary.copied }),
    };
  }
  const perReason = new Map<DownloadSkipReason, number>();
  for (const { reason } of summary.skipped) perReason.set(reason, (perReason.get(reason) ?? 0) + 1);
  const reasons = Array.from(perReason, ([reason, n]) => `${t(`attachments:skipReason.${reason}`)}: ${n}`).join(', ');
  const named = summary.skipped.slice(0, MAX_LOGGED_SKIPPED_IDS).map((s) => s.attachmentId);
  const ids = summary.skipped.length > named.length ? `${named.join(', ')}, …` : named.join(', ');
  return {
    level: 'error',
    message: t('attachments:toolbar.downloadSkippedLog', {
      count: summary.skipped.length,
      copied: summary.copied,
      total,
      reasons,
      ids,
    }),
  };
}

interface AttachmentToolbarProps {
  accountId: string | null;
  totalCount: number;
  selectedTag: string | null;
  availableTags: string[];
  checkedCount: number;
  allChecked: boolean;
  /** Every id of the filter is loading for "select all". */
  isSelectingAll: boolean;
  onSetSelectedTag: (tag: string | null) => void;
  onToggleCheckAll: () => void;
  onClearChecked: () => void;
  checkedIds: Set<string>;
  onOpenRules: () => void;
  /** Pending suggested rules, badged on the "Manage Rules" button. */
  suggestionCount: number;
}

export function AttachmentToolbar({
  accountId,
  totalCount,
  selectedTag,
  availableTags,
  checkedCount,
  allChecked,
  isSelectingAll,
  onSetSelectedTag,
  onToggleCheckAll,
  onClearChecked,
  checkedIds,
  onOpenRules,
  suggestionCount,
}: AttachmentToolbarProps) {
  const { t } = useTranslation(['common', 'attachments']);
  const addLog = useLogStore((s) => s.addLog);
  const addToast = useToastStore((s) => s.addToast);
  const [isDownloading, setIsDownloading] = useState(false);

  const handleBulkDownload = async () => {
    if (checkedCount === 0 || !accountId) return;
    setIsDownloading(true);
    try {
      const summary = await api.bulkDownloadAttachments(accountId, Array.from(checkedIds));
      const { level, message } = downloadLogEntry(summary, t);
      addLog(level, 'attachments', message);
      const total = summary.copied + summary.skipped.length;
      addToast({
        message:
          summary.skipped.length > 0
            ? t('attachments:toolbar.downloadedWithSkippedToast', {
                count: summary.skipped.length,
                copied: summary.copied,
                total,
              })
            : t('attachments:toolbar.downloadedSummaryToast', { count: total, copied: summary.copied }),
        ...(summary.copied > 0 && {
          actionLabel: t('attachments:download.showInFinder'),
          onAction: () => {
            void api.revealInFinder(summary.dest).catch((err) => {
              addLog('error', 'attachments', `Failed to open Downloads folder: ${errorText(err)}`);
            });
          },
        }),
      });
      onClearChecked();
    } catch (err) {
      addLog('error', 'attachments', t('attachments:toolbar.downloadFailedLog', { error: errorText(err) }));
    } finally {
      setIsDownloading(false);
    }
  };

  return (
    <div className="border-b border-gray-200 bg-white px-4 py-3">
      <div className="flex items-center justify-between gap-4">
        {/* Left: title + actions */}
        <div className="flex items-center gap-3 flex-shrink-0">
          <h2 className="text-lg font-semibold text-gray-900">
            {t('attachments:toolbar.title')}
            {totalCount > 0 && <span className="ml-1.5 text-sm font-normal text-gray-400">({totalCount})</span>}
          </h2>

          {/* Select all checkbox */}
          {totalCount > 0 && (
            <label className="flex items-center gap-1.5 cursor-pointer ml-2">
              <input
                type="checkbox"
                checked={allChecked}
                disabled={isSelectingAll}
                onChange={onToggleCheckAll}
                className="w-3.5 h-3.5 rounded border-gray-300 text-primary-600 focus:ring-primary-500"
              />
              <span className="text-xs text-gray-500">
                {checkedCount > 0
                  ? t('attachments:toolbar.selectedCount', { count: checkedCount })
                  : t('attachments:toolbar.selectAll')}
              </span>
            </label>
          )}

          {/* Bulk download */}
          {checkedCount > 0 && (
            <button
              onClick={handleBulkDownload}
              disabled={isDownloading}
              className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors disabled:opacity-50"
            >
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
                />
              </svg>
              {isDownloading
                ? t('attachments:toolbar.downloading')
                : t('attachments:toolbar.download', { count: checkedCount })}
            </button>
          )}
        </div>

        {/* Right: manage rules */}
        <button
          onClick={onOpenRules}
          className="px-3 py-1.5 text-xs font-medium text-primary-600 hover:text-primary-700 hover:bg-primary-50 rounded-lg transition-colors flex-shrink-0 flex items-center gap-1.5"
        >
          {t('attachments:toolbar.manageRules')}
          {suggestionCount > 0 && (
            <span
              className="px-1.5 py-0.5 rounded text-[10px] font-semibold bg-amber-500 text-white"
              title={t('attachments:suggestions.badgeTitle', { count: suggestionCount })}
            >
              {/* The bare number means nothing to a screen reader. */}
              <span aria-hidden="true">{suggestionCount}</span>
              <span className="sr-only">{t('attachments:suggestions.badgeTitle', { count: suggestionCount })}</span>
            </span>
          )}
        </button>
      </div>

      {/* Tag pills — full width, wrapping */}
      {availableTags.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mt-2">
          <button
            onClick={() => onSetSelectedTag(null)}
            className={`px-2.5 py-1 rounded-full text-xs font-medium transition-colors ${
              selectedTag === null ? 'bg-primary-600 text-white' : 'bg-gray-100 text-gray-600 hover:bg-gray-200'
            }`}
          >
            {t('attachments:toolbar.allTags')}
          </button>
          {availableTags.map((tag) => (
            <button
              key={tag}
              onClick={() => onSetSelectedTag(tag === selectedTag ? null : tag)}
              className={`px-2.5 py-1 rounded-full text-xs font-medium transition-colors ${
                selectedTag === tag ? 'bg-primary-600 text-white' : 'bg-gray-100 text-gray-600 hover:bg-gray-200'
              }`}
            >
              {tag}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
