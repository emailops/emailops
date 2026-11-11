import { describe, expect, it } from 'vitest';
import type { ActiveFilter } from '@/types';
import { appendFilterToken, filterFromQuery, filterToken, planListSource } from './filterQuery';

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

describe('planListSource', () => {
  it('sends any query, written or set by a filter, down one path', () => {
    expect(planListSource('tag:priority=urgent')).toEqual({ kind: 'query', query: 'tag:priority=urgent' });
    expect(planListSource(' invoice ')).toEqual({ kind: 'query', query: 'invoice' });
  });

  it('lists the mailbox when there is no query', () => {
    expect(planListSource(null)).toEqual({ kind: 'mailbox' });
    expect(planListSource('   ')).toEqual({ kind: 'mailbox' });
  });
});
