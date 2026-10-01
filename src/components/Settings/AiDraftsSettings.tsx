import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { SettingsPanel } from './SettingsPanel';

/** Mirror of `DEFAULT_PROMPT_TEMPLATE` in `services/emails/drafts.rs`. Keep
 *  these two in sync — the textarea hint and the "Reset to default" button
 *  both depend on it. The template uses `{persona}`, `{style}`,
 *  `{thread_context}`, `{rag_context}`, and `{instructions}` placeholders;
 *  unknown placeholders are left as-is so a user typo doesn't crash the
 *  backend. */
const DEFAULT_PROMPT_TEMPLATE = `You are an email assistant for {persona}.
Writing style: {style}
Language: Match the language of the original email.

{thread_context}
{rag_context}
{instructions}Write the reply (body only, no subject line, ending as instructed above):`;

const DEFAULT_PERSONA = 'a freelance CTO and technical consultant';
const DEFAULT_STYLE = 'concise, friendly but professional, uses short paragraphs, avoids corporate jargon';

/** How an AI draft ends. Mirror of `DraftSignoff` in `services/emails/drafts.rs`. */
type Signoff = 'signature' | 'name' | 'none';
const SIGNOFFS: Signoff[] = ['signature', 'name', 'none'];

function parseSignoff(value: string | null): Signoff {
  return value === 'name' || value === 'none' ? value : 'signature';
}

export function AiDraftsSettings() {
  const { t } = useTranslation(['common', 'settings']);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const [enabled, setEnabled] = useState(true);
  const [persona, setPersona] = useState(DEFAULT_PERSONA);
  const [style, setStyle] = useState(DEFAULT_STYLE);
  const [promptTemplate, setPromptTemplate] = useState(DEFAULT_PROMPT_TEMPLATE);
  const [signoff, setSignoff] = useState<Signoff>('signature');
  const [signoffName, setSignoffName] = useState('');

  useEffect(() => {
    void (async () => {
      try {
        const [en, pers, sty, tpl, so, soName] = await Promise.all([
          api.getPref('ai_drafts_enabled'),
          api.getPref('draft_persona'),
          api.getPref('draft_style'),
          api.getPref('draft_prompt_template'),
          api.getPref('draft_signoff'),
          api.getPref('draft_signoff_name'),
        ]);
        setEnabled(en !== 'false');
        if (pers) setPersona(pers);
        if (sty) setStyle(sty);
        if (tpl) setPromptTemplate(tpl);
        setSignoff(parseSignoff(so));
        if (soName) setSignoffName(soName);
      } catch (e) {
        setError(errorText(e));
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      await Promise.all([
        api.setPref('ai_drafts_enabled', enabled ? 'true' : 'false'),
        api.setPref('draft_persona', persona.trim() || DEFAULT_PERSONA),
        api.setPref('draft_style', style.trim() || DEFAULT_STYLE),
        api.setPref('draft_prompt_template', promptTemplate.trim() ? promptTemplate : DEFAULT_PROMPT_TEMPLATE),
        // "Name" without a name would sign with nothing: save it as the default instead.
        api.setPref('draft_signoff', signoff === 'name' && !signoffName.trim() ? 'signature' : signoff),
        api.setPref('draft_signoff_name', signoffName.trim()),
      ]);
      setSuccess(t('settings:aiDrafts.saved'));
      setTimeout(() => setSuccess(null), 2000);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const handleResetTemplate = () => {
    setPromptTemplate(DEFAULT_PROMPT_TEMPLATE);
  };

  if (loading) {
    return <p className="text-gray-400 text-sm p-4">{t('common:state.loading')}</p>;
  }

  return (
    <SettingsPanel
      footer={
        <div className="px-6 py-4 border-t border-gray-700 flex justify-end flex-shrink-0">
          <button
            onClick={() => void handleSave()}
            disabled={saving}
            className="px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50"
          >
            {saving ? t('common:state.saving') : t('common:actions.save')}
          </button>
        </div>
      }
    >
      {error && <div className="p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>}
      {success && (
        <div className="p-3 bg-green-900/30 border border-green-800 rounded text-green-300 text-sm">{success}</div>
      )}

      <section>
        <div className="flex items-center justify-between py-2">
          <div>
            <label className="block text-sm font-medium text-gray-300">{t('settings:aiDrafts.enable')}</label>
            <p className="text-xs text-gray-500 mt-0.5">{t('settings:aiDrafts.enableDesc')}</p>
          </div>
          <button
            type="button"
            onClick={() => setEnabled((v) => !v)}
            className={`relative inline-flex h-6 w-11 flex-shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
              enabled ? 'bg-primary-600' : 'bg-gray-600'
            }`}
            role="switch"
            aria-checked={enabled}
          >
            <span
              className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                enabled ? 'translate-x-5' : 'translate-x-0'
              }`}
            />
          </button>
        </div>
      </section>

      <section>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:aiDrafts.persona')}</label>
        <p className="text-xs text-gray-500 mb-2">
          {t('settings:aiDrafts.personaHelpStart')} <code>{'{persona}'}</code> {t('settings:aiDrafts.personaHelpEnd')}
        </p>
        <input
          type="text"
          value={persona}
          onChange={(e) => setPersona(e.target.value)}
          disabled={!enabled}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none disabled:opacity-50"
          placeholder={DEFAULT_PERSONA}
        />
      </section>

      <section>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:aiDrafts.writingStyle')}</label>
        <p className="text-xs text-gray-500 mb-2">
          {t('settings:aiDrafts.writingStyleHelpStart')} <code>{'{style}'}</code>
          {t('settings:aiDrafts.writingStyleHelpEnd')}
        </p>
        <textarea
          value={style}
          onChange={(e) => setStyle(e.target.value)}
          disabled={!enabled}
          rows={3}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none disabled:opacity-50"
          placeholder={DEFAULT_STYLE}
        />
      </section>

      <section>
        <span className="block text-sm font-medium text-gray-300 mb-1">{t('settings:aiDrafts.signoff')}</span>
        <p className="text-xs text-gray-500 mb-2">{t('settings:aiDrafts.signoffHelp')}</p>
        <div role="radiogroup" aria-label={t('settings:aiDrafts.signoff')} className="space-y-2">
          {SIGNOFFS.map((option) => (
            <label key={option} className="flex items-start gap-2 text-sm text-gray-300 cursor-pointer">
              <input
                type="radio"
                name="draft-signoff"
                value={option}
                checked={signoff === option}
                onChange={() => setSignoff(option)}
                disabled={!enabled}
                className="mt-0.5"
              />
              <span>
                {t(`settings:aiDrafts.signoffOptions.${option}` as const)}
                <span className="block text-xs text-gray-500">
                  {t(`settings:aiDrafts.signoffOptions.${option}Desc` as const)}
                </span>
              </span>
            </label>
          ))}
        </div>
        {signoff === 'name' && (
          <input
            type="text"
            value={signoffName}
            onChange={(e) => setSignoffName(e.target.value)}
            disabled={!enabled}
            maxLength={80}
            aria-label={t('settings:aiDrafts.signoffName')}
            placeholder={t('settings:aiDrafts.signoffNamePlaceholder')}
            className="mt-2 w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none disabled:opacity-50"
          />
        )}
      </section>

      <section>
        <div className="flex items-center justify-between mb-1">
          <label className="block text-sm font-medium text-gray-300">{t('settings:aiDrafts.promptTemplate')}</label>
          <button
            type="button"
            onClick={handleResetTemplate}
            disabled={!enabled}
            className="text-xs text-primary-400 hover:text-primary-300 disabled:opacity-50"
          >
            {t('settings:aiDrafts.resetToDefault')}
          </button>
        </div>
        <p className="text-xs text-gray-500 mb-2">
          {t('settings:aiDrafts.promptHelpStart')} <code className="text-gray-400">{'{persona}'}</code>,{' '}
          <code className="text-gray-400">{'{style}'}</code>,{' '}
          <code className="text-gray-400">{'{thread_context}'}</code>,{' '}
          <code className="text-gray-400">{'{rag_context}'}</code>,{' '}
          <code className="text-gray-400">{'{instructions}'}</code>
          {t('settings:aiDrafts.promptHelpEnd')}
        </p>
        <textarea
          value={promptTemplate}
          onChange={(e) => setPromptTemplate(e.target.value)}
          disabled={!enabled}
          rows={14}
          spellCheck={false}
          className="w-full bg-[#1e1e1e] text-gray-100 font-mono border border-gray-600 rounded px-3 py-2 text-xs leading-relaxed focus:border-primary-500 outline-none disabled:opacity-50 resize-none"
        />
      </section>
    </SettingsPanel>
  );
}
