import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineError } from '@/components/common/InlineError';
import { Modal } from '@/components/common/Modal';
import { Select } from '@/components/shared/Select';
import { createAgentPanel, deleteAgentPanel, updateAgentPanel } from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { AgentPanel, AgentPanelInput, PanelWindow } from '@/types';

interface AgentPanelDialogProps {
  /** The panel being edited; `null` creates a new one. */
  panel: AgentPanel | null;
  onClose: () => void;
  onChanged: () => void;
}

/** Create, change or delete a stats panel: a title, what to count and over which period. */
export function AgentPanelDialog({ panel, onClose, onChanged }: AgentPanelDialogProps) {
  const { t } = useTranslation(['agent']);
  const [form, setForm] = useState<AgentPanelInput>(
    panel
      ? { title: panel.title, prompt: panel.prompt, window: panel.window }
      : { title: '', prompt: '', window: 'today' },
  );
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const run = async (op: () => Promise<unknown>) => {
    setSaving(true);
    setError(null);
    try {
      await op();
      onChanged();
      onClose();
    } catch (err) {
      setError(`${t('agent:error.save')}: ${errorText(err)}`);
    } finally {
      setSaving(false);
    }
  };

  const windows: { value: PanelWindow; label: string }[] = [
    { value: 'today', label: t('agent:panels.today') },
    { value: 'last7Days', label: t('agent:panels.last7Days') },
    { value: 'last30Days', label: t('agent:panels.last30Days') },
  ];
  const canSave = form.title.trim() && form.prompt.trim() && !saving;

  return (
    <Modal
      open
      onClose={onClose}
      title={t('agent:panels.title')}
      subtitle={t('agent:panels.hint')}
      size="lg"
      footer={
        <div className="flex items-center gap-2">
          {panel && (
            <button
              type="button"
              data-testid="agent-panel-delete"
              disabled={saving}
              onClick={() => void run(() => deleteAgentPanel(panel.id))}
              className="rounded px-3 py-1.5 text-sm text-red-300 hover:bg-red-900/30"
            >
              {t('agent:panels.delete')}
            </button>
          )}
          <span className="flex-1" />
          <button
            type="button"
            onClick={onClose}
            className="rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700"
          >
            {t('agent:panels.cancel')}
          </button>
          <button
            type="button"
            data-testid="agent-panel-save"
            disabled={!canSave}
            onClick={() => void run(() => (panel ? updateAgentPanel(panel.id, form) : createAgentPanel(form)))}
            className="rounded bg-primary-600 px-3 py-1.5 text-sm text-white hover:bg-primary-500 disabled:opacity-50"
          >
            {t('agent:panels.save')}
          </button>
        </div>
      }
    >
      <div className="space-y-3">
        <InlineError message={error} />
        <label className="block text-xs text-gray-400">
          {t('agent:panels.name')}
          <input
            data-testid="agent-panel-title"
            value={form.title}
            maxLength={80}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
            placeholder={t('agent:panels.namePlaceholder')}
            className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1.5 text-sm text-gray-100"
          />
        </label>
        <label className="block text-xs text-gray-400">
          {t('agent:panels.prompt')}
          <textarea
            data-testid="agent-panel-prompt"
            value={form.prompt}
            rows={3}
            maxLength={2000}
            onChange={(e) => setForm({ ...form, prompt: e.target.value })}
            placeholder={t('agent:panels.promptPlaceholder')}
            className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1.5 text-sm text-gray-100"
          />
        </label>
        <div className="text-xs text-gray-400">
          {t('agent:panels.window')}
          <div className="mt-1">
            <Select
              value={form.window}
              options={windows}
              onChange={(window) => setForm({ ...form, window })}
              ariaLabel={t('agent:panels.window')}
              fullWidth
            />
          </div>
        </div>
      </div>
    </Modal>
  );
}
