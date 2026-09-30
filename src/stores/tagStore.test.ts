import { describe, expect, it } from 'vitest';
import type { EmailTag } from '@/types';
import { mergeClassifiedTags } from './tagStore';

function tag(tagType: string, tagValue: string): EmailTag {
  return { emailId: 'e1', tagType, tagValue, confidence: null, createdAt: 0 };
}

describe('mergeClassifiedTags', () => {
  const classified = { priority: 'high', intent: 'request', topic: 'billing', confidence: 0.9 };

  it('keeps tags the classifier does not produce (junk, company)', () => {
    const merged = mergeClassifiedTags([tag('junk', 'spam'), tag('company', 'Acme')], 'e1', classified, 5);
    expect(merged.map((t) => `${t.tagType}:${t.tagValue}`).sort()).toEqual([
      'company:Acme',
      'intent:request',
      'junk:spam',
      'priority:high',
      'topic:billing',
    ]);
  });

  it('replaces the previous priority, intent and topic', () => {
    const merged = mergeClassifiedTags(
      [tag('priority', 'low'), tag('intent', 'fyi'), tag('topic', 'travel')],
      'e1',
      classified,
      5,
    );
    expect(merged).toEqual([
      { emailId: 'e1', tagType: 'priority', tagValue: 'high', confidence: 0.9, createdAt: 5 },
      { emailId: 'e1', tagType: 'intent', tagValue: 'request', confidence: 0.9, createdAt: 5 },
      { emailId: 'e1', tagType: 'topic', tagValue: 'billing', confidence: 0.9, createdAt: 5 },
    ]);
  });
});
