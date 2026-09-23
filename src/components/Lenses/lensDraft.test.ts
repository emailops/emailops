import { describe, expect, it } from 'vitest';

import type { LensTemplate } from '@/types';

import { draftFromTemplate, scopeFromDraft } from './lensDraft';

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
