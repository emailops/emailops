import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { AttachmentRule } from '@/types';

export interface RuleFormPrefill {
  name: string;
  senderEmailPattern: string;
  subjectPattern: string;
}

interface RuleManagementModalProps {
  rules: AttachmentRule[];
  accountId: string;
  prefill?: RuleFormPrefill | null;
  onClose: () => void;
  onCreateRule: (
    name: string,
    senderEmailPattern: string | null,
    subjectPattern: string | null,
    filenamePattern: string | null,
    tags: string[],
  ) => Promise<AttachmentRule>;
  onUpdateRule: (
    ruleId: string,
    name: string,
    senderEmailPattern: string | null,
    subjectPattern: string | null,
    filenamePattern: string | null,
    tags: string[],
    enabled: boolean,
  ) => Promise<AttachmentRule>;
  onDeleteRule: (ruleId: string) => Promise<void>;
  onRefreshAfterApply: () => void;
}

interface RuleFormState {
  name: string;
  senderEmailPattern: string;
  subjectPattern: string;
  filenamePattern: string;
  tags: string;
  applyToExisting: boolean;
}

const EMPTY_FORM: RuleFormState = {
  name: '',
  senderEmailPattern: '',
  subjectPattern: '',
  filenamePattern: '',
  tags: '',
  applyToExisting: true,
};

export function RuleManagementModal({
  rules,
  accountId,
  prefill,
  onClose,
  onCreateRule,
  onUpdateRule,
  onDeleteRule,
  onRefreshAfterApply,
}: RuleManagementModalProps) {
  const { t } = useTranslation(['common', 'attachments']);
  const hasPrefill = !!prefill;
  const [showForm, setShowForm] = useState(hasPrefill);
  const [editingRuleId, setEditingRuleId] = useState<string | null>(null);
  const [form, setForm] = useState<RuleFormState>(
    prefill
      ? {
          name: prefill.name,
          senderEmailPattern: prefill.senderEmailPattern,
          subjectPattern: prefill.subjectPattern,
          filenamePattern: '',
          tags: '',
          applyToExisting: true,
        }
      : EMPTY_FORM,
  );
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [applyingRuleId, setApplyingRuleId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Delete-confirm flow: clicking the trash icon opens an inline warning
  // panel showing how many saved files will also be removed. Inline rather
  // than window.confirm() because Tauri's webview blocks native dialogs and
  // they made earlier deletion attempts fail silently.
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [pendingDeleteCount, setPendingDeleteCount] = useState<number | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const addLog = useLogStore((s) => s.addLog);

  // Fetch the attachment count when a rule is armed for deletion. Done in an
  // effect (not inline) so the warning panel can show a loading state instead
  // of jumping from "no count" to "12 files" mid-render.
  useEffect(() => {
    if (!pendingDeleteId) {
      setPendingDeleteCount(null);
      return;
    }
    let cancelled = false;
    setPendingDeleteCount(null);
    api
      .countAttachmentsForRule(pendingDeleteId)
      .then((n) => {
        if (!cancelled) setPendingDeleteCount(n);
      })
      .catch((err) => {
        if (!cancelled) {
          addLog('error', 'attachments', `Failed to count rule attachments: ${err}`);
          setPendingDeleteCount(0);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [pendingDeleteId, addLog]);

  const resetForm = () => {
    setForm(EMPTY_FORM);
    setShowForm(false);
    setEditingRuleId(null);
    setError(null);
  };

  const startEditing = (rule: AttachmentRule) => {
    setForm({
      name: rule.name,
      senderEmailPattern: rule.senderEmailPattern ?? '',
      subjectPattern: rule.subjectPattern ?? '',
      filenamePattern: rule.filenamePattern ?? '',
      tags: rule.tags.join(', '),
      applyToExisting: false,
    });
    setEditingRuleId(rule.id);
    setShowForm(true);
    setError(null);
  };

  const handleSubmit = async () => {
    const trimmedName = form.name.trim();
    const sender = form.senderEmailPattern.trim() || null;
    const subject = form.subjectPattern.trim() || null;
    const filename = form.filenamePattern.trim() || null;
    const tags = form.tags
      .split(',')
      .map((t) => t.trim())
      .filter((t) => t.length > 0);

    if (!trimmedName) {
      setError(t('attachments:rules.nameRequired'));
      return;
    }
    if (!sender && !subject && !filename) {
      setError(t('attachments:rules.patternRequired'));
      return;
    }

    setIsSubmitting(true);
    setError(null);

    try {
      if (editingRuleId) {
        const existing = rules.find((r) => r.id === editingRuleId);
        await onUpdateRule(editingRuleId, trimmedName, sender, subject, filename, tags, existing?.enabled ?? true);
      } else {
        const newRule = await onCreateRule(trimmedName, sender, subject, filename, tags);
        if (form.applyToExisting) {
          setApplyingRuleId(newRule.id);
          try {
            addLog('info', 'attachments', `Scanning existing emails for rule "${trimmedName}"...`);
            const count = await api.applyRuleRetroactively(newRule.id, accountId);
            addLog('success', 'attachments', `Found ${count} attachments from existing emails`);
            onRefreshAfterApply();
          } catch (err) {
            addLog('error', 'attachments', `Failed to apply rule retroactively: ${err}`);
          } finally {
            setApplyingRuleId(null);
          }
        }
      }
      resetForm();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setIsSubmitting(false);
    }
  };

  const requestDelete = (ruleId: string) => {
    setPendingDeleteId(ruleId);
  };

  const cancelDelete = () => {
    setPendingDeleteId(null);
    setPendingDeleteCount(null);
  };

  const confirmDelete = async () => {
    if (!pendingDeleteId) return;
    const ruleId = pendingDeleteId;
    setIsDeleting(true);
    try {
      await onDeleteRule(ruleId);
      setPendingDeleteId(null);
      setPendingDeleteCount(null);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'attachments', `Failed to delete rule: ${err}`);
    } finally {
      setIsDeleting(false);
    }
  };

  const handleToggleEnabled = async (rule: AttachmentRule) => {
    try {
      await onUpdateRule(
        rule.id,
        rule.name,
        rule.senderEmailPattern,
        rule.subjectPattern,
        rule.filenamePattern,
        rule.tags,
        !rule.enabled,
      );
    } catch (err) {
      setError(errorText(err));
    }
  };

  const handleApplyRetroactively = async (rule: AttachmentRule) => {
    setApplyingRuleId(rule.id);
    try {
      addLog('info', 'attachments', `Scanning existing emails for rule "${rule.name}"...`);
      const count = await api.applyRuleRetroactively(rule.id, accountId);
      addLog('success', 'attachments', `Found ${count} attachments from existing emails`);
      onRefreshAfterApply();
    } catch (err) {
      addLog('error', 'attachments', `Failed to apply rule retroactively: ${err}`);
    } finally {
      setApplyingRuleId(null);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-white rounded-xl shadow-2xl w-full max-w-2xl max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="px-6 py-4 border-b border-gray-200 flex items-center justify-between">
          <h2 className="text-lg font-semibold text-gray-900">{t('attachments:rules.modalTitle')}</h2>
          <button onClick={onClose} className="p-1 text-gray-400 hover:text-gray-600 rounded-lg hover:bg-gray-100">
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 space-y-4">
          {/* Add/Edit form (placed above the rule list so it's immediately
              visible when creating or editing without forcing the user to
              scroll past the existing rules). */}
          {showForm ? (
            <div className="border border-primary-200 rounded-lg p-4 bg-primary-50/30 space-y-3">
              <h3 className="text-sm font-medium text-gray-900">
                {editingRuleId ? t('attachments:rules.editTitle') : t('attachments:rules.newTitle')}
              </h3>

              <div>
                <label className="block text-xs font-medium text-gray-700 mb-1">
                  {t('attachments:rules.ruleName')}
                </label>
                <input
                  type="text"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                  placeholder={t('attachments:rules.namePlaceholder')}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                />
              </div>

              <div>
                <label className="block text-xs font-medium text-gray-700 mb-1">
                  {t('attachments:rules.senderPattern')}
                </label>
                <input
                  type="text"
                  value={form.senderEmailPattern}
                  onChange={(e) => setForm({ ...form, senderEmailPattern: e.target.value })}
                  placeholder={t('attachments:rules.senderPlaceholder')}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                />
                <p className="text-xs text-gray-400 mt-0.5">{t('attachments:rules.senderPatternHelp')}</p>
              </div>

              <div>
                <label className="block text-xs font-medium text-gray-700 mb-1">
                  {t('attachments:rules.subjectPattern')}
                </label>
                <input
                  type="text"
                  value={form.subjectPattern}
                  onChange={(e) => setForm({ ...form, subjectPattern: e.target.value })}
                  placeholder={t('attachments:rules.subjectPlaceholder')}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                />
                <p className="text-xs text-gray-400 mt-0.5">{t('attachments:rules.subjectPatternHelp')}</p>
              </div>

              <div>
                <label className="block text-xs font-medium text-gray-700 mb-1">
                  {t('attachments:rules.filenamePattern')}
                </label>
                <input
                  type="text"
                  value={form.filenamePattern}
                  onChange={(e) => setForm({ ...form, filenamePattern: e.target.value })}
                  placeholder={t('attachments:rules.filenamePlaceholder')}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                />
                <p className="text-xs text-gray-400 mt-0.5">{t('attachments:rules.filenameHelp')}</p>
              </div>

              <div>
                <label className="block text-xs font-medium text-gray-700 mb-1">{t('attachments:rules.tags')}</label>
                <input
                  type="text"
                  value={form.tags}
                  onChange={(e) => setForm({ ...form, tags: e.target.value })}
                  placeholder={t('attachments:rules.tagsPlaceholder')}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                />
                <p className="text-xs text-gray-400 mt-0.5">{t('attachments:rules.tagsHelp')}</p>
              </div>

              {!editingRuleId && (
                <label className="flex items-center gap-2 text-xs text-gray-700 select-none cursor-pointer">
                  <input
                    type="checkbox"
                    checked={form.applyToExisting}
                    onChange={(e) => setForm({ ...form, applyToExisting: e.target.checked })}
                    className="rounded border-gray-300 text-primary-600 focus:ring-primary-500"
                  />
                  {t('attachments:rules.applyToExistingLabel')}
                </label>
              )}

              {error && <p className="text-xs text-red-600">{error}</p>}

              <div className="flex items-center gap-2 pt-1">
                <button
                  onClick={handleSubmit}
                  disabled={isSubmitting}
                  className="px-4 py-2 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors disabled:opacity-50"
                >
                  {isSubmitting
                    ? t('common:state.saving')
                    : editingRuleId
                      ? t('attachments:rules.updateRule')
                      : t('attachments:rules.createRule')}
                </button>
                <button
                  onClick={resetForm}
                  className="px-4 py-2 text-sm font-medium text-gray-600 hover:text-gray-800 hover:bg-gray-100 rounded-lg transition-colors"
                >
                  {t('common:actions.cancel')}
                </button>
              </div>
            </div>
          ) : (
            <button
              onClick={() => {
                setShowForm(true);
                setEditingRuleId(null);
                setForm(EMPTY_FORM);
                setError(null);
              }}
              className="w-full py-3 border-2 border-dashed border-gray-300 rounded-lg text-sm text-gray-500 hover:text-gray-700 hover:border-gray-400 transition-colors flex items-center justify-center gap-2"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
              </svg>
              {t('attachments:list.createRule')}
            </button>
          )}

          {/* Surface errors raised outside the form (e.g. delete failures) so
              they're visible even when the form isn't open. */}
          {!showForm && error && <p className="text-xs text-red-600 px-1">{error}</p>}

          {/* Existing rules */}
          {rules.length > 0 && (
            <div className="space-y-2">
              {rules.map((rule) => (
                <div
                  key={rule.id}
                  className={`border rounded-lg p-4 ${rule.enabled ? 'border-gray-200' : 'border-gray-100 opacity-60'}`}
                >
                  <div className="flex items-center justify-between mb-2">
                    <div className="flex items-center gap-2">
                      <span className="font-medium text-gray-900">{rule.name}</span>
                      {!rule.enabled && (
                        <span className="text-xs text-gray-400">{t('attachments:rules.disabled')}</span>
                      )}
                    </div>
                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => handleApplyRetroactively(rule)}
                        disabled={applyingRuleId === rule.id}
                        className="p-1.5 text-gray-400 hover:text-primary-600 rounded hover:bg-gray-100 disabled:opacity-50"
                        title={t('attachments:rules.applyToExisting')}
                      >
                        {applyingRuleId === rule.id ? (
                          <div className="w-4 h-4 animate-spin rounded-full border-2 border-primary-600 border-t-transparent" />
                        ) : (
                          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                            />
                          </svg>
                        )}
                      </button>
                      <button
                        onClick={() => handleToggleEnabled(rule)}
                        className="p-1.5 text-gray-400 hover:text-gray-600 rounded hover:bg-gray-100"
                        title={rule.enabled ? t('attachments:rules.disable') : t('attachments:rules.enable')}
                      >
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          {rule.enabled ? (
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
                            />
                          ) : (
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M3 3l18 18"
                            />
                          )}
                        </svg>
                      </button>
                      <button
                        onClick={() => startEditing(rule)}
                        className="p-1.5 text-gray-400 hover:text-gray-600 rounded hover:bg-gray-100"
                        title={t('common:actions.edit')}
                      >
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                          />
                        </svg>
                      </button>
                      <button
                        onClick={() => requestDelete(rule.id)}
                        disabled={pendingDeleteId === rule.id}
                        className="p-1.5 text-gray-400 hover:text-red-600 rounded hover:bg-gray-100 disabled:opacity-40"
                        title={t('common:actions.delete')}
                      >
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                          />
                        </svg>
                      </button>
                    </div>
                  </div>
                  {pendingDeleteId === rule.id && (
                    <div className="mb-2 p-3 rounded-md bg-red-50 border border-red-200 text-xs text-red-800">
                      <p className="font-medium mb-1">{t('attachments:rules.deleteConfirm')}</p>
                      <p className="mb-2">
                        {pendingDeleteCount === null
                          ? t('attachments:rules.deleteCounting')
                          : pendingDeleteCount === 0
                            ? t('attachments:rules.deleteNoFiles')
                            : t('attachments:rules.deleteWithFiles', { count: pendingDeleteCount })}
                      </p>
                      <div className="flex items-center gap-2">
                        <button
                          onClick={confirmDelete}
                          disabled={isDeleting || pendingDeleteCount === null}
                          className="px-3 py-1 text-xs font-medium text-white bg-red-600 hover:bg-red-700 rounded disabled:opacity-50"
                        >
                          {isDeleting
                            ? t('attachments:rules.deleting')
                            : pendingDeleteCount && pendingDeleteCount > 0
                              ? t('attachments:rules.deleteRuleWithFiles', { count: pendingDeleteCount })
                              : t('attachments:rules.deleteRule')}
                        </button>
                        <button
                          onClick={cancelDelete}
                          disabled={isDeleting}
                          className="px-3 py-1 text-xs font-medium text-gray-700 bg-white hover:bg-gray-100 border border-gray-300 rounded disabled:opacity-50"
                        >
                          {t('common:actions.cancel')}
                        </button>
                      </div>
                    </div>
                  )}
                  <div className="text-xs text-gray-500 space-y-0.5">
                    {rule.senderEmailPattern && (
                      <div>
                        {t('attachments:rules.rowSender')}{' '}
                        <code className="bg-gray-100 px-1 py-0.5 rounded">{rule.senderEmailPattern}</code>
                      </div>
                    )}
                    {rule.subjectPattern && (
                      <div>
                        {t('attachments:rules.rowSubject')}{' '}
                        <code className="bg-gray-100 px-1 py-0.5 rounded">{rule.subjectPattern}</code>
                      </div>
                    )}
                    {rule.filenamePattern && (
                      <div>
                        {t('attachments:rules.rowFilename')}{' '}
                        <code className="bg-gray-100 px-1 py-0.5 rounded">{rule.filenamePattern}</code>
                      </div>
                    )}
                    {rule.tags.length > 0 && (
                      <div className="flex items-center gap-1 mt-1">
                        {t('attachments:rules.rowTags')}{' '}
                        {rule.tags.map((tag) => (
                          <span
                            key={tag}
                            className="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-primary-100 text-primary-700"
                          >
                            {tag}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-6 py-3 border-t border-gray-200 bg-gray-50 rounded-b-xl">
          <p className="text-xs text-gray-400">{t('attachments:rules.footerHelp')}</p>
        </div>
      </div>
    </div>
  );
}
