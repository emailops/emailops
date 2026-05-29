import { useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { formatDate } from '@/lib/intl';
import type { Attachment } from '@/types';
import { AttachmentRow, ruleColor } from './AttachmentRow';

function monthLabel(timestamp: number, locale: string): string {
  const date = new Date(timestamp * 1000);
  const month = formatDate(timestamp, locale, { month: 'long' });
  if (date.getFullYear() === new Date().getFullYear()) return month;
  return `${month} ${date.getFullYear()}`;
}

interface AttachmentListProps {
  attachments: Attachment[];
  selectedAttachment: Attachment | null;
  ruleNames: Record<string, string>;
  checkedIds: Set<string>;
  isLoading: boolean;
  isLoadingMore: boolean;
  hasMore: boolean;
  onSelectAttachment: (attachment: Attachment | null) => void;
  onToggleChecked: (id: string) => void;
  onLoadMore: () => void;
  onOpenRules: () => void;
}

export function AttachmentList({
  attachments,
  selectedAttachment,
  ruleNames,
  checkedIds,
  isLoading,
  isLoadingMore,
  hasMore,
  onSelectAttachment,
  onToggleChecked,
  onLoadMore,
  onOpenRules,
}: AttachmentListProps) {
  const { t, i18n } = useTranslation(['common', 'attachments']);
  const locale = i18n.language || 'en';
  const listRef = useRef<HTMLDivElement>(null);

  const handleScroll = useCallback(() => {
    const el = listRef.current;
    if (!el || !hasMore || isLoadingMore) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 200) {
      onLoadMore();
    }
  }, [hasMore, isLoadingMore, onLoadMore]);

  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    el.addEventListener('scroll', handleScroll);
    return () => el.removeEventListener('scroll', handleScroll);
  }, [handleScroll]);

  return (
    <div className="w-96 border-r border-gray-200 flex flex-col bg-white">
      <div ref={listRef} className="flex-1 overflow-y-auto">
        {isLoading && attachments.length === 0 ? (
          <div className="flex items-center justify-center h-32">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary-600" />
          </div>
        ) : attachments.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-64 px-6 text-center">
            <svg className="w-12 h-12 text-gray-300 mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
              />
            </svg>
            <p className="text-sm text-gray-500">{t('attachments:list.empty')}</p>
            <p className="text-xs text-gray-400 mt-1">{t('attachments:list.createRulesHint')}</p>
            <button
              onClick={onOpenRules}
              className="mt-3 px-4 py-2 text-sm font-medium text-primary-600 hover:text-primary-700 hover:bg-primary-50 rounded-lg transition-colors"
            >
              {t('attachments:list.createRule')}
            </button>
          </div>
        ) : (
          <>
            {attachments.map((att, idx) => {
              const monthKey = monthLabel(att.emailTimestamp, locale);
              const prevMonthKey = idx > 0 ? monthLabel(attachments[idx - 1].emailTimestamp, locale) : null;
              const showSeparator = monthKey !== prevMonthKey;

              return (
                <div key={att.id}>
                  {showSeparator && (
                    <div className="sticky top-0 z-10 px-3 py-1.5 bg-gray-50 border-b border-gray-200">
                      <span className="text-xs font-semibold text-gray-500 uppercase tracking-wider">{monthKey}</span>
                    </div>
                  )}
                  <AttachmentRow
                    attachment={att}
                    ruleName={ruleNames[att.ruleId]}
                    ruleColor={ruleColor(att.ruleId)}
                    isSelected={selectedAttachment?.id === att.id}
                    isChecked={checkedIds.has(att.id)}
                    onToggleChecked={() => onToggleChecked(att.id)}
                    onClick={() => onSelectAttachment(att)}
                  />
                </div>
              );
            })}
            {isLoadingMore && (
              <div className="flex items-center justify-center py-4">
                <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-primary-600" />
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
