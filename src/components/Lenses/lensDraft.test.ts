import { describe, expect, it } from 'vitest';

import type { LensTemplate } from '@/types';

import { draftColumnsFromSchema, draftFromTemplate, schemaFromDraftColumns, scopeFromDraft } from './lensDraft';

const template: LensTemplate = {
  key: 'contact_form_leads',
  name: 'Contact form leads',
  icon: '📨',
  description: 'People who wrote in through your website contact form.',
  defaultScope: {
    direction: 'inbound',
    dateRange: { lastDays: 365 },
    query: 'contact form OR formulario de contacto',
    querySearchBody: true,
    senderDomains: ['forms.example'],
    senderEmails: ['noreply@forms.example'],
  },
  schema: {
    columns: [
      {
        key: 'contact_email',
        label: 'Email',
        type: 'email',
        description: 'Submitter address.',
        required: true,
        isUniqueKey: true,
      },
      {
        key: 'request_type',
        label: 'Type',
        type: 'enum',
        description: 'Kind of message.',
        required: true,
        enumValues: ['quote_request', 'question'],
      },
    ],
  },
  prompt: 'Extract the submitter.',
};

// Stand-in for i18next: localizes the template name and one builtin column.
const localize = (key: string, fallback: string) =>
  ({
    'lenses:templates.contact_form_leads.name': 'Contactos del formulario web',
    'lenses:columns.builtin.request_type': 'Tipo',
  })[key] ?? fallback;

describe('draftFromTemplate', () => {
  it('prefills the form with the template, localized where a translation exists', () => {
    const draft = draftFromTemplate(template, localize);
    expect(draft.name).toBe('Contactos del formulario web');
    expect(draft.icon).toBe('📨');
    expect(draft.templateKey).toBe('contact_form_leads');
    expect(draft.prompt).toBe('Extract the submitter.');
    expect(draft.columns).toEqual([
      {
        key: 'contact_email',
        label: 'Email',
        type: 'email',
        description: 'Submitter address.',
        required: true,
        isUniqueKey: true,
        enumValues: '',
      },
      {
        key: 'request_type',
        label: 'Tipo',
        type: 'enum',
        description: 'Kind of message.',
        required: true,
        isUniqueKey: false,
        enumValues: 'quote_request, question',
      },
    ]);
  });

  it('shows an unset mailbox or category filter as nothing selected, which means all', () => {
    const draft = draftFromTemplate(template, localize);
    expect(draft.form.mailboxes).toEqual([]);
    expect(draft.form.categories).toEqual([]);
    expect(draft.form.accountId).toBe('');
  });
});

describe('scopeFromDraft', () => {
  it('round-trips a template scope through the form without losing a filter', () => {
    // Body search and sender emails had no field in the create form, so a
    // template routed through it would silently lose them.
    const { form } = draftFromTemplate(template, localize);
    expect(scopeFromDraft(form)).toEqual({
      accountIds: null,
      mailboxes: null,
      categories: null,
      direction: 'inbound',
      query: 'contact form OR formulario de contacto',
      querySearchBody: true,
      senderDomains: ['forms.example'],
      senderEmails: ['noreply@forms.example'],
      dateRange: { lastDays: 365 },
    });
  });

  it('omits body search when it is off, since the backend defaults to subject only', () => {
    const { form } = draftFromTemplate({ ...template, defaultScope: { direction: 'outbound' } }, localize);
    const scope = scopeFromDraft(form);
    expect(scope.querySearchBody).toBeUndefined();
    expect(scope.direction).toBe('outbound');
    expect(scope.dateRange).toBeNull();
  });
});

describe('draftColumnsFromSchema / schemaFromDraftColumns', () => {
  const identity = (_key: string, fallback: string) => fallback;

  it('round-trips a stored schema through the editor rows', () => {
    const drafts = draftColumnsFromSchema(template.schema.columns, identity);
    expect(schemaFromDraftColumns(drafts)).toEqual({
      ok: true,
      columns: template.schema.columns.map((c) => ({
        key: c.key,
        label: c.label,
        type: c.type,
        description: c.description,
        required: c.required,
        ...(c.isUniqueKey ? { isUniqueKey: true } : {}),
        ...(c.enumValues ? { enumValues: c.enumValues } : {}),
      })),
    });
  });

  const row = (over: Partial<ReturnType<typeof draftColumnsFromSchema>[number]>) => ({
    key: 'amount',
    label: 'Amount',
    type: 'number' as const,
    description: '',
    required: false,
    isUniqueKey: false,
    enumValues: '',
    ...over,
  });

  it('rejects a row without a key', () => {
    expect(schemaFromDraftColumns([row({ key: ' ' })])).toEqual({
      ok: false,
      error: { code: 'missingKey', params: {} },
    });
  });

  it('rejects a key that is not an identifier', () => {
    expect(schemaFromDraftColumns([row({ key: '1 total' })])).toEqual({
      ok: false,
      error: { code: 'invalidKey', params: { key: '1 total' } },
    });
  });

  it('rejects a repeated key', () => {
    expect(schemaFromDraftColumns([row({}), row({})])).toEqual({
      ok: false,
      error: { code: 'duplicateKey', params: { key: 'amount' } },
    });
  });

  it('rejects a list column with no values', () => {
    expect(schemaFromDraftColumns([row({ type: 'enum' })])).toEqual({
      ok: false,
      error: { code: 'enumNeedsValues', params: { key: 'amount' } },
    });
  });

  it('falls back to the key when the label is blank', () => {
    const result = schemaFromDraftColumns([row({ label: '  ' })]);
    expect(result.ok && result.columns[0].label).toBe('amount');
  });
});
