import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { EmailCategory, EmbeddingsConfig } from '@/types';

const ALL_CATEGORIES: EmailCategory[] = ['primary', 'social', 'updates', 'forums', 'promotions'];

const DEFAULT_CONFIG: EmbeddingsConfig = {
  categories: ['primary'],
};

interface AiSearchSettingsProps {
  activeAccountId: string | null;
}

export function AiSearchSettings({ activeAccountId }: AiSearchSettingsProps) {
  const { t } = useTranslation(['common', 'settings']);
  const [config, setConfig] = useState<EmbeddingsConfig>(DEFAULT_CONFIG);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [isRebuilding, setIsRebuilding] = useState(false);
  const [rebuildProgress, setRebuildProgress] = useState<string | null>(null);
  const addLog = useLogStore((s) => s.addLog);

  // Track the embedding rebuild lifecycle so the button reflects backend state
  // even after this dialog is reopened mid-run.
  useEffect(() => {
    const unlisten = listen<{ status: string; message: string }>('embedding-progress', (event) => {
      const { status, message } = event.payload;
      if (status === 'starting' || status === 'clearing' || status === 'generating') {
        setIsRebuilding(true);
        setRebuildProgress(message);
      } else if (status === 'complete') {
        setIsRebuilding(false);
        setRebuildProgress(null);
      } else if (status === 'error') {
        setIsRebuilding(false);
        setRebuildProgress(null);
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (!activeAccountId) {
      setLoading(false);
      return;
    }
    setLoading(true);
    void (async () => {
      try {
        const cfg = await api.getEmbeddingsConfig(activeAccountId);
        setConfig(cfg);
      } catch (e) {
        setError(errorText(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [activeAccountId]);

  const toggleCategory = (id: EmailCategory) => {
    setConfig((prev) => {
      const has = prev.categories.includes(id);
      return {
        ...prev,
        categories: has ? prev.categories.filter((c) => c !== id) : [...prev.categories, id],
      };
    });
  };

  const handleSave = async () => {
    if (!activeAccountId) return;
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      await api.setEmbeddingsConfig(activeAccountId, config);
      setSuccess(t('settings:aiSearch.saved'));
      setTimeout(() => setSuccess(null), 2000);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const handleRebuild = async () => {
    if (!activeAccountId) return;
    setError(null);
    setIsRebuilding(true);
    addLog('info', 'embeddings', t('settings:aiSearch.rebuildLog'));
    try {
      await api.regenerateEmbeddings(activeAccountId);
    } catch (e) {
      setIsRebuilding(false);
      setError(errorText(e));
      addLog('error', 'embeddings', t('settings:aiSearch.rebuildFailLog', { error: errorText(e) }));
    }
  };

  if (!activeAccountId) {
    return <p className="text-gray-400 text-sm p-4">{t('settings:aiSearch.selectAccount')}</p>;
  }
  if (loading) {
    return <p className="text-gray-400 text-sm p-4">{t('common:state.loading')}</p>;
  }

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
        {error && <div className="p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>}
        {success && (
          <div className="p-3 bg-green-900/30 border border-green-800 rounded text-green-300 text-sm">{success}</div>
        )}

        <section>
          <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:aiSearch.categoriesTitle')}</h3>
          <p className="text-xs text-gray-500 mb-3">{t('settings:aiSearch.categoriesHelp')}</p>
          <div className="flex flex-wrap gap-2">
            {ALL_CATEGORIES.map((cat) => {
              const active = config.categories.includes(cat);
              const label =
                cat === 'primary'
                  ? t('settings:aiSearch.catPrimary')
                  : cat === 'social'
                    ? t('settings:aiSearch.catSocial')
                    : cat === 'updates'
                      ? t('settings:aiSearch.catUpdates')
                      : cat === 'forums'
                        ? t('settings:aiSearch.catForums')
                        : t('settings:aiSearch.catPromotions');
              return (
                <button
                  key={cat}
                  type="button"
                  onClick={() => toggleCategory(cat)}
                  className={`px-3 py-1.5 rounded border text-sm transition-colors ${
                    active
                      ? 'bg-primary-700 border-primary-600 text-white'
                      : 'bg-[#2a2a2b] border-gray-700 text-gray-400 hover:border-gray-500'
                  }`}
                >
                  {label}
                </button>
              );
            })}
          </div>
          {config.categories.length === 0 && (
            <p className="text-xs text-amber-400 mt-2">{t('settings:aiSearch.noCategoriesWarn')}</p>
          )}
        </section>

        <section>
          <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:aiSearch.rebuildTitle')}</h3>
          <p className="text-xs text-gray-500 mb-3">{t('settings:aiSearch.rebuildHelp')}</p>
          <button
            type="button"
            onClick={() => void handleRebuild()}
            disabled={isRebuilding || !activeAccountId}
            className="px-3 py-2 rounded border border-gray-600 bg-[#2a2a2b] text-sm text-gray-200 hover:border-gray-500 hover:bg-[#333] disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
          >
            <svg
              className={`w-4 h-4 ${isRebuilding ? 'animate-spin' : ''}`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
              />
            </svg>
            {isRebuilding ? t('settings:aiSearch.rebuilding') : t('settings:aiSearch.rebuildButton')}
          </button>
          {rebuildProgress && <p className="text-xs text-gray-400 mt-2">{rebuildProgress}</p>}
        </section>
      </div>

      {/* Footer */}
      <div className="px-6 py-4 border-t border-gray-700 flex justify-end flex-shrink-0">
        <button
          onClick={() => void handleSave()}
          disabled={saving || !activeAccountId}
          className="px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50"
        >
          {saving ? t('common:state.saving') : t('common:actions.save')}
        </button>
      </div>
    </div>
  );
}
