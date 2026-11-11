import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { ClassificationConfig } from '@/types';
import { type ClassificationRulePrefill, ClassificationRulesTab } from './ClassificationRulesTab';
import { PromptEditorBlock } from './PromptEditorBlock';
import { SettingsPanel } from './SettingsPanel';

export type { ClassificationRulePrefill };

interface ClassificationSettingsProps {
  activeAccountId: string | null;
  prefill?: ClassificationRulePrefill | null;
}

const ALL_CATEGORIES = [
  { id: 'primary', label: 'Primary' },
  { id: 'social', label: 'Social' },
  { id: 'updates', label: 'Updates' },
  { id: 'forums', label: 'Forums' },
  { id: 'promotions', label: 'Promotions' },
] as const;

const DEFAULT_INTENTS = [
  'request',
  'approval',
  'scheduling',
  'delivery',
  'question',
  'introduction',
  'feedback',
  'notification',
  'complaint',
  'promotion',
  'newsletter',
  'conversation',
];

const DEFAULT_TOPICS = [
  'billing',
  'contract',
  'project',
  'hiring',
  'support',
  'legal',
  'sales',
  'operations',
  'networking',
  'education',
  'finance',
  'travel',
  'personal',
  'marketing',
  'security',
];

export function ClassificationSettings({ activeAccountId, prefill }: ClassificationSettingsProps) {
  const { t } = useTranslation(['common', 'settings']);
  const [config, setConfig] = useState<ClassificationConfig | null>(null);
  const [intentsText, setIntentsText] = useState('');
  const [topicsText, setTopicsText] = useState('');
  const [saving, setSaving] = useState(false);
  const [classifying, setClassifying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<'settings' | 'rules'>(prefill ? 'rules' : 'settings');
  const addLog = useLogStore((s) => s.addLog);

  // biome-ignore lint/correctness/useExhaustiveDependencies: load on mount only
  useEffect(() => {
    void loadConfig();
  }, []);

  const loadConfig = async () => {
    setLoading(true);
    setError(null);
    try {
      const cfg = await api.getClassificationConfig();
      setConfig(cfg);
      setIntentsText(cfg.intents.join('\n'));
      setTopicsText(cfg.topics.join('\n'));
    } catch (err) {
      setError(t('settings:classification.loadFailed', { error: errorText(err) }));
    } finally {
      setLoading(false);
    }
  };

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    setError(null);
    setSuccess(null);

    const intents = intentsText
      .split('\n')
      .map((s) => s.trim().toLowerCase())
      .filter((s) => s.length > 0);
    const topics = topicsText
      .split('\n')
      .map((s) => s.trim().toLowerCase())
      .filter((s) => s.length > 0);
    const updated = { ...config, intents, topics };

    try {
      await api.setClassificationConfig(updated);
      setConfig(updated);
      setSuccess(t('settings:classification.saveSuccess'));
      addLog(
        'success',
        'ai',
        t('settings:classification.saveLog', {
          state: updated.enabled
            ? t('settings:classification.saveLogEnabled')
            : t('settings:classification.saveLogDisabled'),
        }),
      );
    } catch (err) {
      setError(t('settings:classification.saveFailed', { error: errorText(err) }));
    } finally {
      setSaving(false);
    }
  };

  const handleClassifyPrevious = async () => {
    if (!activeAccountId) return;
    setClassifying(true);
    try {
      await api.classifyPreviousEmails(activeAccountId);
      setSuccess(t('settings:classification.classifyUnclassifiedSuccess'));
    } catch (err) {
      setError(t('settings:classification.actionFailed', { error: errorText(err) }));
    } finally {
      setClassifying(false);
    }
  };

  const handleReclassifyAll = async () => {
    if (!activeAccountId) return;
    setClassifying(true);
    try {
      await api.reclassifyAllEmails(activeAccountId);
      setSuccess(t('settings:classification.reclassifyAllSuccess'));
      addLog('info', 'ai', t('settings:classification.reclassifyAllLog'));
    } catch (err) {
      setError(t('settings:classification.actionFailed', { error: errorText(err) }));
    } finally {
      setClassifying(false);
    }
  };

  const intents = [...new Set([...(config?.intents ?? []), ...DEFAULT_INTENTS])];
  const topics = [...new Set([...(config?.topics ?? []), ...DEFAULT_TOPICS])];

  if (loading && !config && !error) {
    return (
      <SettingsPanel>
        <p className="text-gray-500">{t('settings:classification.loading')}</p>
      </SettingsPanel>
    );
  }

  if (!config && error) {
    return (
      <SettingsPanel>
        <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm">{error}</div>
        <button onClick={loadConfig} className="px-4 py-2 bg-primary-600 text-white rounded text-sm">
          {t('common:actions.retry')}
        </button>
      </SettingsPanel>
    );
  }

  if (!config) return null;

  return (
    <SettingsPanel>
      {/* Tabs */}
      <div className="flex gap-1 mb-4 border-b border-gray-200">
        <button
          onClick={() => setTab('settings')}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            tab === 'settings'
              ? 'border-primary-500 text-primary-600'
              : 'border-transparent text-gray-500 hover:text-gray-700'
          }`}
        >
          {t('settings:classification.tabSettings')}
        </button>
        <button
          onClick={() => setTab('rules')}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            tab === 'rules'
              ? 'border-primary-500 text-primary-600'
              : 'border-transparent text-gray-500 hover:text-gray-700'
          }`}
        >
          {t('settings:classification.tabRules')}
        </button>
      </div>

      {error && <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm">{error}</div>}
      {success && (
        <div className="mb-4 p-3 bg-green-50 border border-green-200 rounded text-green-700 text-sm">{success}</div>
      )}

      {tab === 'settings' && (
        <div className="space-y-4">
          {/* Enable/Disable */}
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm font-medium text-gray-700">
                {t('settings:classification.autoClassify')}
              </label>
              <p className="text-xs text-gray-500 mt-0.5">{t('settings:classification.autoClassifyDesc')}</p>
            </div>
            <button
              type="button"
              onClick={() => setConfig({ ...config, enabled: !config.enabled })}
              className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ${
                config.enabled ? 'bg-primary-600' : 'bg-gray-300'
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ${
                  config.enabled ? 'translate-x-5' : 'translate-x-0'
                }`}
              />
            </button>
          </div>

          {/* Categories */}
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              {t('settings:classification.classifyIn')}
            </label>
            <p className="text-xs text-gray-500 mb-2">{t('settings:classification.classifyInHelp')}</p>
            <div className="flex flex-wrap gap-2">
              {ALL_CATEGORIES.map(({ id, label }) => {
                const checked = config.categories.includes(id);
                return (
                  <button
                    key={id}
                    type="button"
                    onClick={() => {
                      const next = checked ? config.categories.filter((c) => c !== id) : [...config.categories, id];
                      setConfig({ ...config, categories: next });
                    }}
                    className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-xs font-medium border transition-colors ${
                      checked
                        ? 'bg-primary-50 border-primary-500 text-primary-700'
                        : 'bg-white border-gray-300 text-gray-500 hover:border-gray-400'
                    }`}
                  >
                    <span
                      className={`w-3 h-3 rounded-sm border flex items-center justify-center flex-shrink-0 ${
                        checked ? 'bg-primary-500 border-primary-500' : 'border-gray-400'
                      }`}
                    >
                      {checked && (
                        <svg className="w-2.5 h-2.5 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={3} d="M5 13l4 4L19 7" />
                        </svg>
                      )}
                    </span>
                    {label}
                  </button>
                );
              })}
            </div>
            {config.categories.length === 0 && (
              <p className="text-xs text-yellow-500 mt-1">{t('settings:classification.noCategoriesSelected')}</p>
            )}
          </div>

          {/* Intents */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="block text-sm font-medium text-gray-700">{t('settings:classification.intents')}</label>
              <button
                onClick={() => setIntentsText(DEFAULT_INTENTS.join('\n'))}
                className="text-xs text-primary-600 hover:text-primary-700"
              >
                {t('common:actions.reset')}
              </button>
            </div>
            <textarea
              value={intentsText}
              onChange={(e) => setIntentsText(e.target.value)}
              rows={4}
              className="w-full bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
            />
          </div>

          {/* Topics */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="block text-sm font-medium text-gray-700">{t('settings:classification.topics')}</label>
              <button
                onClick={() => setTopicsText(DEFAULT_TOPICS.join('\n'))}
                className="text-xs text-primary-600 hover:text-primary-700"
              >
                {t('common:actions.reset')}
              </button>
            </div>
            <textarea
              value={topicsText}
              onChange={(e) => setTopicsText(e.target.value)}
              rows={4}
              className="w-full bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
            />
          </div>

          {/* Classification prompt */}
          <div className="pt-2">
            <PromptEditorBlock
              promptId="classify.email"
              title={t('settings:classification.prompt')}
              description={t('settings:classification.promptDescription')}
            />
          </div>

          {/* Actions */}
          <div className="flex gap-2 pt-2">
            <button
              onClick={handleClassifyPrevious}
              disabled={classifying || !activeAccountId}
              className="px-3 py-2 bg-gray-100 text-gray-800 rounded text-sm hover:bg-gray-200 disabled:opacity-50"
            >
              {classifying
                ? t('settings:classification.classifyStarting')
                : t('settings:classification.classifyPrevious')}
            </button>
            <button
              onClick={handleReclassifyAll}
              disabled={classifying || !activeAccountId}
              className="px-3 py-2 bg-gray-100 text-gray-800 rounded text-sm hover:bg-gray-200 disabled:opacity-50"
            >
              {t('settings:classification.reclassifyAll')}
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="flex-1 px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50"
            >
              {saving ? t('settings:classification.saving') : t('common:actions.save')}
            </button>
          </div>
        </div>
      )}

      {tab === 'rules' && (
        <ClassificationRulesTab
          activeAccountId={activeAccountId}
          intents={intents}
          topics={topics}
          prefill={prefill}
          startWithFormOpen={!!prefill}
          onSuccess={setSuccess}
          onError={setError}
        />
      )}
    </SettingsPanel>
  );
}
