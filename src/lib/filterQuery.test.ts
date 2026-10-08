import { describe, expect, it } from 'vitest';
import type { ActiveFilter } from '@/types';
import { appendFilterToken, filterFromQuery, filterMailboxScope, filterToken, planListSource } from './filterQuery';

describe('filterToken', () => {
  it('writes each smart filter as the search operator that lists the same mail', () => {
    const cases: [ActiveFilter, string][] = [
      [{ type: 'priority', value: 'urgent' }, 'tag:priority=urgent'],
      [{ type: 'intent', value: 'request' }, 'tag:intent=request'],
      [{ type: 'topic', value: 'billing' }, 'tag:topic=billing'],
      [{ type: 'company', value: 'globex' }, 'tag:company=globex'],
      [{ type: 'sender', value: 'ana@vendor.example' }, 'from:ana@vendor.example'],
      [{ type: 'domain', value: 'vendor.example' }, 'domain:vendor.example'],
      [{ type: 'attachment_ext', value: 'pdf' }, 'ext:pdf'],
    ];
    for (const [filter, token] of cases) {
      expect(filterToken(filter)).toBe(token);
    }
  });

  it('quotes a value with spaces so the query keeps it whole', () => {
    expect(filterToken({ type: 'company', value: 'acme corp' })).toBe('tag:"company=acme corp"');
  });
});

describe('filterFromQuery', () => {
  it('reads back every token filterToken writes', () => {
    const filters: ActiveFilter[] = [
      { type: 'priority', value: 'urgent' },
      { type: 'company', value: 'acme corp' },
      { type: 'sender', value: 'ana@vendor.example' },
      { type: 'domain', value: 'vendor.example' },
      { type: 'attachment_ext', value: 'pdf' },
    ];
    for (const filter of filters) {
      expect(filterFromQuery(filterToken(filter))).toEqual(filter);
    }
  });

  it('ignores surrounding whitespace', () => {
    expect(filterFromQuery('  tag:priority=urgent ')).toEqual({ type: 'priority', value: 'urgent' });
  });

  it('is null for anything that is not exactly one smart-filter token', () => {
    for (const query of [
      '',
      'invoice',
      'tag:priority=urgent invoice',
      'tag:priority=urgent tag:topic=billing',
      'tag:urgent', // a bare tag spans every type: no single smart filter lists it
      'tag:unknown=x',
      'subject:invoice',
      'from:ana', // the sender filter is a whole address
    ]) {
      expect(filterFromQuery(query), query).toBeNull();
    }
  });
});

describe('appendFilterToken', () => {
  it('adds the token after the current query', () => {
    expect(appendFilterToken('invoice', { type: 'priority', value: 'urgent' })).toBe('invoice tag:priority=urgent');
  });

  it('is the token alone when there is no query', () => {
    expect(appendFilterToken(null, { type: 'topic', value: 'billing' })).toBe('tag:topic=billing');
    expect(appendFilterToken('  ', { type: 'topic', value: 'billing' })).toBe('tag:topic=billing');
  });

  it('does not repeat a token the query already has', () => {
    expect(appendFilterToken('tag:priority=urgent invoice', { type: 'priority', value: 'urgent' })).toBe(
      'tag:priority=urgent invoice',
    );
  });
});

describe('filterMailboxScope', () => {
  it('narrows to the view for Sent, Archive and custom folders', () => {
    expect(filterMailboxScope('sent')).toBe('sent');
    expect(filterMailboxScope('archive')).toBe('archive');
    expect(filterMailboxScope('folder:Clients')).toBe('folder:Clients');
  });

  it('keeps every live mailbox for the inbox and views that are not a mailbox', () => {
    // The inbox's filters reach archived and sent mail (DECISIONS 2026-10-02).
    for (const view of ['inbox', 'starred', 'snoozed', 'spam', 'deleted'] as const) {
      expect(filterMailboxScope(view), view).toBeUndefined();
    }
  });
});

describe('planListSource', () => {
  it('lists a one-filter query through the paged smart-filter path, inside the view', () => {
    expect(planListSource('tag:priority=urgent', 'sent')).toEqual({
      kind: 'filter',
      filter: { type: 'priority', value: 'urgent' },
      mailbox: 'sent',
    });
  });

  it('lists any other query through search, inside the view', () => {
    expect(planListSource('tag:priority=urgent invoice', 'folder:Clients')).toEqual({
      kind: 'search',
      query: 'tag:priority=urgent invoice',
      mailbox: 'folder:Clients',
    });
    expect(planListSource('invoice', 'inbox')).toEqual({ kind: 'search', query: 'invoice', mailbox: undefined });
  });

  it('lists the mailbox when there is no query', () => {
    expect(planListSource(null, 'archive')).toEqual({ kind: 'mailbox' });
    expect(planListSource('   ', 'inbox')).toEqual({ kind: 'mailbox' });
  });
});
