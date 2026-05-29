import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { ClassificationRule } from '@/types';

export interface ClassificationRulePrefill {
  senderEmail: string;
  subject: string;
  senderName: string;
}

interface ClassificationRulesTabProps {
  activeAccountId: string | null;
  /** Pool of intent values shown in the dropdown (configured + defaults). */
  intents: string[];
  /** Pool of topic values shown in the dropdown (configured + defaults). */
  topics: string[];
  /** Optional pre-fill for opening the rule form when launched from a context menu. */
  prefill?: ClassificationRulePrefill | null;
  /** Initial open-state of the rule form (true when launched from a prefill action). */
  startWithFormOpen?: boolean;
  onSuccess: (msg: string) => void;
  onError: (msg: string) => void;
}

/**
 * Manage classification rules: list + create/edit/delete form.
 * Rules match emails by sender/subject pattern and assign tags instantly
 * without invoking the AI.
 */
export function ClassificationRulesTab({
  activeAccountId,
  intents,
  topics,
  prefill,
  startWithFormOpen = false,
  onSuccess,
  onError,
}: ClassificationRulesTabProps) {
  const { t } = useTranslation(['common', 'settings']);
  const [rules, setRules] = useState<ClassificationRule[]>([]);

  const [editingRule, setEditingRule] = useState<ClassificationRule | null>(null);
  const [ruleName, setRuleName] = useState('');
  const [ruleSender, setRuleSender] = useState('');
  const [ruleSubject, setRuleSubject] = useState('');
  const [rulePriority, setRulePriority] = useState('low');
  const [ruleIntent, setRuleIntent] = useState('notification');
  const [ruleTopic, setRuleTopic] = useState('operations');
  const [showRuleForm, setShowRuleForm] = useState(startWithFormOpen);

  const loadRules = async () => {
    if (!activeAccountId) return;
    try {
      const r = await api.listClassificationRules(activeAccountId);
      setRules(r);
    } catch (err) {
      console.error('Failed to load rules:', err);
    }
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: load on mount + when account changes
  useEffect(() => {
    if (activeAccountId) {
      void loadRules();
    }
  }, [activeAccountId]);

  // Apply prefill when it changes
  useEffect(() => {
    if (prefill) {
      const domain = prefill.senderEmail.split('@')[1] || '';
      setRuleName(`${prefill.senderName || domain}`);
      setRuleSender(`*@${domain}`);
      setRuleSubject('');
      setRulePriority('low');
      setRuleIntent('notification');
      setRuleTopic('operations');
      setShowRuleForm(true);
    }
  }, [prefill]);

  const resetRuleForm = () => {
    setEditingRule(null);
    setRuleName('');
    setRuleSender('');
    setRuleSubject('');
    setRulePriority('low');
    setRuleIntent('notification');
    setRuleTopic('operations');
    setShowRuleForm(false);
  };

  const editRule = (rule: ClassificationRule) => {
    setEditingRule(rule);
    setRuleName(rule.name);
    setRuleSender(rule.senderPattern || '');
    setRuleSubject(rule.subjectPattern || '');
    setRulePriority(rule.priority);
    setRuleIntent(rule.intent);
    setRuleTopic(rule.topic);
    setShowRuleForm(true);
  };

  const handleSaveRule = async () => {
    if (!activeAccountId || !ruleName.trim()) return;
    if (!ruleSender.trim() && !ruleSubject.trim()) {
      onError(t('settings:classification.patternRequired'));
      return;
    }
    try {
      if (editingRule) {
        await api.updateClassificationRule({
          ...editingRule,
          name: ruleName.trim(),
          senderPattern: ruleSender.trim() || null,
          subjectPattern: ruleSubject.trim() || null,
          priority: rulePriority,
          intent: ruleIntent,
          topic: ruleTopic,
          updatedAt: Math.floor(Date.now() / 1000),
        });
      } else {
        await api.createClassificationRule(
          activeAccountId,
          ruleName.trim(),
          ruleSender.trim() || null,
          ruleSubject.trim() || null,
          rulePriority,
          ruleIntent,
          ruleTopic,
        );
      }
      resetRuleForm();
      await loadRules();
      onSuccess(editingRule ? t('settings:classification.ruleUpdated') : t('settings:classification.ruleCreated'));
    } catch (err) {
      onError(t('settings:classification.saveRuleFailed', { error: errorText(err) }));
    }
  };

  const handleDeleteRule = async (rule: ClassificationRule) => {
    if (!activeAccountId) return;
    try {
      await api.deleteClassificationRule(rule.id, activeAccountId);
      await loadRules();
    } catch (err) {
      onError(t('settings:classification.deleteRuleFailed', { error: errorText(err) }));
    }
  };

  const handleToggleRule = async (rule: ClassificationRule) => {
    if (!activeAccountId) return;
    try {
      await api.updateClassificationRule({
        ...rule,
        enabled: !rule.enabled,
        updatedAt: Math.floor(Date.now() / 1000),
      });
      await loadRules();
    } catch (err) {
      onError(t('settings:classification.toggleRuleFailed', { error: errorText(err) }));
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-xs text-gray-500">
          {t('settings:classification.rulesIntro')} <code className="text-gray-400">*</code>
          {/* i18n-ignore */} {t('settings:classification.rulesIntroSuffix')}
        </p>
        {!showRuleForm && (
          <button
            onClick={() => {
              resetRuleForm();
              setShowRuleForm(true);
            }}
            className="px-3 py-1.5 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 flex-shrink-0"
          >
            {t('settings:classification.addRuleButton')}
          </button>
        )}
      </div>

      {showRuleForm && (
        <div className="bg-[#1e1e1e] border border-gray-600 rounded-lg p-4 space-y-3">
          <input
            value={ruleName}
            onChange={(e) => setRuleName(e.target.value)}
            placeholder={t('settings:classification.ruleNamePlaceholder')}
            className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
          />

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs text-gray-400 mb-1">{t('settings:classification.senderPattern')}</label>
              <input
                value={ruleSender}
                onChange={(e) => setRuleSender(e.target.value)}
                placeholder={t('settings:classification.senderPatternPlaceholder')}
                className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
              />
            </div>
            <div>
              <label className="block text-xs text-gray-400 mb-1">{t('settings:classification.subjectPattern')}</label>
              <input
                value={ruleSubject}
                onChange={(e) => setRuleSubject(e.target.value)}
                placeholder={t('settings:classification.subjectPatternPlaceholder')}
                className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
              />
            </div>
          </div>

          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="block text-xs text-gray-400 mb-1">{t('settings:classification.priority')}</label>
              <select
                value={rulePriority}
                onChange={(e) => setRulePriority(e.target.value)}
                className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm outline-none"
              >
                <option value="urgent">{t('settings:classification.priorityUrgent')}</option>
                <option value="normal">{t('settings:classification.priorityNormal')}</option>
                <option value="low">{t('settings:classification.priorityLow')}</option>
              </select>
            </div>
            <div>
              <label className="block text-xs text-gray-400 mb-1">{t('settings:classification.intent')}</label>
              <select
                value={ruleIntent}
                onChange={(e) => setRuleIntent(e.target.value)}
                className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm outline-none"
              >
                {intents.map((i) => (
                  <option key={i} value={i}>
                    {i}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="block text-xs text-gray-400 mb-1">{t('settings:classification.topic')}</label>
              <select
                value={ruleTopic}
                onChange={(e) => setRuleTopic(e.target.value)}
                className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm outline-none"
              >
                {topics.map((topic) => (
                  <option key={topic} value={topic}>
                    {topic}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="flex gap-2">
            <button
              onClick={handleSaveRule}
              className="px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500"
            >
              {editingRule ? t('settings:classification.updateRule') : t('settings:classification.createRule')}
            </button>
            <button
              onClick={resetRuleForm}
              className="px-4 py-2 bg-gray-700 text-gray-300 rounded text-sm hover:bg-gray-600"
            >
              {t('common:actions.cancel')}
            </button>
          </div>
        </div>
      )}

      <div className="space-y-1">
        {rules.map((rule) => (
          <div
            key={rule.id}
            className={`flex items-center gap-3 px-3 py-2 rounded-lg border border-gray-700 ${
              rule.enabled ? 'bg-[#1e1e1e]' : 'bg-[#1e1e1e] opacity-50'
            }`}
          >
            <button
              onClick={() => handleToggleRule(rule)}
              className={`w-4 h-4 rounded flex-shrink-0 border ${rule.enabled ? 'bg-primary-600 border-primary-600' : 'border-gray-500'}`}
              title={rule.enabled ? t('settings:classification.disableRule') : t('settings:classification.enableRule')}
            >
              {rule.enabled && (
                <svg className="w-4 h-4 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={3} d="M5 13l4 4L19 7" />
                </svg>
              )}
            </button>
            <div className="flex-1 min-w-0">
              <div className="text-sm text-gray-200 font-medium">{rule.name}</div>
              <div className="text-xs text-gray-500 flex gap-3">
                {rule.senderPattern && (
                  <span>
                    {t('settings:classification.ruleSender')}{' '}
                    <code className="text-gray-400">{rule.senderPattern}</code>
                  </span>
                )}
                {rule.subjectPattern && (
                  <span>
                    {t('settings:classification.ruleSubject')}{' '}
                    <code className="text-gray-400">{rule.subjectPattern}</code>
                  </span>
                )}
              </div>
            </div>
            <div className="flex gap-1 flex-shrink-0">
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-red-900/30 text-red-300">{rule.priority}</span>
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-900/30 text-blue-300">{rule.intent}</span>
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-amber-900/30 text-amber-300">{rule.topic}</span>
            </div>
            <div className="flex gap-1 flex-shrink-0">
              <button
                onClick={() => editRule(rule)}
                className="p-1 text-gray-400 hover:text-gray-200"
                title={t('settings:classification.editRule')}
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                  />
                </svg>
              </button>
              <button
                onClick={() => handleDeleteRule(rule)}
                className="p-1 text-gray-400 hover:text-red-400"
                title={t('settings:classification.deleteRule')}
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
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
        ))}
        {rules.length === 0 && (
          <p className="text-sm text-gray-500 text-center py-4">{t('settings:classification.noRulesYet')}</p>
        )}
      </div>
    </div>
  );
}

/** Convenience export so callers exporting just the rule count don't need to know about the tab. */
export function getRuleCount(rules: ClassificationRule[]): number {
  return rules.length;
}
