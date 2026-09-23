// Folder chips for a Lens scope: the custom IMAP folders of the selected
// account, toggled in and out of `LensScope.mailboxes` as `folder:<path>`.
// Folders are per-account, so nothing is offered for "All accounts"; Gmail
// and Outlook accounts have no custom folders and render nothing.

import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { Folder } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';

import { folderChips, visibleFolderChips } from './scopeFolders';

/** Above this many folders the list gets a filter field. */
const FILTER_THRESHOLD = 10;

interface LensFolderChipsProps {
  /** '' = all accounts. */
  accountId: string;
  selected: string[];
  onToggle: (mailbox: string) => void;
}

export function LensFolderChips({ accountId, selected, onToggle }: LensFolderChipsProps) {
  const { t } = useTranslation(['lenses']);
  const [folders, setFolders] = useState<Folder[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  useEffect(() => {
    setFolders([]);
    setLoadError(null);
    setQuery('');
    if (!accountId) return;
    let cancelled = false;
    api
      .getFolders(accountId)
      .then((fs) => {
        if (!cancelled) setFolders(fs);
      })
      .catch((e: unknown) => {
        if (!cancelled) setLoadError(errorText(e));
      });
    return () => {
      cancelled = true;
    };
  }, [accountId]);

  if (!accountId) {
    return <p className="text-[10px] text-gray-500">{t('lenses:scope.foldersPickAccount')}</p>;
  }

  const chips = folderChips(folders, selected);
  if (chips.length === 0 && !loadError) return null;
  const showFilter = folders.length > FILTER_THRESHOLD;
  const visible = visibleFolderChips(chips, selected, showFilter ? query : '');

  return (
    <div className="space-y-1">
      <span className="block text-gray-400">{t('lenses:scope.folders')}</span>
      {loadError && (
        <p className="text-[10px] text-red-400">{t('lenses:scope.foldersLoadError', { error: loadError })}</p>
      )}
      {showFilter && (
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('lenses:scope.foldersFilter')}
          aria-label={t('lenses:scope.foldersFilter')}
          className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
        />
      )}
      {/* ~4 rows of chips; a long folder list scrolls here instead of pushing
          the rest of the scope form off-screen. */}
      <div className="flex max-h-28 flex-wrap gap-1.5 overflow-y-auto">
        {visible.map((chip) => (
          <button
            key={chip.value}
            type="button"
            onClick={() => onToggle(chip.value)}
            title={chip.missing ? t('lenses:scope.folderMissing') : chip.label}
            className={`rounded border px-2 py-0.5 text-[11px] ${
              chip.missing
                ? 'border-yellow-600 text-yellow-300 line-through'
                : selected.includes(chip.value)
                  ? 'border-blue-500 bg-blue-600/30 text-blue-200'
                  : 'border-gray-600 text-gray-300 hover:bg-gray-700'
            }`}
          >
            {chip.label}
          </button>
        ))}
      </div>
    </div>
  );
}
