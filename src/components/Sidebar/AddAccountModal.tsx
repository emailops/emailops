import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

type SyncHistoryPreset = '7d' | '30d' | '90d' | '365d' | 'all' | 'custom';

interface AddAccountModalProps {
  onClose: () => void;
  onConfirm: (syncFromTimestamp: number | null) => void | Promise<void>; // i18n-ignore
  /** Optional external submission flag; modal also tracks its own in-flight state so reopening resets. */
  isSubmitting?: boolean;
  providerLabel?: string;
  title?: string;
  description?: string;
  confirmLabel?: string;
  submittingLabel?: string;
  initialSyncFromTimestamp?: number | null;
  warningMessage?: string;
}

function buildPresets(providerLabel: string): { id: SyncHistoryPreset; label: string; description: string }[] {
  return [
    { id: '7d', label: 'Last 7 days', description: 'Fastest setup, recent email only.' },
    { id: '30d', label: 'Last 30 days', description: 'Good default for lightweight onboarding.' },
    { id: '90d', label: 'Last 90 days', description: 'Useful if you need recent project history.' },
    { id: '365d', label: 'Last year', description: 'Broader history without syncing everything.' },
    { id: 'all', label: 'All mail', description: `Import everything available from ${providerLabel}.` },
    { id: 'custom', label: 'Custom date', description: 'Choose the first date to sync from.' },
  ];
}

function dateInputDefaultValue(): string {
  const now = new Date();
  now.setMonth(now.getMonth() - 1);
  return formatDateInputValue(now);
}

function stateFromInitialTimestamp(initialSyncFromTimestamp?: number | null): {
  preset: SyncHistoryPreset;
  customDate: string;
} {
  if (initialSyncFromTimestamp == null) {
    return { preset: 'all', customDate: dateInputDefaultValue() };
  }

  const now = new Date();
  now.setHours(0, 0, 0, 0);
  const selected = new Date(initialSyncFromTimestamp * 1000);
  selected.setHours(0, 0, 0, 0);
  const diffDays = Math.round((now.getTime() - selected.getTime()) / (24 * 60 * 60 * 1000));

  const preset =
    diffDays === 7 ? '7d' : diffDays === 30 ? '30d' : diffDays === 90 ? '90d' : diffDays === 365 ? '365d' : 'custom';

  return {
    preset,
    customDate: formatDateInputValue(selected),
  };
}

function formatDateInputValue(date: Date): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, '0');
  const day = `${date.getDate()}`.padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function presetToTimestamp(preset: SyncHistoryPreset, customDate: string): number | null {
  if (preset === 'all') {
    return null;
  }

  const date = new Date();
  date.setHours(0, 0, 0, 0);

  switch (preset) {
    case '7d':
      date.setDate(date.getDate() - 7);
      return Math.floor(date.getTime() / 1000);
    case '30d':
      date.setDate(date.getDate() - 30);
      return Math.floor(date.getTime() / 1000);
    case '90d':
      date.setDate(date.getDate() - 90);
      return Math.floor(date.getTime() / 1000);
    case '365d':
      date.setDate(date.getDate() - 365);
      return Math.floor(date.getTime() / 1000);
    case 'custom': {
      const selected = new Date(`${customDate}T00:00:00`);
      if (Number.isNaN(selected.getTime())) {
        return null;
      }
      return Math.floor(selected.getTime() / 1000);
    }
  }
}

export function AddAccountModal({
  isSubmitting: externalSubmitting,
  onClose,
  onConfirm,
  providerLabel = 'Gmail',
  title,
  description = 'Choose how much email history to import on the first sync.',
  confirmLabel,
  submittingLabel = 'Connecting...',
  initialSyncFromTimestamp,
  warningMessage,
}: AddAccountModalProps) {
  const { t } = useTranslation(['common', 'modal']);
  const initialState = useMemo(() => stateFromInitialTimestamp(initialSyncFromTimestamp), [initialSyncFromTimestamp]);
  const [preset, setPreset] = useState<SyncHistoryPreset>(initialState.preset);
  const [customDate, setCustomDate] = useState(initialState.customDate);
  // Local in-flight flag — fresh on every mount so reopening after a stuck parent state resets the button.
  // Track in-flight state locally so reopening the modal after cancelling a stuck attempt
  // resets the button. A stale `externalSubmitting` prop is intentionally ignored on mount.
  const [isSubmitting, setLocalSubmitting] = useState(false);
  void externalSubmitting;
  const resolvedTitle = title ?? `Add ${providerLabel} Account`;
  const resolvedConfirm = confirmLabel ?? `Connect ${providerLabel}`;
  const presets = useMemo(() => buildPresets(providerLabel), [providerLabel]);

  const syncFromTimestamp = useMemo(() => presetToTimestamp(preset, customDate), [preset, customDate]);

  const isCustomDateInvalid = preset === 'custom' && syncFromTimestamp === null;

  const handleConfirm = async () => {
    if (isSubmitting) return;
    setLocalSubmitting(true);
    try {
      await onConfirm(syncFromTimestamp);
    } finally {
      setLocalSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="w-full max-w-lg rounded-xl bg-[#1f1f20] shadow-2xl">
        <div className="border-b border-gray-700 px-6 py-4">
          <h2 className="text-lg font-semibold text-gray-100">{resolvedTitle}</h2>
          <p className="mt-1 text-sm text-gray-500">{description}</p>
        </div>

        <div className="space-y-3 px-6 py-5">
          {presets.map((option) => {
            const selected = preset === option.id;
            return (
              <label
                key={option.id}
                className={`block cursor-pointer rounded-lg border p-3 transition-colors ${
                  selected ? 'border-primary-500 bg-primary-900/20' : 'border-gray-700 hover:border-gray-500'
                }`}
              >
                <div className="flex items-start gap-3">
                  <input
                    type="radio"
                    name="syncHistory"
                    value={option.id}
                    checked={selected}
                    onChange={() => setPreset(option.id)}
                    className="mt-1 h-4 w-4 border-gray-700 text-primary-600 focus:ring-primary-500"
                  />
                  <div>
                    <div className="text-sm font-medium text-gray-100">{option.label}</div>
                    <div className="text-sm text-gray-500">{option.description}</div>
                  </div>
                </div>
              </label>
            );
          })}

          {preset === 'custom' && (
            <div className="rounded-lg border border-gray-700 bg-[#27272a] p-3">
              <label className="block text-sm font-medium text-gray-300" htmlFor="sync-from-date">
                {t('modal:accountSettings.syncSince')}
              </label>
              <input
                id="sync-from-date"
                type="date"
                value={customDate}
                max={formatDateInputValue(new Date())}
                onChange={(event) => setCustomDate(event.target.value)}
                className="mt-2 w-full rounded-lg border border-gray-700 bg-[#1f1f20] text-gray-100 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40 [color-scheme:dark]"
              />
              {isCustomDateInvalid && <p className="mt-2 text-xs text-red-400">Pick a valid date to continue.</p>}
            </div>
          )}

          {warningMessage && (
            <div className="rounded-lg border border-amber-800 bg-amber-900/20 px-3 py-2 text-sm text-amber-200">
              {warningMessage}
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-3 border-t border-gray-700 px-6 py-4">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-4 py-2 text-sm font-medium text-gray-400 hover:bg-gray-800"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleConfirm}
            disabled={isSubmitting || isCustomDateInvalid}
            className="rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isSubmitting ? submittingLabel : resolvedConfirm}
          </button>
        </div>
      </div>
    </div>
  );
}
