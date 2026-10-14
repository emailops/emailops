import { expect, test } from 'claude-code/testing';

import { mainCheckout, newlyWaiting, parseSummary } from './summary';

test('parses the summary the board prints', () => {
  const s = parseSummary('{"counts": {"todo": 2}, "needs_you": ["T-0003"], "text": "tasks: 2 todo"}');
  expect(s?.text).toBe('tasks: 2 todo');
  expect(s?.needs_you).toEqual(['T-0003']);
});

test('anything else is no summary', () => {
  expect(parseSummary('error: boom')).toBe(null);
  expect(parseSummary('{"other": 1}')).toBe(null);
});

test('only tasks that started waiting since the last poll are news', () => {
  expect(newlyWaiting(['T-0001'], ['T-0001', 'T-0002'])).toEqual(['T-0002']);
  expect(newlyWaiting(['T-0001'], [])).toEqual([]);
});

test('the first poll announces nothing', () => {
  expect(newlyWaiting(null, ['T-0001'])).toEqual([]);
});

test('the main checkout is the parent of the common git dir', () => {
  expect(mainCheckout('/repo/emailops/.git\n')).toBe('/repo/emailops');
  expect(mainCheckout('/repo/bare.git')).toBe(null);
});
