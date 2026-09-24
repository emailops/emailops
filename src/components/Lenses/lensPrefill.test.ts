import { describe, expect, it } from 'vitest';

import { accountIdForEmail, toLensFormPrefill, toLensFormValues } from './lensPrefill';

describe('toLensFormPrefill', () => {
  it('maps the scalar fields the model filled', () => {
    const p = toLensFormPrefill({ name: 'Facturas', icon: '🧾', promptText: 'Extrae los datos.' });
    expect(p.name).toBe('Facturas');
    expect(p.icon).toBe('🧾');
    expect(p.prompt).toBe('Extrae los datos.');
  });

  it('leaves a field the model did not fill out of the prefill entirely', () => {
    // The modal spreads the prefill onto existing state, so an absent key must
    // stay absent — writing `undefined` would wipe what the user already typed.
    const p = toLensFormPrefill({ name: 'Facturas' });
    expect('icon' in p).toBe(false);
    expect('columns' in p).toBe(false);
  });

  it('flattens the scope keys into the form pieces that hold them', () => {
    const p = toLensFormPrefill({
      scopeMailboxes: ['inbox', 'sent'],
      scopeCategories: ['Primary'],
      scopeDirection: 'outbound',
      scopeSenderDomains: ['stripe.com', 'fly.io'],
      scopeQuery: 'invoice',
    });
    expect(p.mailboxes).toEqual(['inbox', 'sent']);
    expect(p.categories).toEqual(['Primary']);
    expect(p.direction).toBe('outbound');
    expect(p.senderDomains).toBe('stripe.com, fly.io');
    expect(p.query).toBe('invoice');
  });

  it('builds draft columns with the modal comma-separated enum convention', () => {
    const p = toLensFormPrefill({
      columns: [
        {
          key: 'status',
          label: 'Estado',
          type: 'enum',
          description: 'Estado actual',
          enumValues: ['new', 'interviewing'],
          required: true,
          isUniqueKey: false,
        },
      ],
    });
    expect(p.columns).toHaveLength(1);
    expect(p.columns?.[0]).toMatchObject({
      key: 'status',
      label: 'Estado',
      type: 'enum',
      description: 'Estado actual',
      required: true,
      enumValues: 'new, interviewing',
    });
  });

  it('falls back to string for a column type this build does not know', () => {
    const p = toLensFormPrefill({ columns: [{ key: 'a', label: 'A', type: 'quaternion' }] });
    expect(p.columns?.[0].type).toBe('string');
  });

  it('falls back to string when the model omitted the column type', () => {
    const p = toLensFormPrefill({ columns: [{ key: 'a', label: 'A' }] });
    expect(p.columns?.[0].type).toBe('string');
  });

  it('drops a column with no key or no label rather than adding an unusable row', () => {
    const p = toLensFormPrefill({
      columns: [{ key: 'a', label: 'A' }, { label: 'orphan' }, { key: 'b' }],
    });
    expect(p.columns).toHaveLength(1);
    expect(p.columns?.[0].key).toBe('a');
  });

  it('omits columns entirely when every row was unusable', () => {
    const p = toLensFormPrefill({ columns: [{ label: 'orphan' }] });
    expect('columns' in p).toBe(false);
  });

  it('ignores a direction outside the declared set', () => {
    expect('direction' in toLensFormPrefill({ scopeDirection: 'sideways' })).toBe(false);
  });

  it('ignores an empty or whitespace-only string', () => {
    const p = toLensFormPrefill({ name: '   ', scopeQuery: '' });
    expect('name' in p).toBe(false);
    expect('query' in p).toBe(false);
  });

  it('ignores a list that is not a list', () => {
    const p = toLensFormPrefill({ scopeMailboxes: 'inbox', columns: 'nope' });
    expect('mailboxes' in p).toBe(false);
    expect('columns' in p).toBe(false);
  });

  it('survives a completely empty fill', () => {
    expect(toLensFormPrefill({})).toEqual({});
  });

  it('treats a non-boolean required flag as false rather than truthy', () => {
    const p = toLensFormPrefill({ columns: [{ key: 'a', label: 'A', required: 'yes' }] });
    expect(p.columns?.[0].required).toBe(false);
  });
});

describe('toLensFormValues', () => {
  it('round-trips a filled form back into the backend field keys', () => {
    const original = {
      name: 'Facturas',
      scopeMailboxes: ['inbox'],
      scopeDirection: 'inbound',
      scopeSenderDomains: ['stripe.com'],
      promptText: 'Extrae los datos.',
      columns: [{ key: 'amount', label: 'Importe', type: 'currency', required: true }],
    };
    const roundTripped = toLensFormValues(toLensFormPrefill(original));
    expect(roundTripped).toMatchObject(original);
  });

  it('omits empty fields instead of sending blanks the model would read as choices', () => {
    const values = toLensFormValues({ name: '', senderDomains: '   ', columns: [] });
    expect(values).toEqual({});
  });

  it('splits the comma-separated domain input back into a list', () => {
    expect(toLensFormValues({ senderDomains: 'stripe.com, fly.io ' }).scopeSenderDomains).toEqual([
      'stripe.com',
      'fly.io',
    ]);
  });

  it('drops a column the user has not named yet', () => {
    const values = toLensFormValues({
      columns: [
        { key: '', label: '', type: 'string', description: '', required: false, isUniqueKey: false, enumValues: '' },
        { key: 'a', label: 'A', type: 'string', description: '', required: false, isUniqueKey: false, enumValues: '' },
      ],
    });
    expect(values.columns).toHaveLength(1);
  });

  it('omits the false flags rather than sending them', () => {
    const values = toLensFormValues({
      columns: [
        { key: 'a', label: 'A', type: 'string', description: '', required: false, isUniqueKey: false, enumValues: '' },
      ],
    });
    expect(values.columns).toEqual([{ key: 'a', label: 'A', type: 'string' }]);
  });
});

describe('account', () => {
  it('reads the account the request named', () => {
    expect(toLensFormPrefill({ scopeAccount: ' Owner@Studio.example ' }).accountEmail).toBe('Owner@Studio.example');
  });

  it('sends the account on screen back as the form value', () => {
    expect(toLensFormValues({ accountEmail: 'owner@studio.example' }).scopeAccount).toBe('owner@studio.example');
    expect(toLensFormValues({}).scopeAccount).toBeUndefined();
  });
});

describe('accountIdForEmail', () => {
  const accounts = [
    { id: 'a1', email: 'owner@studio.example' },
    { id: 'a2', email: 'personal@mail.example' },
  ];

  it('matches an account by address, ignoring case', () => {
    expect(accountIdForEmail(accounts, 'OWNER@studio.example')).toBe('a1');
  });

  it('finds nothing for an address that is not one of the accounts', () => {
    expect(accountIdForEmail(accounts, 'someone@else.example')).toBeNull();
  });
});
