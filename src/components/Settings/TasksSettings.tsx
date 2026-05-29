import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { BackfillStatus, EmailCategory, TaskConfig } from '@/types';
import { PromptEditorBlock } from './PromptEditorBlock';

interface TasksSettingsProps {
  activeAccountId: string | null;
  /** Master switch for the AI Tasks feature. Mirrors the `task_enabled`
   *  SQLite preference that the backend extractor reads — toggling this both
   *  hides the sidebar entry and stops task extraction. */
  experimentalEnabled: boolean;
  onChangeExperimentalEnabled: (enabled: boolean) => void;
}

const ALL_CATEGORIES: { id: EmailCategory; label: string }[] = [
  { id: 'primary', label: 'Primary' },
  { id: 'social', label: 'Social' },
  { id: 'updates', label: 'Updates' },
  { id: 'forums', label: 'Forums' },
  { id: 'promotions', label: 'Promotions' },
];

const DEFAULT_CONFIG: TaskConfig = {
  enabled: true,
  extractOnSync: true,
  categories: ['primary'],
  excludedSenders: [],
  excludedTags: ['marketing', 'sales', 'hiring', 'newsletter', 'promotion'],
  maxTasksPerEmail: 1,
  backfillDays: 30,
  extractFromSelfOnly: true,
};

const SUGGESTED_EXCLUDED_TAGS = ['marketing', 'sales', 'hiring', 'newsletter', 'promotion', 'notification', 'receipt'];

export function TasksSettings({
  activeAccountId,
  experimentalEnabled,
  onChangeExperimentalEnabled,
}: TasksSettingsProps) {
  const { t } = useTranslation(['common', 'settings']);
  const [config, setConfig] = useState<TaskConfig>(DEFAULT_CONFIG);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [status, setStatus] = useState<BackfillStatus>({ running: false, remaining: 0 });
  const [newExcluded, setNewExcluded] = useState('');
  const [newExcludedTag, setNewExcludedTag] = useState('');
  const addLog = useLogStore((s) => s.addLog);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: load the global task config once when the panel mounts.
  useEffect(() => {
    void loadConfig();
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: polling is keyed only by the selected account.
  useEffect(() => {
    if (!activeAccountId) return;
    void refreshStatus();
    pollRef.current = setInterval(() => {
      void refreshStatus();
    }, 3000);
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [activeAccountId]);

  async function loadConfig() {
    setLoading(true);
    setError(null);
    try {
      const cfg = await api.getTaskConfig();
      setConfig(cfg);
    } catch (e) {
      setError(t('settings:tasks.loadFailed', { error: errorText(e) }));
    } finally {
      setLoading(false);
    }
  }

  async function handleSave() {
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      // The experimental toggle is the source of truth for `enabled` — force
      // it into the saved config so a stale form value can't clobber a recent
      // toggle change made via the master switch.
      await api.setTaskConfig({ ...config, enabled: experimentalEnabled });
      setSuccess(t('settings:tasks.saveSuccess'));
      addLog('success', 'tasks', t('settings:tasks.saveLog'));
    } catch (e) {
      setError(t('settings:tasks.saveFailed', { error: errorText(e) }));
    } finally {
      setSaving(false);
    }
  }

  async function refreshStatus() {
    if (!activeAccountId) return;
    try {
      const s = await api.getTaskBackfillStatus(activeAccountId);
      setStatus(s);
    } catch {
      // non-fatal
    }
  }

  async function handleStartBackfill() {
    if (!activeAccountId) return;
    setError(null);
    try {
      await api.startTaskBackfill(activeAccountId);
      addLog('info', 'tasks', t('settings:tasks.backfillStarted', { account: activeAccountId }));
      setStatus((s) => ({ ...s, running: true }));
    } catch (e) {
      setError(t('settings:tasks.backfillStartFailed', { error: errorText(e) }));
    }
  }

  async function handleCancelBackfill() {
    setError(null);
    try {
      await api.cancelTaskBackfill();
      addLog('info', 'tasks', t('settings:tasks.backfillCancelRequested'));
    } catch (e) {
      setError(t('settings:tasks.backfillCancelFailed', { error: errorText(e) }));
    }
  }

  async function handleResetExtraction() {
    if (!activeAccountId) return;
    setError(null);
    try {
      const count = await api.resetTaskExtraction(activeAccountId);
      addLog('info', 'tasks', t('settings:tasks.resetExtractionLog', { count }));
      const s = await api.getTaskBackfillStatus(activeAccountId);
      setStatus(s);
    } catch (e) {
      setError(t('settings:tasks.resetExtractionFailed', { error: errorText(e) }));
    }
  }

  function toggleCategory(cat: EmailCategory) {
    setConfig((c) =>
      c.categories.includes(cat)
        ? { ...c, categories: c.categories.filter((x) => x !== cat) }
        : { ...c, categories: [...c.categories, cat] },
    );
  }

  function addExcluded() {
    const v = newExcluded.trim().toLowerCase();
    if (!v) return;
    if (config.excludedSenders.includes(v)) {
      setNewExcluded('');
      return;
    }
    setConfig((c) => ({ ...c, excludedSenders: [...c.excludedSenders, v] }));
    setNewExcluded('');
  }

  function removeExcluded(pattern: string) {
    setConfig((c) => ({
      ...c,
      excludedSenders: c.excludedSenders.filter((x) => x !== pattern),
    }));
  }

  function addExcludedTag(raw?: string) {
    const v = (raw ?? newExcludedTag).trim().toLowerCase();
    if (!v) return;
    setConfig((c) => (c.excludedTags.includes(v) ? c : { ...c, excludedTags: [...c.excludedTags, v] }));
    setNewExcludedTag('');
  }

  function removeExcludedTag(tag: string) {
    setConfig((c) => ({
      ...c,
      excludedTags: c.excludedTags.filter((x) => x !== tag),
    }));
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

        {/* Experimental header */}
        <section className="p-3 rounded-lg border border-amber-700/50 bg-amber-900/10">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium text-gray-100">{t('settings:tasks.title')}</span>
                <span className="px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-amber-900/40 text-amber-300 border border-amber-700/50">
                  {t('settings:dialog.experimental')}
                </span>
              </div>
              <p className="text-xs text-gray-400 mt-1">{t('settings:tasks.experimentalDesc')}</p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={experimentalEnabled}
              onClick={() => onChangeExperimentalEnabled(!experimentalEnabled)}
              className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors mt-0.5 ${
                experimentalEnabled ? 'bg-primary-600' : 'bg-neutral-600'
              }`}
            >
              <span
                className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                  experimentalEnabled ? 'translate-x-5' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        </section>

        {!experimentalEnabled && <p className="text-xs text-gray-500 italic">{t('settings:tasks.enablePrompt')}</p>}

        {experimentalEnabled && (
          <>
            {/* Global toggles. The master "Task extraction enabled" switch
                lives in the experimental header above; this section only
                exposes the downstream behavioural toggles. */}
            <section>
              <ToggleRow
                label={t('settings:tasks.extractOnSync')}
                description={t('settings:tasks.extractOnSyncDesc')}
                enabled={config.extractOnSync}
                onToggle={() => setConfig({ ...config, extractOnSync: !config.extractOnSync })}
              />
              <ToggleRow
                label={t('settings:tasks.selfOnly')}
                description={t('settings:tasks.selfOnlyDesc')}
                enabled={config.extractFromSelfOnly}
                onToggle={() => setConfig({ ...config, extractFromSelfOnly: !config.extractFromSelfOnly })}
              />
            </section>

            {/* Backfill controls */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:tasks.backfill')}</h3>
              <p className="text-xs text-gray-500 mb-3">{t('settings:tasks.backfillDescription')}</p>
              <div className="flex items-center gap-3 text-sm">
                <span className="text-gray-400">
                  {t('settings:tasks.remaining')} <span className="text-gray-200 font-mono">{status.remaining}</span>
                </span>
                <button
                  type="button"
                  disabled={!activeAccountId || status.running}
                  onClick={() => void handleStartBackfill()}
                  className="px-3 py-1.5 bg-primary-600 hover:bg-primary-500 disabled:opacity-50 text-white rounded text-sm"
                >
                  {status.running ? t('settings:tasks.running') : t('settings:tasks.runBackfill')}
                </button>
                {status.running && (
                  <button
                    type="button"
                    onClick={() => void handleCancelBackfill()}
                    className="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-gray-200 rounded text-sm"
                  >
                    {t('common:actions.cancel')}
                  </button>
                )}
                <button
                  type="button"
                  disabled={!activeAccountId || status.running}
                  onClick={() => void handleResetExtraction()}
                  className="px-3 py-1.5 bg-red-900/40 hover:bg-red-900/60 disabled:opacity-50 text-red-200 border border-red-800 rounded text-sm"
                >
                  {t('settings:tasks.resetExtraction')}
                </button>
              </div>
            </section>

            {/* Categories */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:tasks.gmailCategories')}</h3>
              <p className="text-xs text-gray-500 mb-2">{t('settings:tasks.gmailCategoriesDesc')}</p>
              <div className="flex flex-wrap gap-2">
                {ALL_CATEGORIES.map((cat) => {
                  const active = config.categories.includes(cat.id);
                  return (
                    <button
                      key={cat.id}
                      type="button"
                      onClick={() => toggleCategory(cat.id)}
                      className={`px-3 py-1.5 rounded border text-sm transition-colors ${
                        active
                          ? 'bg-primary-700 border-primary-600 text-white'
                          : 'bg-[#2a2a2b] border-gray-700 text-gray-400 hover:border-gray-500'
                      }`}
                    >
                      {cat.label}
                    </button>
                  );
                })}
              </div>
            </section>

            {/* Excluded senders */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:tasks.excludedSenders')}</h3>
              <p className="text-xs text-gray-500 mb-2">
                {t('settings:tasks.excludedSendersDescStart')}{' '}
                <code className="text-gray-400">{t('settings:tasks.excludedSendersExample1')}</code>{' '}
                {t('settings:tasks.excludedSendersDescEnd')}
              </p>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={newExcluded}
                  onChange={(e) => setNewExcluded(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      addExcluded();
                    }
                  }}
                  placeholder={t('settings:tasks.excludedSendersPlaceholder')}
                  className="flex-1 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
                />
                <button
                  type="button"
                  onClick={addExcluded}
                  className="px-3 py-2 bg-primary-600 hover:bg-primary-500 text-white rounded text-sm"
                >
                  {t('common:actions.add')}
                </button>
              </div>
              {config.excludedSenders.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-2">
                  {config.excludedSenders.map((p) => (
                    <span
                      key={p}
                      className="inline-flex items-center gap-1.5 px-2 py-1 bg-[#2a2a2b] border border-gray-700 rounded text-xs text-gray-300 font-mono"
                    >
                      {p}
                      <button
                        type="button"
                        onClick={() => removeExcluded(p)}
                        className="text-gray-500 hover:text-red-400"
                        aria-label={t('settings:tasks.removeAria', { value: p })}
                      >
                        × {/* i18n-ignore */}
                      </button>
                    </span>
                  ))}
                </div>
              )}
            </section>

            {/* Excluded tags */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:tasks.excludedTags')}</h3>
              <p className="text-xs text-gray-500 mb-2">{t('settings:tasks.excludedTagsDesc')}</p>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={newExcludedTag}
                  onChange={(e) => setNewExcludedTag(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      addExcludedTag();
                    }
                  }}
                  placeholder={t('settings:tasks.tagPlaceholder')}
                  className="flex-1 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
                />
                <button
                  type="button"
                  onClick={() => addExcludedTag()}
                  className="px-3 py-2 bg-primary-600 hover:bg-primary-500 text-white rounded text-sm"
                >
                  {t('common:actions.add')}
                </button>
              </div>
              {config.excludedTags.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-2">
                  {config.excludedTags.map((tag) => (
                    <span
                      key={tag}
                      className="inline-flex items-center gap-1.5 px-2 py-1 bg-[#2a2a2b] border border-gray-700 rounded text-xs text-gray-300 font-mono"
                    >
                      {tag}
                      <button
                        type="button"
                        onClick={() => removeExcludedTag(tag)}
                        className="text-gray-500 hover:text-red-400"
                        aria-label={t('settings:tasks.removeAria', { value: tag })}
                      >
                        × {/* i18n-ignore */}
                      </button>
                    </span>
                  ))}
                </div>
              )}
              {SUGGESTED_EXCLUDED_TAGS.some((tag) => !config.excludedTags.includes(tag)) && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <span className="text-xs text-gray-500 self-center">{t('settings:tasks.suggestions')}</span>
                  {SUGGESTED_EXCLUDED_TAGS.filter((tag) => !config.excludedTags.includes(tag)).map((tag) => (
                    <button
                      key={tag}
                      type="button"
                      onClick={() => addExcludedTag(tag)}
                      className="px-2 py-0.5 bg-transparent border border-gray-700 text-gray-400 hover:border-primary-500 hover:text-primary-300 rounded text-xs font-mono"
                    >
                      + {tag} {/* i18n-ignore */}
                    </button>
                  ))}
                </div>
              )}
            </section>

            {/* Tuning */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:tasks.tuning')}</h3>
              <div className="grid grid-cols-2 gap-4">
                <NumberField
                  label={t('settings:tasks.maxTasksPerEmail')}
                  hint={t('settings:tasks.maxTasksPerEmailHint')}
                  min={1}
                  max={10}
                  step={1}
                  value={config.maxTasksPerEmail}
                  onChange={(v) => setConfig({ ...config, maxTasksPerEmail: v })}
                />
                <NumberField
                  label={t('settings:tasks.backfillDays')}
                  hint={t('settings:tasks.backfillDaysHint')}
                  min={0}
                  step={1}
                  value={config.backfillDays}
                  onChange={(v) => setConfig({ ...config, backfillDays: v })}
                />
              </div>
            </section>

            {/* Task extraction prompt */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:tasks.extractionPrompt')}</h3>
              <p className="text-xs text-gray-500 mb-3">{t('settings:tasks.extractionPromptDesc')}</p>
              <PromptEditorBlock
                promptId="tasks.extract"
                title={t('settings:tasks.taskExtractionTitle')}
                description={t('settings:tasks.taskExtractionDesc')}
              />
            </section>
          </>
        )}
      </div>

      {/* Footer — only when the user can actually save changes. */}
      {experimentalEnabled && (
        <div className="px-6 py-4 border-t border-gray-700 flex justify-end flex-shrink-0">
          <button
            onClick={() => void handleSave()}
            disabled={saving}
            className="px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50"
          >
            {saving ? t('common:state.saving') : t('common:actions.save')}
          </button>
        </div>
      )}
    </div>
  );
}

function ToggleRow({
  label,
  description,
  enabled,
  onToggle,
}: {
  label: string;
  description: string;
  enabled: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between py-2">
      <div>
        <label className="block text-sm font-medium text-gray-300">{label}</label>
        <p className="text-xs text-gray-500 mt-0.5">{description}</p>
      </div>
      <button
        type="button"
        onClick={onToggle}
        className={`relative inline-flex h-6 w-11 flex-shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
          enabled ? 'bg-primary-600' : 'bg-gray-600'
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
            enabled ? 'translate-x-5' : 'translate-x-0'
          }`}
        />
      </button>
    </div>
  );
}

function NumberField({
  label,
  hint,
  value,
  onChange,
  min,
  max,
  step,
}: {
  label: string;
  hint?: string;
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
}) {
  return (
    <div>
      <label className="block text-sm font-medium text-gray-300 mb-1">{label}</label>
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(e) => {
          const n = parseFloat(e.target.value);
          if (!Number.isNaN(n)) onChange(n);
        }}
        className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
      />
      {hint && <p className="text-xs text-gray-500 mt-1">{hint}</p>}
    </div>
  );
}
