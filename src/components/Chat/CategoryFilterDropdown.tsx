// Checkbox dropdown used next to the chat input to pick which Gmail
// categories RAG is allowed to search. Default is `primary` only (keeps
// signal dense and retrieval fast). Selection is persisted via
// chatStore.setSelectedCategories → `chat.default_categories` preference.

import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CHAT_CATEGORY_ORDER, useChatStore } from '@/stores/chatStore';
import type { EmailCategory } from '@/types';

const LABELS: Record<EmailCategory, { label: string; icon: string; hint: string }> = {
  primary: { label: 'Primary', icon: '📥', hint: 'Real people, direct mail' },
  updates: { label: 'Updates', icon: '📦', hint: 'Receipts, shipments, automated' },
  promotions: { label: 'Promotions', icon: '🏷️', hint: 'Offers, newsletters' },
  social: { label: 'Social', icon: '👥', hint: 'LinkedIn, social notifications' },
  forums: { label: 'Forums', icon: '💬', hint: 'Mailing lists, forum posts' },
};

function summaryLabel(cats: EmailCategory[]): string {
  if (cats.length === 0) return 'None';
  if (cats.length === CHAT_CATEGORY_ORDER.length) return 'All categories';
  if (cats.length === 1) return LABELS[cats[0]].label;
  if (cats.length <= 2) return cats.map((c) => LABELS[c].label).join(', ');
  return `${cats.length} categories`;
}

export function CategoryFilterDropdown() {
  const { t } = useTranslation(['common', 'chat']);
  const { selectedCategories, setSelectedCategories } = useChatStore();
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on outside click. Using capture phase so we beat the child onClick
  // handlers that re-open the menu.
  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!containerRef.current) return;
      if (!containerRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [open]);

  const toggle = (cat: EmailCategory) => {
    const has = selectedCategories.includes(cat);
    const next = has ? selectedCategories.filter((c) => c !== cat) : [...selectedCategories, cat];
    void setSelectedCategories(next);
  };

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md border border-gray-200 bg-white text-xs text-gray-600 hover:border-gray-300 hover:bg-gray-50 transition-colors dark:border-gray-700 dark:bg-surface dark:text-gray-400 dark:hover:border-gray-600 dark:hover:bg-surface-raised"
        title={t('chat:categoryDropdown.title')}
      >
        <span className="text-gray-400 dark:text-gray-500">{t('chat:categoryDropdown.searchLabel')}</span>
        <span className="font-medium text-gray-800 dark:text-gray-200">{summaryLabel(selectedCategories)}</span>
        <svg className="w-3 h-3 text-gray-400 dark:text-gray-500" viewBox="0 0 20 20" fill="currentColor">
          <path
            fillRule="evenodd"
            d="M5.23 7.21a.75.75 0 011.06.02L10 11.06l3.71-3.83a.75.75 0 111.08 1.04l-4.24 4.38a.75.75 0 01-1.08 0L5.21 8.27a.75.75 0 01.02-1.06z"
            clipRule="evenodd"
          />
        </svg>
      </button>

      {open && (
        <div className="absolute bottom-full left-0 mb-1 z-20 min-w-[240px] bg-white border border-gray-200 rounded-md shadow-lg py-1 dark:bg-surface dark:border-gray-700">
          <div className="px-3 py-1.5 text-[10px] uppercase tracking-wide text-gray-400 font-medium dark:text-gray-500">
            {t('chat:categoryDropdown.categoriesHeader')}
          </div>
          {CHAT_CATEGORY_ORDER.map((cat) => {
            const meta = LABELS[cat];
            const checked = selectedCategories.includes(cat);
            return (
              <label
                key={cat}
                className="flex items-start gap-2 px-3 py-1.5 hover:bg-gray-50 cursor-pointer text-sm dark:hover:bg-surface-raised"
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => toggle(cat)}
                  className="mt-0.5 rounded border-gray-300 text-primary-600 focus:ring-primary-500 dark:border-gray-600 dark:text-primary-400"
                />
                <div className="flex-1">
                  <div className="flex items-center gap-1.5">
                    <span>{meta.icon}</span>
                    <span className="text-gray-800 dark:text-gray-200">{meta.label}</span>
                  </div>
                  <div className="text-[11px] text-gray-500 dark:text-gray-400">{meta.hint}</div>
                </div>
              </label>
            );
          })}
          <div className="border-t border-gray-100 mt-1 pt-1 flex justify-between px-3 py-1.5 text-[11px] dark:border-gray-800">
            <button
              type="button"
              className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-300"
              onClick={() => void setSelectedCategories(['primary'])}
            >
              {t('chat:categoryDropdown.resetToPrimary')}
            </button>
            <button
              type="button"
              className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-300"
              onClick={() => void setSelectedCategories([...CHAT_CATEGORY_ORDER])}
            >
              {t('common:actions.selectAll')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
