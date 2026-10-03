import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineError } from '@/components/common/InlineError';
import { Modal } from '@/components/common/Modal';
import { ToggleSwitch } from '@/components/common/ToggleSwitch';
import { Select } from '@/components/shared/Select';
import { createAgentRule, deleteAgentRule, updateAgentRule } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import type { AgentRule, AgentRuleInput, AgentTrigger } from '@/types';

interface AgentRulesDialogProps {
  open: boolean;
  rules: AgentRule[];
  onClose: () => void;
  /** Called after any rule was created, changed or deleted. */
  onChanged: () => void;
}

const EMPTY: AgentRuleInput = {
  name: '',
  trigger: 'email',
  accountId: null,
  matchPrompt: '',
  actionPrompt: '',
  alwaysApprove: false,
  enabled: true,
};

const ALL_ACCOUNTS = '__all__';

function toInput(rule: AgentRule): AgentRuleInput {
  return {
    name: rule.name,
    trigger: rule.trigger,
    accountId: rule.accountId,
    matchPrompt: rule.matchPrompt,
    actionPrompt: rule.actionPrompt,
    alwaysApprove: rule.alwaysApprove,
    enabled: rule.enabled,
  };
}

/**
 * The agent's rules: a list, and an editor for the one being created or
 * changed. A rule is a classification prompt (which emails or events) plus
 * an action prompt (what to do with them).
 */
export function AgentRulesDialog({ open, rules, onClose, onChanged }: AgentRulesDialogProps) {
  const { t } = useTranslation(['agent']);
  const accounts = useAccountStore((s) => s.accounts);
  // `null` = no editor open; '' = a new rule; otherwise the id being edited.
  const [editing, setEditing] = useState<string | null>(null);
  const [form, setForm] = useState<AgentRuleInput>(EMPTY);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const startEdit = (rule: AgentRule | null) => {
    setError(null);
    setEditing(rule ? rule.id : '');
    setForm(rule ? toInput(rule) : EMPTY);
  };

  const run = async (op: () => Promise<unknown>) => {
    setSaving(true);
    setError(null);
    try {
      await op();
      onChanged();
      return true;
    } catch (err) {
      setError(`${t('agent:error.save')}: ${errorText(err)}`);
      return false;
    } finally {
      setSaving(false);
    }
  };

  const save = async () => {
    const ok = await run(() => (editing ? updateAgentRule(editing, form) : createAgentRule(form)));
    if (ok) setEditing(null);
  };

  const accountOptions = [
    { value: ALL_ACCOUNTS, label: t('agent:rules.allAccounts') },
    ...accounts.map((a) => ({ value: a.id, label: a.email })),
  ];
  const triggerOptions: { value: AgentTrigger; label: string }[] = [
    { value: 'email', label: t('agent:rules.triggerEmail') },
    { value: 'event', label: t('agent:rules.triggerEvent') },
  ];
  const canSave = form.name.trim() && form.matchPrompt.trim() && form.actionPrompt.trim() && !saving;

  return (
    <Modal open={open} onClose={onClose} title={t('agent:rules.title')} subtitle={t('agent:rules.subtitle')} size="2xl">
      <div className="space-y-4" data-testid="agent-rules-dialog">
        <InlineError message={error} />
        {editing === null ? (
          <>
            {rules.length === 0 && <p className="text-sm text-gray-400">{t('agent:rules.empty')}</p>}
            <ul className="space-y-2">
              {rules.map((rule) => (
                <li
                  key={rule.id}
                  data-testid={`agent-rule-${rule.id}`}
                  className="flex items-start gap-3 rounded-lg border border-gray-700 bg-gray-800/50 px-3 py-2"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className={`text-sm font-medium ${rule.enabled ? 'text-gray-100' : 'text-gray-500'}`}>
                        {rule.name}
                      </span>
                      <span className="rounded bg-gray-700 px-1.5 py-0.5 text-[10px] uppercase text-gray-300">
                        {rule.trigger === 'email' ? t('agent:rules.emailBadge') : t('agent:rules.eventBadge')}
                      </span>
                      {!rule.enabled && (
                        <span className="text-[10px] uppercase text-gray-500">{t('agent:rules.off')}</span>
                      )}
                    </div>
                    <p className="mt-1 text-xs text-gray-400 line-clamp-2">{rule.matchPrompt}</p>
                    <p className="text-xs text-gray-500 line-clamp-2">→ {rule.actionPrompt}</p>
                  </div>
                  <button
                    type="button"
                    onClick={() => startEdit(rule)}
                    className="rounded px-2 py-1 text-xs text-gray-300 hover:bg-gray-700"
                  >
                    {t('agent:rules.edit')}
                  </button>
                  <button
                    type="button"
                    data-testid={`agent-rule-delete-${rule.id}`}
                    onClick={() => void run(() => deleteAgentRule(rule.id))}
                    className="rounded px-2 py-1 text-xs text-red-300 hover:bg-red-900/30"
                  >
                    {t('agent:rules.delete')}
                  </button>
                </li>
              ))}
            </ul>
            <button
              type="button"
              data-testid="agent-rule-new"
              onClick={() => startEdit(null)}
              className="rounded bg-primary-600 px-3 py-1.5 text-sm text-white hover:bg-primary-500"
            >
              {t('agent:rules.new')}
            </button>
          </>
        ) : (
          <form
            className="space-y-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (canSave) void save();
            }}
          >
            <label className="block text-xs text-gray-400">
              {t('agent:rules.name')}
              <input
                data-testid="agent-rule-name"
                value={form.name}
                maxLength={80}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                placeholder={t('agent:rules.namePlaceholder')}
                className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1.5 text-sm text-gray-100"
              />
            </label>
            <div className="flex gap-3">
              <div className="flex-1 text-xs text-gray-400">
                {t('agent:rules.trigger')}
                <div className="mt-1">
                  <Select
                    value={form.trigger}
                    options={triggerOptions}
                    onChange={(trigger) => setForm({ ...form, trigger })}
                    ariaLabel={t('agent:rules.trigger')}
                    fullWidth
                  />
                </div>
              </div>
              <div className="flex-1 text-xs text-gray-400">
                {t('agent:rules.account')}
                <div className="mt-1">
                  <Select
                    value={form.accountId ?? ALL_ACCOUNTS}
                    options={accountOptions}
                    onChange={(v) => setForm({ ...form, accountId: v === ALL_ACCOUNTS ? null : v })}
                    ariaLabel={t('agent:rules.account')}
                    fullWidth
                  />
                </div>
              </div>
            </div>
            <label className="block text-xs text-gray-400">
              {t('agent:rules.matchPrompt')}
              <textarea
                data-testid="agent-rule-match"
                value={form.matchPrompt}
                rows={3}
                maxLength={2000}
                onChange={(e) => setForm({ ...form, matchPrompt: e.target.value })}
                placeholder={t('agent:rules.matchPlaceholder')}
                className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1.5 text-sm text-gray-100"
              />
            </label>
            <label className="block text-xs text-gray-400">
              {t('agent:rules.actionPrompt')}
              <textarea
                data-testid="agent-rule-action"
                value={form.actionPrompt}
                rows={3}
                maxLength={2000}
                onChange={(e) => setForm({ ...form, actionPrompt: e.target.value })}
                placeholder={t('agent:rules.actionPlaceholder')}
                className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1.5 text-sm text-gray-100"
              />
            </label>
            <ToggleSwitch
              checked={form.alwaysApprove}
              onChange={(alwaysApprove) => setForm({ ...form, alwaysApprove })}
              label={t('agent:rules.alwaysApprove')}
              description={t('agent:rules.alwaysApproveHint')}
            />
            <ToggleSwitch
              checked={form.enabled}
              onChange={(enabled) => setForm({ ...form, enabled })}
              label={t('agent:rules.enabled')}
            />
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setEditing(null)}
                className="rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700"
              >
                {t('agent:rules.cancel')}
              </button>
              <button
                type="submit"
                data-testid="agent-rule-save"
                disabled={!canSave}
                className="rounded bg-primary-600 px-3 py-1.5 text-sm text-white hover:bg-primary-500 disabled:opacity-50"
              >
                {t('agent:rules.save')}
              </button>
            </div>
          </form>
        )}
      </div>
    </Modal>
  );
}
