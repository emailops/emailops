import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type Language, NATIVE_NAMES, SUPPORTED_LANGUAGES, useUiLanguage } from '@/i18n';
import { useAiStore } from '@/stores/aiStore';
import type { Account, InboxLayout } from '@/types';
import { AiDraftsSettings } from './AiDraftsSettings';
import { AiSearchSettings } from './AiSearchSettings';
import { AiSettings } from './AiSettings';
import { CalendarSettings } from './CalendarSettings';
import type { ClassificationRulePrefill } from './ClassificationSettings';
import { ClassificationSettings } from './ClassificationSettings';
import { LensesSettings } from './LensesSettings';
import { MemorySettings } from './MemorySettings';
import { PrivacySettings } from './PrivacySettings';
import { TasksSettings } from './TasksSettings';

export type SettingsTab =
  | 'appearance'
  | 'calendar'
  | 'ai'
  | 'classification'
  | 'tasks'
  | 'memory'
  | 'lenses'
  | 'aidrafts'
  | 'aisearch'
  | 'privacy';

interface SettingsDialogProps {
  initialTab?: SettingsTab;
  activeAccountId: string | null;
  accounts?: Account[];
  currentLayout: InboxLayout;
  onChangeLayout: (layout: InboxLayout) => void;
  classificationPrefill?: ClassificationRulePrefill | null;
  tasksEnabled: boolean;
  onChangeTasksEnabled: (enabled: boolean) => void;
  memoriesEnabled: boolean;
  onChangeMemoriesEnabled: (enabled: boolean) => void;
  lensesEnabled: boolean;
  onChangeLensesEnabled: (enabled: boolean) => void;
  onClose: () => void;
}

type TabSpec = { id: SettingsTab; experimental?: boolean };

// IDs only. Labels and descriptions are pulled from i18n (`settings.tabs.*`)
// at render time so they re-render on language switch without re-mounting
// the dialog.
const ALL_TABS: TabSpec[] = [
  { id: 'appearance' },
  { id: 'calendar' },
  { id: 'ai' },
  { id: 'classification' },
  { id: 'tasks', experimental: true },
  { id: 'memory', experimental: true },
  { id: 'lenses', experimental: true },
  { id: 'aidrafts' },
  { id: 'aisearch' },
  { id: 'privacy' },
];

export function SettingsDialog({
  initialTab = 'appearance',
  activeAccountId,
  accounts = [],
  currentLayout,
  onChangeLayout,
  classificationPrefill,
  tasksEnabled,
  onChangeTasksEnabled,
  memoriesEnabled,
  onChangeMemoriesEnabled,
  lensesEnabled,
  onChangeLensesEnabled,
  onClose,
}: SettingsDialogProps) {
  // Both AI Tasks and AI Memory are always visible in Settings now: each tab
  // owns its experimental enable toggle in its own panel. The underlying
  // sidebar visibility flags (`tasksEnabled` / `memoriesEnabled`) are still
  // wired in so the panels can drive them.
  //
  // The master AI switch (`useAiStore`) hides the AI-feature tabs entirely
  // when off — only Appearance / AI Backend & Models / Privacy remain so the
  // user can re-enable AI from the AI tab.
  const { t } = useTranslation(['common', 'settings']);
  const { enabled: aiEnabled } = useAiStore();
  const visibleTabs = useMemo(
    () =>
      aiEnabled
        ? ALL_TABS
        : ALL_TABS.filter((t) => t.id === 'appearance' || t.id === 'calendar' || t.id === 'ai' || t.id === 'privacy'),
    [aiEnabled],
  );
  const [tab, setTab] = useState<SettingsTab>(() =>
    visibleTabs.some((t) => t.id === initialTab) ? initialTab : 'appearance',
  );

  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(activeAccountId);
  const [pendingAccountId, setPendingAccountId] = useState<string | null>(null);

  const handleOverlayClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onClose();
    },
    [onClose],
  );

  const handleAccountChange = (newId: string) => {
    if (newId === selectedAccountId) return;
    setPendingAccountId(newId);
  };

  const confirmAccountSwitch = () => {
    if (pendingAccountId) {
      setSelectedAccountId(pendingAccountId);
      setPendingAccountId(null);
    }
  };

  const cancelAccountSwitch = () => {
    setPendingAccountId(null);
  };

  const effectiveAccountId = selectedAccountId;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={handleOverlayClick}>
      <div className="bg-[#252526] border border-gray-700 rounded-lg w-full max-w-5xl h-[85vh] shadow-xl flex overflow-hidden">
        {/* Tab rail */}
        <aside className="w-56 flex-shrink-0 border-r border-gray-700 bg-[#1f1f20] py-4 flex flex-col">
          <div className="px-4 pb-3 mb-1 border-b border-gray-700">
            <h2 className="text-sm font-semibold text-gray-100">{t('settings:title')}</h2>
          </div>
          <nav className="flex-1 overflow-y-auto">
            {visibleTabs.map((spec) => (
              <button
                key={spec.id}
                onClick={() => setTab(spec.id)}
                className={`w-full text-left px-4 py-3 border-l-2 transition-colors ${
                  tab === spec.id
                    ? 'border-primary-500 bg-primary-900/20 text-primary-300'
                    : 'border-transparent text-gray-400 hover:text-gray-200 hover:bg-gray-800'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium">{t(`settings:tabs.${spec.id}` as const)}</span>
                  {spec.experimental && (
                    <span className="px-1 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider bg-amber-900/40 text-amber-300 border border-amber-700/50">
                      {t('settings:dialog.experimental')}
                    </span>
                  )}
                </div>
                <div className="text-xs opacity-70 mt-0.5">{t(`settings:tabs.${spec.id}Desc` as const)}</div>
              </button>
            ))}
          </nav>
          <div className="px-4 pt-3 mt-1 border-t border-gray-700">
            <button
              onClick={onClose}
              className="w-full px-3 py-2 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded transition-colors"
            >
              {t('common:actions.close')}
            </button>
          </div>
        </aside>

        {/* Active panel */}
        <div className="flex-1 min-w-0 min-h-0 flex flex-col">
          {/* Header with account selector and close button */}
          <div className="flex items-center justify-between px-4 py-2 border-b border-gray-700 flex-shrink-0">
            {accounts.length > 1 &&
            (tab === 'classification' || tab === 'tasks' || tab === 'memory' || tab === 'aisearch') ? (
              <select
                value={selectedAccountId ?? ''}
                onChange={(e) => handleAccountChange(e.target.value)}
                className="bg-[#333] text-gray-200 border border-gray-600 rounded px-2 py-1 text-sm focus:border-primary-500 outline-none"
              >
                {accounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.email}
                  </option>
                ))}
              </select>
            ) : (
              <div />
            )}
            <button
              onClick={onClose}
              className="p-1 text-gray-400 hover:text-white hover:bg-gray-700 rounded transition-colors"
              title={t('settings:dialog.closeTitle')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>

          {tab === 'appearance' && <AppearancePanel currentLayout={currentLayout} onChangeLayout={onChangeLayout} />}
          {tab === 'calendar' && <CalendarSettings />}
          {tab === 'ai' && <AiSettings onClose={onClose} embedded />}
          {tab === 'classification' && (
            <ClassificationSettings
              onClose={onClose}
              activeAccountId={effectiveAccountId}
              prefill={classificationPrefill ?? null}
              embedded
            />
          )}
          {tab === 'tasks' && (
            <TasksSettings
              activeAccountId={effectiveAccountId}
              experimentalEnabled={tasksEnabled}
              onChangeExperimentalEnabled={onChangeTasksEnabled}
            />
          )}
          {tab === 'memory' && (
            <MemorySettings
              activeAccountId={effectiveAccountId}
              experimentalEnabled={memoriesEnabled}
              onChangeExperimentalEnabled={onChangeMemoriesEnabled}
            />
          )}
          {tab === 'lenses' && (
            <LensesSettings experimentalEnabled={lensesEnabled} onChangeExperimentalEnabled={onChangeLensesEnabled} />
          )}
          {tab === 'aidrafts' && <AiDraftsSettings />}
          {tab === 'aisearch' && <AiSearchSettings activeAccountId={effectiveAccountId} />}
          {tab === 'privacy' && <PrivacySettings />}
        </div>
      </div>

      {/* Account switch confirmation dialog */}
      {pendingAccountId && (
        <div className="fixed inset-0 z-60 flex items-center justify-center bg-black/40">
          <div className="bg-[#2d2d2e] border border-gray-600 rounded-lg p-5 shadow-xl max-w-sm w-full mx-4">
            <h3 className="text-sm font-semibold text-gray-100 mb-2">{t('settings:dialog.switchAccountTitle')}</h3>
            <p className="text-xs text-gray-400 mb-4">{t('settings:dialog.switchAccountConfirm')}</p>
            <div className="flex gap-2 justify-end">
              <button
                onClick={cancelAccountSwitch}
                className="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded transition-colors"
              >
                {t('common:actions.cancel')}
              </button>
              <button
                onClick={confirmAccountSwitch}
                className="px-3 py-1.5 text-sm bg-primary-600 text-white rounded hover:bg-primary-500 transition-colors"
              >
                {t('settings:dialog.switchAction')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function AppearancePanel({
  currentLayout,
  onChangeLayout,
}: {
  currentLayout: InboxLayout;
  onChangeLayout: (layout: InboxLayout) => void;
}) {
  const { t } = useTranslation(['common', 'settings']);
  const { language, setLanguage, isLoading: isLanguageLoading } = useUiLanguage();

  return (
    <div className="flex-1 overflow-y-auto px-6 py-5 space-y-6">
      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:appearance.language')}</h3>
        <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.languageHelp')}</p>
        <select
          value={language}
          disabled={isLanguageLoading}
          onChange={(e) => {
            const next = e.target.value as Language;
            // Fire-and-forget — `useUiLanguage` handles errors internally and
            // the dropdown re-reads from i18n state on next render.
            void setLanguage(next);
          }}
          className="bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none disabled:opacity-60"
        >
          {SUPPORTED_LANGUAGES.map((code) => (
            <option key={code} value={code}>
              {NATIVE_NAMES[code]}
            </option>
          ))}
        </select>
      </section>
      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:appearance.layout')}</h3>
        <div className="grid grid-cols-2 gap-3">
          <LayoutOption
            selected={currentLayout === 'split'}
            onClick={() => onChangeLayout('split')}
            label={t('settings:appearance.layoutSplitLabel')}
            description={t('settings:appearance.layoutSplitDesc')}
            icon={<SplitLayoutIcon />}
          />
          <LayoutOption
            selected={currentLayout === 'full-width'}
            onClick={() => onChangeLayout('full-width')}
            label={t('settings:appearance.layoutFullWidthLabel')}
            description={t('settings:appearance.layoutFullWidthDesc')}
            icon={<FullWidthLayoutIcon />}
          />
        </div>
      </section>
    </div>
  );
}

function LayoutOption({
  selected,
  onClick,
  label,
  description,
  icon,
}: {
  selected: boolean;
  onClick: () => void;
  label: string;
  description: string;
  icon: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex flex-col items-center gap-3 p-4 rounded-lg border-2 text-left transition-colors ${
        selected ? 'border-primary-500 bg-primary-500/10' : 'border-neutral-700 bg-neutral-800 hover:border-neutral-500'
      }`}
    >
      <div className={`w-full ${selected ? 'text-primary-400' : 'text-neutral-400'}`}>{icon}</div>
      <div className="w-full">
        <div className={`text-sm font-medium ${selected ? 'text-white' : 'text-neutral-300'}`}>{label}</div>
        <div className="text-xs text-neutral-500 mt-0.5">{description}</div>
      </div>
    </button>
  );
}

function SplitLayoutIcon() {
  return (
    <svg viewBox="0 0 80 50" className="w-full h-10" fill="none">
      <rect x="1" y="1" width="78" height="48" rx="3" stroke="currentColor" strokeWidth="1.5" />
      <line x1="32" y1="1" x2="32" y2="49" stroke="currentColor" strokeWidth="1.5" />
      <rect x="5" y="7" width="22" height="3" rx="1" fill="currentColor" opacity="0.5" />
      <rect x="5" y="14" width="22" height="3" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="21" width="22" height="3" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="28" width="22" height="3" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="36" y="7" width="38" height="3" rx="1" fill="currentColor" opacity="0.6" />
      <rect x="36" y="14" width="32" height="2" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="36" y="19" width="35" height="2" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="36" y="24" width="28" height="2" rx="1" fill="currentColor" opacity="0.3" />
    </svg>
  );
}

function FullWidthLayoutIcon() {
  return (
    <svg viewBox="0 0 80 50" className="w-full h-10" fill="none">
      <rect x="1" y="1" width="78" height="48" rx="3" stroke="currentColor" strokeWidth="1.5" />
      <rect x="5" y="7" width="70" height="5" rx="1" fill="currentColor" opacity="0.5" />
      <rect x="5" y="16" width="70" height="5" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="25" width="70" height="5" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="34" width="70" height="5" rx="1" fill="currentColor" opacity="0.3" />
    </svg>
  );
}
