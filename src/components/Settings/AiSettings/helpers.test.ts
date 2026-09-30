import { describe, expect, it } from 'vitest';
import { contextBudgetFromPref, contextBudgetToPref, DEFAULT_CONTEXT_BUDGET, MIN_CONTEXT_BUDGET } from './helpers';

describe('contextBudgetFromPref', () => {
  it('falls back to the default when the preference is unset or not a budget', () => {
    for (const raw of [null, '', '0', 'lots', '-5', '512']) {
      expect(contextBudgetFromPref(raw)).toBe(DEFAULT_CONTEXT_BUDGET);
    }
  });

  it('reads a stored budget', () => {
    expect(contextBudgetFromPref('65536')).toBe(65536);
    expect(contextBudgetFromPref(' 16384 ')).toBe(16384);
  });
});

describe('contextBudgetToPref', () => {
  it('rounds and keeps the value the backend accepts', () => {
    expect(contextBudgetToPref(65536.4)).toBe('65536');
    expect(contextBudgetToPref(100)).toBe(String(MIN_CONTEXT_BUDGET));
    expect(contextBudgetToPref(Number.NaN)).toBe(String(DEFAULT_CONTEXT_BUDGET));
  });
});
