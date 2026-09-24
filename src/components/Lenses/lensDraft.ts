// Create-form state for a Lens, and the two conversions around it: a built-in
// template prefilling the form, and the form turning back into a LensScope.
// The form is the only path to a new Lens, so every scope field a template
// can carry must survive the round trip.

import type { LensColumn, LensColumnType, LensDirection, LensScope, LensTemplate } from '@/types';

import { validateSenderDomains, validateSenderEmails } from './scopeValidation';

export interface DraftColumn {
  key: string;
  label: string;
  type: LensColumnType;
  description: string;
  required: boolean;
  isUniqueKey: boolean;
  enumValues: string; // comma-separated; parsed on submit
}

/** Scope fields as the form edits them ('' / [] = no filter). */
export interface ScopeForm {
  accountId: string;
  mailboxes: string[];
  categories: string[];
  direction: LensDirection;
  lastDays: string;
  query: string;
  querySearchBody: boolean;
  senderDomains: string;
  senderEmails: string;
}

export interface LensDraft {
  name: string;
  icon: string;
  templateKey: string;
  prompt: string;
  columns: DraftColumn[];
  form: ScopeForm;
}

/** `localize(i18nKey, fallback)` — returns the fallback when no translation exists. */
export function draftFromTemplate(tpl: LensTemplate, localize: (key: string, fallback: string) => string): LensDraft {
  const s = tpl.defaultScope;
  return {
    name: localize(`lenses:templates.${tpl.key}.name`, tpl.name),
    icon: tpl.icon,
    templateKey: tpl.key,
    prompt: tpl.prompt,
    columns: draftColumnsFromSchema(tpl.schema.columns, localize),
    form: {
      accountId: s.accountIds?.length === 1 ? s.accountIds[0] : '',
      mailboxes: s.mailboxes ?? [],
      categories: s.categories ?? [],
      direction: s.direction ?? 'either',
      lastDays: s.dateRange?.lastDays != null ? String(s.dateRange.lastDays) : '',
      query: s.query ?? '',
      querySearchBody: s.querySearchBody ?? false,
      senderDomains: (s.senderDomains ?? []).join(', '),
      senderEmails: (s.senderEmails ?? []).join(', '),
    },
  };
}

/** Callers validate the sender lists first; invalid entries are passed through
 *  here and rejected by that validation, not silently dropped. */
export function scopeFromDraft(form: ScopeForm): LensScope {
  const days = form.lastDays.trim() ? Number.parseInt(form.lastDays.trim(), 10) : Number.NaN;
  const domains = validateSenderDomains(form.senderDomains).values;
  const emails = validateSenderEmails(form.senderEmails).values;
  const scope: LensScope = {
    accountIds: form.accountId ? [form.accountId] : null,
    mailboxes: form.mailboxes.length ? form.mailboxes : null,
    categories: form.categories.length ? form.categories : null,
    direction: form.direction === 'either' ? null : form.direction,
    query: form.query.trim() || null,
    senderDomains: domains.length ? domains : null,
    senderEmails: emails.length ? emails : null,
    dateRange: Number.isFinite(days) && days > 0 ? { lastDays: days } : null,
  };
  // Only sent when true — the backend defaults to subject-only search.
  if (form.querySearchBody) scope.querySearchBody = true;
  return scope;
}

/** Stored columns as editor rows (labels localized where a translation exists). */
export function draftColumnsFromSchema(
  columns: LensColumn[],
  localize: (key: string, fallback: string) => string,
): DraftColumn[] {
  return columns.map((c) => ({
    key: c.key,
    label: localize(`lenses:columns.builtin.${c.key}`, c.label),
    type: c.type,
    description: c.description,
    required: c.required,
    isUniqueKey: c.isUniqueKey ?? false,
    enumValues: (c.enumValues ?? []).join(', '),
  }));
}

/** Why a set of editor rows is not a valid schema; `code` names a
 *  `lenses:create.errors.*` message. */
export interface ColumnsError {
  code: 'missingKey' | 'invalidKey' | 'duplicateKey' | 'enumNeedsValues';
  params: { key?: string };
}

/** Editor rows as a schema, or the first reason they are not one. */
export function schemaFromDraftColumns(
  drafts: DraftColumn[],
): { ok: true; columns: LensColumn[] } | { ok: false; error: ColumnsError } {
  const columns: LensColumn[] = [];
  for (const d of drafts) {
    const key = d.key.trim();
    if (!key) return { ok: false, error: { code: 'missingKey', params: {} } };
    if (!/^[a-z][a-z0-9_]*$/i.test(key)) return { ok: false, error: { code: 'invalidKey', params: { key } } };
    if (columns.some((c) => c.key === key)) return { ok: false, error: { code: 'duplicateKey', params: { key } } };
    const col: LensColumn = {
      key,
      label: d.label.trim() || key,
      type: d.type,
      description: d.description.trim(),
      required: d.required,
      ...(d.isUniqueKey ? { isUniqueKey: true } : {}),
    };
    if (d.type === 'enum') {
      const values = d.enumValues
        .split(',')
        .map((v) => v.trim())
        .filter(Boolean);
      if (values.length === 0) return { ok: false, error: { code: 'enumNeedsValues', params: { key } } };
      col.enumValues = values;
    }
    columns.push(col);
  }
  return { ok: true, columns };
}
