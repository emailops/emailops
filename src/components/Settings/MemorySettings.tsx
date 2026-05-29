import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { BackfillStatus, EmailCategory, MemoryConfig } from '@/types';
import { PromptEditorBlock } from './PromptEditorBlock';

interface MemorySettingsProps {
  activeAccountId: string | null;
  /** Master switch for the AI Memory feature. Mirrors the `memory_enabled`
   *  SQLite preference that the backend extractor reads — toggling this both
   *  hides the sidebar entry and stops fact extraction. */
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

const DEFAULT_CONFIG: MemoryConfig = {
  enabled: true,
  extractOnSync: true,
  categories: ['primary'],
  excludedSenders: [],
  excludedTags: ['marketing', 'sales', 'hiring', 'newsletter', 'promotion'],
  consolidationIntervalMinutes: 30,
  promoteThreshold: 0.75,
  candidateTtlDays: 14,
  eventRetentionDays: 30,
  backfillBatchSize: 50,
  extractFromSelfOnly: true,
  aiOutputLanguage: 'Spanish',
};

/** Tag values we surface as quick-add suggestions in the exclusion chip input.
 * These align with the classification taxonomy in services/classification.rs. */
const SUGGESTED_EXCLUDED_TAGS = ['marketing', 'sales', 'hiring', 'newsletter', 'promotion', 'notification', 'receipt'];

export function MemorySettings({
  activeAccountId,
  experimentalEnabled,
  onChangeExperimentalEnabled,
}: MemorySettingsProps) {
  const { t } = useTranslation(['common', 'settings']);
  const [config, setConfig] = useState<MemoryConfig>(DEFAULT_CONFIG);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [status, setStatus] = useState<BackfillStatus>({ running: false, remaining: 0 });
  const [newExcluded, setNewExcluded] = useState('');
  const [newExcludedTag, setNewExcludedTag] = useState('');
  const addLog = useLogStore((s) => s.addLog);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    void loadConfig();
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  useEffect(() => {
    if (!activeAccountId) return;
    void refreshStatus();
    // Poll while a backfill is running so remaining/running updates live.
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
      const cfg = await api.getMemoryConfig();
      setConfig(cfg);
    } catch (e) {
      setError(t('settings:memory.loadFailed', { error: errorText(e) }));
    } finally {
      setLoading(false);
    }
  }

  async function refreshStatus() {
    if (!activeAccountId) return;
    try {
      const s = await api.getMemoryBackfillStatus(activeAccountId);
      setStatus(s);
    } catch {
      // non-fatal
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
      await api.setMemoryConfig({ ...config, enabled: experimentalEnabled });
      setSuccess(t('settings:memory.saveSuccess'));
      addLog('success', 'memory', t('settings:memory.saveLog'));
    } catch (e) {
      setError(t('settings:memory.saveFailed', { error: errorText(e) }));
    } finally {
      setSaving(false);
    }
  }

  async function handleStartBackfill() {
    if (!activeAccountId) return;
    setError(null);
    try {
      await api.startMemoryBackfill(activeAccountId);
      addLog('info', 'memory', t('settings:memory.backfillStarted', { account: activeAccountId }));
      setStatus((s) => ({ ...s, running: true }));
    } catch (e) {
      setError(t('settings:memory.backfillStartFailed', { error: errorText(e) }));
    }
  }

  async function handleCancelBackfill() {
    try {
      await api.cancelMemoryBackfill();
      addLog('info', 'memory', t('settings:memory.backfillCancelRequested'));
    } catch (e) {
      setError(t('settings:memory.backfillCancelFailed', { error: errorText(e) }));
    }
  }

  async function handleResetExtraction() {
    if (!activeAccountId) return;
    setError(null);
    try {
      const count = await api.resetMemoryExtraction(activeAccountId);
      addLog('info', 'memory', t('settings:memory.resetExtractionLog', { count }));
      // Refresh remaining count
      const s = await api.getMemoryBackfillStatus(activeAccountId);
      setStatus(s);
    } catch (e) {
      setError(t('settings:memory.resetExtractionFailed', { error: errorText(e) }));
    }
  }

  async function handleRunConsolidation() {
    if (!activeAccountId) return;
    setError(null);
    try {
      await api.runMemoryConsolidation(activeAccountId);
      addLog('info', 'memory', t('settings:memory.consolidationLog'));
    } catch (e) {
      setError(t('settings:memory.consolidationFailed', { error: errorText(e) }));
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
                <span className="text-sm font-medium text-gray-100">{t('settings:memory.title')}</span>
                <span className="px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-amber-900/40 text-amber-300 border border-amber-700/50">
                  {t('settings:dialog.experimental')}
                </span>
              </div>
              <p className="text-xs text-gray-400 mt-1">{t('settings:memory.experimentalDesc')}</p>
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

        {!experimentalEnabled && <p className="text-xs text-gray-500 italic">{t('settings:memory.enablePrompt')}</p>}

        {experimentalEnabled && (
          <>
            {/* Global toggle. The master "Memory enabled" switch lives in the
                experimental header above; this section only exposes the
                downstream behavioural toggles. */}
            <section>
              <ToggleRow
                label={t('settings:memory.extractOnSync')}
                description={t('settings:memory.extractOnSyncDesc')}
                enabled={config.extractOnSync}
                onToggle={() => setConfig({ ...config, extractOnSync: !config.extractOnSync })}
              />
              <ToggleRow
                label={t('settings:memory.selfOnly')}
                description={t('settings:memory.selfOnlyDesc')}
                enabled={config.extractFromSelfOnly}
                onToggle={() => setConfig({ ...config, extractFromSelfOnly: !config.extractFromSelfOnly })}
              />
            </section>

            {/* Categories */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:memory.gmailCategories')}</h3>
              <p className="text-xs text-gray-500 mb-2">{t('settings:memory.gmailCategoriesDesc')}</p>
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
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:memory.excludedSenders')}</h3>
              <p className="text-xs text-gray-500 mb-2">
                {t('settings:memory.excludedSendersDescStart')}{' '}
                <code className="text-gray-400">{t('settings:memory.excludedSendersExample1')}</code>{' '}
                {t('settings:memory.excludedSendersDescMiddle')}{' '}
                <code className="text-gray-400">{t('settings:memory.excludedSendersExample2')}</code>{' '}
                {t('settings:memory.excludedSendersDescEnd')}
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
                  placeholder={t('settings:memory.excludedSendersPlaceholder')}
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
                        aria-label={t('settings:memory.removeAria', { value: p })}
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
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:memory.excludedTags')}</h3>
              <p className="text-xs text-gray-500 mb-2">{t('settings:memory.excludedTagsDesc')}</p>
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
                  placeholder={t('settings:memory.tagPlaceholder')}
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
                        aria-label={t('settings:memory.removeAria', { value: tag })}
                      >
                        × {/* i18n-ignore */}
                      </button>
                    </span>
                  ))}
                </div>
              )}
              {/* Suggestions: click to add, hidden once already present. */}
              {SUGGESTED_EXCLUDED_TAGS.some((tag) => !config.excludedTags.includes(tag)) && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <span className="text-xs text-gray-500 self-center">{t('settings:memory.suggestions')}</span>
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
              <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:memory.tuning')}</h3>
              <div className="grid grid-cols-2 gap-4">
                <NumberField
                  label={t('settings:memory.consolidationInterval')}
                  hint={t('settings:memory.consolidationIntervalHint')}
                  min={0}
                  step={5}
                  value={config.consolidationIntervalMinutes}
                  onChange={(v) => setConfig({ ...config, consolidationIntervalMinutes: v })}
                />
                <NumberField
                  label={t('settings:memory.promoteThreshold')}
                  hint={t('settings:memory.promoteThresholdHint')}
                  min={0}
                  max={1}
                  step={0.05}
                  value={config.promoteThreshold}
                  onChange={(v) => setConfig({ ...config, promoteThreshold: v })}
                />
                <NumberField
                  label={t('settings:memory.candidateTtl')}
                  hint={t('settings:memory.candidateTtlHint')}
                  min={1}
                  step={1}
                  value={config.candidateTtlDays}
                  onChange={(v) => setConfig({ ...config, candidateTtlDays: v })}
                />
                <NumberField
                  label={t('settings:memory.eventRetention')}
                  hint={t('settings:memory.eventRetentionHint')}
                  min={1}
                  step={1}
                  value={config.eventRetentionDays}
                  onChange={(v) => setConfig({ ...config, eventRetentionDays: v })}
                />
                <NumberField
                  label={t('settings:memory.backfillBatchSize')}
                  hint={t('settings:memory.backfillBatchSizeHint')}
                  min={1}
                  max={500}
                  step={1}
                  value={config.backfillBatchSize}
                  onChange={(v) => setConfig({ ...config, backfillBatchSize: v })}
                />
              </div>
            </section>

            {/* Backfill + consolidation actions */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:memory.backfillSection')}</h3>
              <p className="text-xs text-gray-500 mb-3">{t('settings:memory.backfillSectionDesc')}</p>
              {!activeAccountId && (
                <p className="text-xs text-amber-400 mb-3">{t('settings:memory.selectAccountWarn')}</p>
              )}
              <div className="flex items-center gap-2 flex-wrap">
                {status.running ? (
                  <button
                    type="button"
                    onClick={() => void handleCancelBackfill()}
                    className="px-3 py-2 bg-red-700 hover:bg-red-600 text-white rounded text-sm"
                  >
                    {t('settings:memory.cancelBackfill')}
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={() => void handleStartBackfill()}
                    disabled={!activeAccountId || !experimentalEnabled}
                    className="px-3 py-2 bg-primary-600 hover:bg-primary-500 text-white rounded text-sm disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    {t('settings:memory.startBackfill')}
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => void handleRunConsolidation()}
                  disabled={!activeAccountId}
                  className="px-3 py-2 bg-gray-700 hover:bg-gray-600 text-gray-200 rounded text-sm disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {t('settings:memory.runConsolidation')}
                </button>
                <button
                  type="button"
                  onClick={() => void handleResetExtraction()}
                  disabled={!activeAccountId || status.running}
                  className="px-3 py-2 bg-gray-700 hover:bg-red-900 text-gray-400 hover:text-red-300 rounded text-sm disabled:opacity-50 disabled:cursor-not-allowed"
                  title={t('settings:memory.resetExtractionTitle')}
                >
                  {t('settings:memory.resetExtraction')}
                </button>
                <div className="text-xs text-gray-400 ml-2">
                  {status.running ? (
                    <span>
                      {t('settings:memory.runningWithRemaining')}
                      <span className="text-gray-500">
                        {t('settings:memory.runningRemaining', { count: status.remaining })}
                      </span>
                    </span>
                  ) : (
                    <span>{t('settings:memory.eligibleForExtraction', { count: status.remaining })}</span>
                  )}
                </div>
              </div>
            </section>

            {/* Fact extraction prompt */}
            <section>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:memory.extractionPrompt')}</h3>
              <p className="text-xs text-gray-500 mb-3">{t('settings:memory.extractionPromptDesc')}</p>
              <PromptEditorBlock
                promptId="memory.extract_facts"
                title={t('settings:memory.factExtractionTitle')}
                description={t('settings:memory.factExtractionDesc')}
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
