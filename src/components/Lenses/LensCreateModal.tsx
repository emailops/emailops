// Create Lens modal.
//
// Two tabs: a Templates picker (default) and the Custom form. Picking a
// template prefills the form and switches to it, so every Lens is reviewed
// (account, folders, columns, prompt) before `create_lens` runs.

import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import { Select } from '@/components/shared/Select';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useChatFilledForm, useFormFillStore } from '@/stores/formFillStore';
import { useLensStore } from '@/stores/lensStore';
import { useViewContextStore } from '@/stores/viewContextStore';
import type {
  CreateLensInput,
  Lens,
  LensColumn,
  LensColumnType,
  LensDirection,
  LensPreviewRow,
  LensScope,
  LensTemplate,
} from '@/types';

import { LensFolderChips } from './LensFolderChips';
import { type DraftColumn, draftFromTemplate, scopeFromDraft } from './lensDraft';
import { toLensFormPrefill, toLensFormValues } from './lensPrefill';
import { withoutFolderMailboxes } from './scopeFolders';
import { validateSenderDomains, validateSenderEmails } from './scopeValidation';

interface LensCreateModalProps {
  open: boolean;
  onClose: () => void;
  onCreated: (lens: Lens) => void;
}

const COLUMN_TYPES: LensColumnType[] = [
  'string',
  'text',
  'number',
  'currency',
  'date',
  'boolean',
  'enum',
  'email',
  'url',
];

const MAILBOXES = ['inbox', 'sent', 'archive', 'spam', 'trash'] as const;
const CATEGORIES = ['Primary', 'Promotions', 'Social', 'Updates', 'Forums'] as const;

function newColumn(): DraftColumn {
  return { key: '', label: '', type: 'string', description: '', required: false, isUniqueKey: false, enumValues: '' };
}

export function LensCreateModal({ open, onClose, onCreated }: LensCreateModalProps) {
  const { t } = useTranslation(['common', 'lenses']);
  const accounts = useAccountStore((s) => s.accounts);
  const createLens = useLensStore((s) => s.createLens);

  const [tab, setTab] = useState<'templates' | 'custom'>('templates');

  // Templates tab state
  const [templates, setTemplates] = useState<LensTemplate[]>([]);
  const [templatesLoading, setTemplatesLoading] = useState(false);
  /** Template the form was prefilled from; kept on the created Lens. */
  const [templateKey, setTemplateKey] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    // Every entry point (sidebar "+", header button) starts from the templates.
    setTab('templates');
    setTemplatesLoading(true);
    api
      .listLensTemplates()
      .then(setTemplates)
      .catch((err) => setError(errorText(err)))
      .finally(() => setTemplatesLoading(false));
  }, [open]);

  const [name, setName] = useState('');
  const [icon, setIcon] = useState('');
  const [accountId, setAccountId] = useState<string>(''); // '' = all accounts
  const [mailboxes, setMailboxes] = useState<string[]>(['inbox']);
  const [categories, setCategories] = useState<string[]>(['Primary', 'Updates']);
  const [direction, setDirection] = useState<LensDirection>('inbound');
  const [lastDays, setLastDays] = useState<string>('60');
  const [query, setQuery] = useState('');
  const [querySearchBody, setQuerySearchBody] = useState(false);
  const [senderDomains, setSenderDomains] = useState('');
  const [senderEmails, setSenderEmails] = useState('');
  const [prompt, setPrompt] = useState(() => t('lenses:create.defaultPrompt'));
  const [columns, setColumns] = useState<DraftColumn[]>(() => [
    {
      key: 'summary',
      label: t('lenses:columns.builtin.summary'),
      type: 'text',
      description: t('lenses:create.defaultSummaryDescription'),
      required: true,
      isUniqueKey: false,
      enumValues: '',
    },
  ]);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewRows, setPreviewRows] = useState<LensPreviewRow[] | null>(null);

  // ── Filled from chat ──────────────────────────────────────────────────
  // The chat drops its filled values in `formFillStore`; we apply them here
  // and highlight whatever it could not fill. Everything the model did not
  // touch keeps whatever is already in the form, so "añade una columna de
  // IVA" adds to the user's work instead of replacing it.
  const pendingFill = useChatFilledForm('lens.create');
  const clearFilledForm = useFormFillStore((s) => s.clearFilledForm);
  const [missingFromChat, setMissingFromChat] = useState<string[]>([]);
  const appliedNonce = useRef<number | null>(null);

  useEffect(() => {
    // Gated on `open`: this component stays mounted while closed, and applying
    // (and clearing) a fill before the dialog is on screen would race the
    // parent effect that opens it — child effects flush first.
    if (!open || !pendingFill || appliedNonce.current === pendingFill.nonce) return;
    appliedNonce.current = pendingFill.nonce;
    const prefill = toLensFormPrefill(pendingFill.values);
    // A chat fill is always a custom lens — it defines its own columns, which
    // is exactly what the Templates tab cannot express.
    setTab('custom');
    if (prefill.name !== undefined) setName(prefill.name);
    if (prefill.icon !== undefined) setIcon(prefill.icon);
    if (prefill.mailboxes !== undefined) setMailboxes(prefill.mailboxes);
    if (prefill.categories !== undefined) setCategories(prefill.categories);
    if (prefill.direction !== undefined) setDirection(prefill.direction);
    if (prefill.senderDomains !== undefined) setSenderDomains(prefill.senderDomains);
    if (prefill.query !== undefined) setQuery(prefill.query);
    if (prefill.prompt !== undefined) setPrompt(prefill.prompt);
    if (prefill.columns !== undefined) setColumns(prefill.columns);
    setMissingFromChat(pendingFill.missingRequired);
    setError(null);
    clearFilledForm();
  }, [open, pendingFill, clearFilledForm]);

  // Tell the chat this form is on screen, and what is currently in it, so
  // "añade una columna para el IVA" edits THIS form instead of starting a new
  // one. Cleared on close: the context describes the moment, nothing more.
  const setOpenForm = useViewContextStore((s) => s.setOpenForm);
  useEffect(() => {
    if (!open) {
      setOpenForm(null);
      return;
    }
    setOpenForm({
      token: 'form/lens.create',
      values: toLensFormValues({
        name,
        icon,
        mailboxes,
        categories,
        direction,
        senderDomains,
        query,
        prompt,
        columns,
      }),
    });
    return () => {
      setOpenForm(null);
    };
  }, [open, name, icon, mailboxes, categories, direction, senderDomains, query, prompt, columns, setOpenForm]);

  if (!open) return null;

  // Built-in template names come from the backend in English; localize them by
  // key, falling back to the backend text for a template the locales lack.
  const templateName = (tpl: LensTemplate) => t(`lenses:templates.${tpl.key}.name`, { defaultValue: tpl.name });

  const applyTemplate = (tpl: LensTemplate) => {
    const draft = draftFromTemplate(tpl, (key, fallback) => t(key, { defaultValue: fallback }));
    setName(draft.name);
    setIcon(draft.icon);
    setTemplateKey(draft.templateKey);
    setPrompt(draft.prompt);
    setColumns(draft.columns);
    setAccountId(draft.form.accountId);
    setMailboxes(draft.form.mailboxes);
    setCategories(draft.form.categories);
    setDirection(draft.form.direction);
    setLastDays(draft.form.lastDays);
    setQuery(draft.form.query);
    setQuerySearchBody(draft.form.querySearchBody);
    setSenderDomains(draft.form.senderDomains);
    setSenderEmails(draft.form.senderEmails);
    setPreviewRows(null);
    setError(null);
    setTab('custom');
  };

  const toggleInArray = (list: string[], v: string, setter: (next: string[]) => void) => {
    setter(list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  };

  const updateColumn = (idx: number, patch: Partial<DraftColumn>) => {
    setColumns((prev) => prev.map((c, i) => (i === idx ? { ...c, ...patch } : c)));
  };
  const removeColumn = (idx: number) => {
    setColumns((prev) => prev.filter((_, i) => i !== idx));
  };
  const addColumn = () => setColumns((prev) => [...prev, newColumn()]);

  /** Build LensScope+LensSchema from the form, or set an error and return null. */
  const buildScopeAndSchema = (): { scope: LensScope; schema: { columns: LensColumn[] } } | null => {
    const finalisedColumns: LensColumn[] = [];
    for (const c of columns) {
      const key = c.key.trim();
      if (!key) {
        setError(t('lenses:create.errors.missingKey'));
        return null;
      }
      if (!/^[a-z][a-z0-9_]*$/i.test(key)) {
        setError(t('lenses:create.errors.invalidKey', { key }));
        return null;
      }
      if (finalisedColumns.some((existing) => existing.key === key)) {
        setError(t('lenses:create.errors.duplicateKey', { key }));
        return null;
      }
      const col: LensColumn = {
        key,
        label: c.label.trim() || key,
        type: c.type,
        description: c.description.trim(),
        required: c.required,
        ...(c.isUniqueKey ? { isUniqueKey: true } : {}),
      };
      if (c.type === 'enum') {
        const values = c.enumValues
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean);
        if (values.length === 0) {
          setError(t('lenses:create.errors.enumNeedsValues', { key }));
          return null;
        }
        col.enumValues = values;
      }
      finalisedColumns.push(col);
    }
    const inputError = validateSenderDomains(senderDomains).error ?? validateSenderEmails(senderEmails).error;
    if (inputError) {
      setError(t(`lenses:scope.errors.${inputError.code}`, inputError.params));
      return null;
    }
    const scope = scopeFromDraft({
      accountId,
      mailboxes,
      categories,
      direction,
      lastDays,
      query,
      querySearchBody,
      senderDomains,
      senderEmails,
    });
    return { scope, schema: { columns: finalisedColumns } };
  };

  const handleSubmit = async () => {
    setError(null);
    if (!name.trim()) {
      setError(t('lenses:create.errors.nameRequired'));
      return;
    }
    if (!prompt.trim()) {
      setError(t('lenses:create.errors.promptRequired'));
      return;
    }
    const built = buildScopeAndSchema();
    if (!built) return;
    const input: CreateLensInput = {
      name: name.trim(),
      icon: icon.trim() || null,
      templateKey,
      accountId: accountId || null,
      scope: built.scope,
      schema: built.schema,
      promptText: prompt.trim(),
    };

    setSubmitting(true);
    try {
      const lens = await createLens(input);
      onCreated(lens);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSubmitting(false);
    }
  };

  const handlePreview = async () => {
    setError(null);
    setPreviewRows(null);
    if (!prompt.trim()) {
      setError(t('lenses:create.errors.promptBeforePreview'));
      return;
    }
    const built = buildScopeAndSchema();
    if (!built) return;
    setPreviewing(true);
    try {
      const rows = await api.previewLensExtraction(built.scope, built.schema, prompt.trim(), 3);
      setPreviewRows(rows);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setPreviewing(false);
    }
  };

  return (
    <Modal
      open={open}
      onClose={submitting ? () => undefined : onClose}
      title={t('lenses:create.title')}
      subtitle={t('lenses:create.subtitle')}
      size="2xl"
      disableBackdropClose
      // Filling this form from chat is a conversation: the docked panel has to
      // stay visible and clickable so the user can say "no, ese importe es una
      // moneda" without closing the dialog first.
      nonBlocking
      footer={
        <div className="flex justify-end gap-2 border-t border-gray-700 px-6 py-3">
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="rounded border border-gray-600 px-3 py-1.5 text-xs text-gray-200 hover:bg-gray-700 disabled:opacity-50"
          >
            {t('common:actions.cancel')}
          </button>
          {tab === 'custom' && (
            <button
              type="button"
              onClick={() => void handleSubmit()}
              disabled={submitting}
              className="rounded bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-50"
            >
              {submitting ? t('lenses:create.creating') : t('lenses:create.submit')}
            </button>
          )}
        </div>
      }
    >
      <div className="space-y-5 text-xs text-gray-300">
        {error && <div className="rounded border border-red-700/50 bg-red-900/30 px-3 py-2 text-red-300">{error}</div>}

        {/* Tabs */}
        <div className="flex gap-1 border-b border-gray-700">
          <button
            type="button"
            onClick={() => setTab('templates')}
            className={`px-3 py-1.5 text-xs font-medium ${
              tab === 'templates' ? 'border-b-2 border-blue-500 text-blue-300' : 'text-gray-400 hover:text-gray-200'
            }`}
          >
            {t('lenses:create.tabTemplates')}
          </button>
          <button
            type="button"
            onClick={() => setTab('custom')}
            className={`px-3 py-1.5 text-xs font-medium ${
              tab === 'custom' ? 'border-b-2 border-blue-500 text-blue-300' : 'text-gray-400 hover:text-gray-200'
            }`}
          >
            {t('lenses:create.tabCustom')}
          </button>
        </div>

        {missingFromChat.length > 0 && (
          <div
            data-testid="lens-create-missing-from-chat"
            className="rounded border border-amber-700/50 bg-amber-900/30 px-3 py-2 text-xs text-amber-200"
          >
            {t('lenses:create.filledFromChatMissing', { fields: missingFromChat.join(', ') })}
          </div>
        )}

        {tab === 'templates' && (
          <section className="space-y-3">
            {templatesLoading ? (
              <div className="py-6 text-center text-gray-500">{t('lenses:loadingTemplates')}</div>
            ) : templates.length === 0 ? (
              <div className="py-6 text-center text-gray-500">{t('lenses:noTemplates')}</div>
            ) : (
              <div className="grid grid-cols-2 gap-2">
                {templates.map((tpl) => (
                  <button
                    key={tpl.key}
                    type="button"
                    onClick={() => applyTemplate(tpl)}
                    className="group flex items-start gap-3 rounded border border-gray-700 bg-[#1e1e1e]/60 p-3 text-left transition-colors hover:border-blue-500/60 hover:bg-blue-900/10 disabled:opacity-50"
                  >
                    <span className="text-xl leading-none">{tpl.icon}</span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium text-gray-100">{templateName(tpl)}</span>
                      <span className="mt-0.5 block text-[11px] leading-snug text-gray-400">
                        {t(`lenses:templates.${tpl.key}.description`, { defaultValue: tpl.description })}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            )}
          </section>
        )}

        {tab === 'custom' && (
          <>
            {/* Identity */}
            <section className="space-y-2">
              <h3 className="text-[11px] uppercase tracking-wider text-gray-500">{t('lenses:identity.title')}</h3>
              <div className="grid grid-cols-2 gap-3">
                <label className="block">
                  <span className="mb-1 block text-gray-400">{t('lenses:identity.name')}</span>
                  <input
                    type="text"
                    data-testid="lens-create-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={t('lenses:create.namePlaceholder')}
                    className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                  />
                </label>
                <label className="block">
                  <span className="mb-1 block text-gray-400">{t('lenses:identity.icon')}</span>
                  <input
                    type="text"
                    value={icon}
                    onChange={(e) => setIcon(e.target.value)}
                    placeholder="🧾"
                    maxLength={4}
                    className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                  />
                </label>
              </div>
            </section>

            {/* Scope */}
            <section className="space-y-2">
              <h3 className="text-[11px] uppercase tracking-wider text-gray-500">{t('lenses:scope.title')}</h3>
              <div className="grid grid-cols-2 gap-3">
                <label className="block">
                  <span className="mb-1 block text-gray-400">{t('lenses:scope.account')}</span>
                  <Select
                    value={accountId}
                    options={[
                      { value: '', label: t('lenses:scope.allAccounts') },
                      ...accounts.map((a) => ({ value: a.id, label: a.email })),
                    ]}
                    onChange={(value) => {
                      setAccountId(value);
                      setMailboxes((prev) => withoutFolderMailboxes(prev));
                    }}
                    ariaLabel={t('lenses:scope.account')}
                    fullWidth
                  />
                </label>
                <label className="block">
                  <span className="mb-1 block text-gray-400">{t('lenses:scope.direction')}</span>
                  <Select
                    value={direction}
                    options={[
                      { value: 'either', label: t('lenses:scope.either') },
                      { value: 'inbound', label: t('lenses:scope.inboundOnly') },
                      { value: 'outbound', label: t('lenses:scope.outboundOnly') },
                    ]}
                    onChange={(value) => setDirection(value as LensDirection)}
                    ariaLabel={t('lenses:scope.direction')}
                    fullWidth
                  />
                </label>
              </div>

              <div className="space-y-1">
                <span className="block text-gray-400">{t('lenses:scope.mailboxes')}</span>
                <div className="flex flex-wrap gap-1.5">
                  {MAILBOXES.map((m) => (
                    <button
                      key={m}
                      type="button"
                      onClick={() => toggleInArray(mailboxes, m, setMailboxes)}
                      className={`rounded border px-2 py-0.5 text-[11px] ${
                        mailboxes.includes(m)
                          ? 'border-blue-500 bg-blue-600/30 text-blue-200'
                          : 'border-gray-600 text-gray-300 hover:bg-gray-700'
                      }`}
                    >
                      {t(`lenses:scope.mailboxNames.${m}`)}
                    </button>
                  ))}
                </div>
              </div>

              <LensFolderChips
                accountId={accountId}
                selected={mailboxes}
                onToggle={(v) => toggleInArray(mailboxes, v, setMailboxes)}
              />

              <div className="space-y-1">
                <span className="block text-gray-400">{t('lenses:scope.categories')}</span>
                <div className="flex flex-wrap gap-1.5">
                  {CATEGORIES.map((c) => (
                    <button
                      key={c}
                      type="button"
                      onClick={() => toggleInArray(categories, c, setCategories)}
                      className={`rounded border px-2 py-0.5 text-[11px] ${
                        categories.includes(c)
                          ? 'border-blue-500 bg-blue-600/30 text-blue-200'
                          : 'border-gray-600 text-gray-300 hover:bg-gray-700'
                      }`}
                    >
                      {t(`lenses:scope.categoryNames.${c}`)}
                    </button>
                  ))}
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <label className="block">
                  <span className="mb-1 block text-gray-400">{t('lenses:scope.lastNDays')}</span>
                  <input
                    type="number"
                    min={1}
                    value={lastDays}
                    onChange={(e) => setLastDays(e.target.value)}
                    className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                  />
                </label>
                <label className="block">
                  <span className="mb-1 block text-gray-400">{t('lenses:scope.senderDomainsCsv')}</span>
                  <input
                    type="text"
                    value={senderDomains}
                    onChange={(e) => setSenderDomains(e.target.value)}
                    placeholder="stripe.com, wise.com" // i18n-ignore: example sender domains
                    className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                  />
                </label>
              </div>

              <label className="block">
                <span className="mb-1 block text-gray-400">{t('lenses:scope.senderEmails')}</span>
                <input
                  type="text"
                  value={senderEmails}
                  onChange={(e) => setSenderEmails(e.target.value)}
                  placeholder="billing@stripe.com, invoices@vendor.com" // i18n-ignore: example sender emails
                  className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                />
              </label>

              <label className="block">
                <span className="mb-1 block text-gray-400">{t('lenses:scope.keywordQuery')}</span>
                <input
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={t('lenses:scope.keywordPlaceholder')}
                  className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                />
              </label>
              <label className="flex items-center gap-2 text-[11px] text-gray-300">
                <input
                  type="checkbox"
                  checked={querySearchBody}
                  onChange={(e) => setQuerySearchBody(e.target.checked)}
                  className="h-3 w-3 accent-blue-500"
                />
                {t('lenses:scope.searchBody')}
                <span className="text-gray-500">{t('lenses:scope.keywordsBodyHint')}</span>
              </label>
            </section>

            {/* Schema */}
            <section className="space-y-2">
              <div className="flex items-center justify-between">
                <h3 className="text-[11px] uppercase tracking-wider text-gray-500">{t('lenses:columns.title')}</h3>
                <button
                  type="button"
                  onClick={addColumn}
                  className="rounded border border-gray-600 px-2 py-0.5 text-[11px] text-gray-200 hover:bg-gray-700"
                >
                  {t('lenses:columns.add')}
                </button>
              </div>
              <div className="space-y-2">
                {columns.map((c, idx) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: column rows have no stable id during creation; reorder/remove would still re-render correctly because inputs are uncontrolled
                  <div key={idx} className="rounded border border-gray-700 bg-[#1e1e1e]/60 p-3">
                    <div className="grid grid-cols-12 gap-2">
                      <label className="col-span-3 block">
                        <span className="mb-1 block text-[10px] uppercase text-gray-500">
                          {t('lenses:columns.key')}
                        </span>
                        <input
                          type="text"
                          data-testid="lens-create-column-key"
                          value={c.key}
                          onChange={(e) => updateColumn(idx, { key: e.target.value })}
                          placeholder="amount" // i18n-ignore: example column key (technical identifier)
                          className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                        />
                      </label>
                      <label className="col-span-3 block">
                        <span className="mb-1 block text-[10px] uppercase text-gray-500">
                          {t('lenses:columns.label')}
                        </span>
                        <input
                          type="text"
                          value={c.label}
                          onChange={(e) => updateColumn(idx, { label: e.target.value })}
                          placeholder={t('lenses:columns.builtin.amount')}
                          className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                        />
                      </label>
                      <label className="col-span-3 block">
                        <span className="mb-1 block text-[10px] uppercase text-gray-500">
                          {t('lenses:columns.type')}
                        </span>
                        <Select
                          value={c.type}
                          options={COLUMN_TYPES.map((colType) => ({
                            value: colType,
                            label: t(`lenses:columns.types.${colType}`),
                          }))}
                          onChange={(value) => updateColumn(idx, { type: value as LensColumnType })}
                          ariaLabel={t('lenses:columns.type')}
                          size="xs"
                          fullWidth
                        />
                      </label>
                      <div className="col-span-2 flex items-end gap-3">
                        <label className="flex items-center gap-1 text-[11px] text-gray-300">
                          <input
                            type="checkbox"
                            checked={c.required}
                            onChange={(e) => updateColumn(idx, { required: e.target.checked })}
                          />
                          {t('lenses:columns.required')}
                        </label>
                        <label
                          className="flex items-center gap-1 text-[11px] text-gray-300"
                          title={t('lenses:create.uniqueKeyTooltip')}
                        >
                          <input
                            type="checkbox"
                            checked={c.isUniqueKey}
                            onChange={(e) => {
                              // Only one column can be unique key at a time.
                              if (e.target.checked) {
                                setColumns((cols) => cols.map((col, i) => ({ ...col, isUniqueKey: i === idx })));
                              } else {
                                updateColumn(idx, { isUniqueKey: false });
                              }
                            }}
                          />
                          {t('lenses:create.uniqueKey')}
                        </label>
                      </div>
                      <div className="col-span-1 flex items-end justify-end">
                        <button
                          type="button"
                          onClick={() => removeColumn(idx)}
                          className="rounded p-1 text-gray-500 hover:bg-gray-700 hover:text-red-300"
                          title={t('lenses:create.removeColumn')}
                          aria-label={t('lenses:create.removeColumn')}
                        >
                          ✕
                        </button>
                      </div>
                    </div>
                    <label className="mt-2 block">
                      <span className="mb-1 block text-[10px] uppercase text-gray-500">
                        {t('lenses:columns.description')}
                      </span>
                      <input
                        type="text"
                        value={c.description}
                        onChange={(e) => updateColumn(idx, { description: e.target.value })}
                        placeholder={t('lenses:create.descriptionPlaceholder')}
                        className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                      />
                    </label>
                    {c.type === 'enum' && (
                      <label className="mt-2 block">
                        <span className="mb-1 block text-[10px] uppercase text-gray-500">
                          {t('lenses:columns.enumValues')}
                        </span>
                        <input
                          type="text"
                          value={c.enumValues}
                          onChange={(e) => updateColumn(idx, { enumValues: e.target.value })}
                          placeholder="paid, unpaid, refunded" // i18n-ignore: example enum values
                          className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                        />
                      </label>
                    )}
                  </div>
                ))}
              </div>
            </section>

            {/* Prompt */}
            <section className="space-y-2">
              <h3 className="text-[11px] uppercase tracking-wider text-gray-500">
                {t('lenses:columns.extractionPrompt')}
              </h3>
              <textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={5}
                className="w-full resize-y rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
              />
              <p className="text-[11px] text-gray-500">{t('lenses:create.promptHelp')}</p>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void handlePreview()}
                  disabled={previewing || submitting}
                  className="rounded border border-gray-600 px-2 py-1 text-[11px] text-gray-200 hover:bg-gray-700 disabled:opacity-50"
                >
                  {previewing ? t('lenses:create.testing') : t('lenses:create.testOnSample')}
                </button>
                <span className="text-[11px] text-gray-500">{t('lenses:create.previewHelp')}</span>
              </div>
              {previewRows && previewRows.length > 0 && (
                <div className="mt-2 space-y-2">
                  <h4 className="text-[11px] uppercase tracking-wider text-gray-500">
                    {t('lenses:create.previewTitle', { count: previewRows.length })}
                  </h4>
                  {previewRows.map((r) => (
                    <div key={r.emailId} className="rounded border border-gray-700 bg-[#1e1e1e]/60 p-2 text-[11px]">
                      <div className="truncate font-medium text-gray-200">
                        {r.emailSubject || t('lenses:create.noSubject')}
                      </div>
                      <div className="truncate text-gray-500">{r.emailSender}</div>
                      {r.errorMessage ? (
                        <div className="mt-1 text-red-400">{r.errorMessage}</div>
                      ) : (
                        <pre className="mt-1 whitespace-pre-wrap break-words font-mono text-[10px] text-gray-300">
                          {JSON.stringify(r.data, null, 2)}
                        </pre>
                      )}
                    </div>
                  ))}
                </div>
              )}
              {previewRows && previewRows.length === 0 && (
                <div className="mt-2 text-[11px] text-gray-500">{t('lenses:create.previewEmpty')}</div>
              )}
            </section>
          </>
        )}
      </div>
    </Modal>
  );
}
