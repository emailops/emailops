import { describe, expect, it } from 'vitest';
import { applySuggestion, matchSkills, slashCandidates, slashQuery } from './slashSkills';

const skill = (name: string, enabled = true) => ({ name, description: `${name} description`, path: '', enabled });

describe('slashCandidates', () => {
  it('adds the bundled skills, without the ones a skill of the user replaces', () => {
    const candidates = slashCandidates({
      enabled: true,
      dir: null,
      skills: [skill('inbox-triage')],
      bundled: [
        { name: 'inbox-triage', description: 'bundled', enabled: true, shadowed: true },
        { name: 'follow-up-check', description: 'bundled', enabled: true, shadowed: false },
        { name: 'other', description: 'bundled', enabled: false, shadowed: false },
      ],
      errors: [],
    });
    expect(matchSkills(candidates, '').map((s) => s.name)).toEqual(['follow-up-check', 'inbox-triage']);
    expect(candidates.find((s) => s.name === 'inbox-triage')?.description).toBe('inbox-triage description');
  });
});

describe('slashQuery', () => {
  it('reads the skill name being typed at the start of the message', () => {
    expect(slashQuery('/')).toBe('');
    expect(slashQuery('/wee')).toBe('wee');
    expect(slashQuery('  /Wee')).toBe('wee');
  });

  it('stops once the name is finished or the message is not an invocation', () => {
    expect(slashQuery('/weekly-report ')).toBeNull();
    expect(slashQuery('/weekly-report last week')).toBeNull();
    expect(slashQuery('what about /wee')).toBeNull();
    expect(slashQuery('hello')).toBeNull();
    expect(slashQuery('')).toBeNull();
  });
});

describe('matchSkills', () => {
  const skills = [skill('trip-brief'), skill('weekly-report'), skill('weekly-digest', false), skill('vendor-weekly')];

  it('lists enabled skills, prefix matches first', () => {
    expect(matchSkills(skills, 'week').map((s) => s.name)).toEqual(['weekly-report', 'vendor-weekly']);
  });

  it('lists every enabled skill for a bare slash, capped', () => {
    const many = Array.from({ length: 10 }, (_, i) => skill(`s-${i}`));
    expect(matchSkills(many, '')).toHaveLength(6);
    expect(matchSkills(skills, '').map((s) => s.name)).toEqual(['trip-brief', 'vendor-weekly', 'weekly-report']);
  });
});

describe('applySuggestion', () => {
  it('completes the invocation and leaves room for the request', () => {
    expect(applySuggestion('weekly-report')).toBe('/weekly-report ');
  });
});
