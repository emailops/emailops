// Create Lens modal.
//
// Two tabs: a Templates picker (default — pick from the 8 built-ins) and a
// Custom form (define scope + schema + prompt by hand). Both routes converge
// on backend commands `create_lens_from_template` / `create_lens`.

import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import { Select } from '@/components/shared/Select';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLensStore } from '@/stores/lensStore';
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

import { validateSenderDomains } from './scopeValidation';

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

interface DraftColumn {
  key: string;
  label: string;
  type: LensColumnType;
  description: string;
  required: boolean;
  isUniqueKey: boolean;
  enumValues: string; // comma-separated; parsed on submit
}

function newColumn(): DraftColumn {
  return { key: '', label: '', type: 'string', description: '', required: false, isUniqueKey: false, enumValues: '' };
}

export function LensCreateModal({ open, onClose, onCreated }: LensCreateModalProps) {
  const { t } = useTranslation(['common', 'lenses']);
  const accounts = useAccountStore((s) => s.accounts);
  const createLens = useLensStore((s) => s.createLens);
  const refreshLenses = useLensStore((s) => s.refreshLenses);

  const [tab, setTab] = useState<'templates' | 'custom'>('templates');

  // Templates tab state
  const [templates, setTemplates] = useState<LensTemplate[]>([]);
  const [templatesLoading, setTemplatesLoading] = useState(false);
  const [templateAccountId, setTemplateAccountId] = useState<string>('');

  useEffect(() => {
    if (!open) return;
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
  const [senderDomains, setSenderDomains] = useState('');
  const [prompt, setPrompt] = useState(
    'Extract the fields below from this email. Leave nullable fields as null when the email does not contain the information.',
  );
  const [columns, setColumns] = useState<DraftColumn[]>([
    {
      key: 'summary',
      label: 'Summary',
      type: 'text',
      description: 'One-sentence summary of the email.',
      required: true,
      isUniqueKey: false,
      enumValues: '',
    },
  ]);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewRows, setPreviewRows] = useState<LensPreviewRow[] | null>(null);

  if (!open) return null;

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
        setError('Every column needs a key.');
        return null;
      }
      if (!/^[a-z][a-z0-9_]*$/i.test(key)) {
        setError(`Column key "${key}" must be alphanumeric/underscore and start with a letter.`);
        return null;
      }
      if (finalisedColumns.some((existing) => existing.key === key)) {
        setError(`Duplicate column key "${key}".`);
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
          setError(`Enum column "${key}" needs at least one value.`);
          return null;
        }
        col.enumValues = values;
      }
      finalisedColumns.push(col);
    }
    const domainCheck = validateSenderDomains(senderDomains);
    if (domainCheck.error) {
      setError(domainCheck.error);
      return null;
    }
    const scope: LensScope = {
      accountIds: accountId ? [accountId] : null,
      mailboxes: mailboxes.length ? mailboxes : null,
      categories: categories.length ? categories : null,
      direction: direction === 'either' ? null : direction,
      query: query.trim() || null,
      senderDomains: domainCheck.values.length ? domainCheck.values : null,
      dateRange: lastDays.trim() ? { lastDays: Number.parseInt(lastDays, 10) || null } : null,
    };
    return { scope, schema: { columns: finalisedColumns } };
  };

  const handleSubmit = async () => {
    setError(null);
    if (!name.trim()) {
      setError('Name is required.');
      return;
    }
    if (!prompt.trim()) {
      setError('Prompt is required.');
      return;
    }
    const built = buildScopeAndSchema();
    if (!built) return;
    const input: CreateLensInput = {
      name: name.trim(),
      icon: icon.trim() || null,
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
      setError('Add a prompt before previewing.');
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

  const handleCreateFromTemplate = async (tpl: LensTemplate) => {
    setError(null);
    setSubmitting(true);
    try {
      const lens = await api.createLensFromTemplate(tpl.key, undefined, templateAccountId || undefined);
      await refreshLenses();
      onCreated(lens);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      open={open}
      onClose={submitting ? () => undefined : onClose}
      title={t('lenses:create.title')}
      subtitle="Define a filter, a schema, and an extraction prompt."
      size="2xl"
      disableBackdropClose
      footer={
        <div className="flex justify-end gap-2 border-t border-gray-700 px-6 py-3">
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="rounded border border-gray-600 px-3 py-1.5 text-xs text-gray-200 hover:bg-gray-700 disabled:opacity-50"
          >
            Cancel
          </button>
          {tab === 'custom' && (
            <button
              type="button"
              onClick={() => void handleSubmit()}
              disabled={submitting}
              className="rounded bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-50"
            >
              {submitting ? 'Creating…' : 'Create Lens'}
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
            Templates
          </button>
          <button
            type="button"
            onClick={() => setTab('custom')}
            className={`px-3 py-1.5 text-xs font-medium ${
              tab === 'custom' ? 'border-b-2 border-blue-500 text-blue-300' : 'text-gray-400 hover:text-gray-200'
            }`}
          >
            Custom
          </button>
        </div>

        {tab === 'templates' && (
          <section className="space-y-3">
            <label className="block max-w-xs">
              <span className="mb-1 block text-gray-400">{t('lenses:apply.applyToAccount')}</span>
              <Select
                value={templateAccountId}
                options={[
                  { value: '', label: t('lenses:apply.allAccounts') },
                  ...accounts.map((a) => ({ value: a.id, label: a.email })),
                ]}
                onChange={(value) => setTemplateAccountId(value)}
                ariaLabel={t('lenses:apply.applyToAccount')}
                fullWidth
              />
            </label>
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
                    disabled={submitting}
                    onClick={() => void handleCreateFromTemplate(tpl)}
                    className="group flex items-start gap-3 rounded border border-gray-700 bg-[#1e1e1e]/60 p-3 text-left transition-colors hover:border-blue-500/60 hover:bg-blue-900/10 disabled:opacity-50"
                  >
                    <span className="text-xl leading-none">{tpl.icon}</span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium text-gray-100">{tpl.name}</span>
                      <span className="mt-0.5 block text-[11px] leading-snug text-gray-400">{tpl.description}</span>
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
                    onChange={(value) => setAccountId(value)}
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
                      {m}
                    </button>
                  ))}
                </div>
              </div>

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
                      {c}
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
                <span className="mb-1 block text-gray-400">{t('lenses:scope.keywordQuery')}</span>
                <input
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder='e.g. "invoice" OR "receipt"' // i18n-ignore: FTS5 query syntax sample
                  className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1.5 text-gray-100 focus:border-blue-500 focus:outline-none"
                />
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
                  + Add column
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
                          options={COLUMN_TYPES.map((colType) => ({ value: colType, label: colType }))}
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
                          required
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
                          Enum values (comma-separated)
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
                  {previewing ? 'Testing…' : 'Test on 3 emails'}
                </button>
                <span className="text-[11px] text-gray-500">{t('lenses:create.previewHelp')}</span>
              </div>
              {previewRows && previewRows.length > 0 && (
                <div className="mt-2 space-y-2">
                  <h4 className="text-[11px] uppercase tracking-wider text-gray-500">Preview ({previewRows.length})</h4>
                  {previewRows.map((r) => (
                    <div key={r.emailId} className="rounded border border-gray-700 bg-[#1e1e1e]/60 p-2 text-[11px]">
                      <div className="truncate font-medium text-gray-200">{r.emailSubject || '(no subject)'}</div>
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
