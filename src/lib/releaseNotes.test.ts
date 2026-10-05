import { describe, expect, it } from 'vitest';
import { SUPPORTED_LANGUAGES } from '@/i18n';
import packageJson from '../../package.json';
import {
  bundleFromModules,
  planReleaseNotes,
  RELEASE_NOTES,
  type ReleaseNotesBundle,
  releaseNoteMarkdown,
} from './releaseNotes';

const bundle: ReleaseNotesBundle = {
  '0.6.10': { en: 'ten en', es: 'ten es' },
  '0.6.11': { en: 'eleven en', es: 'eleven es' },
  '0.6.12': { en: 'twelve en', es: 'twelve es' },
  '0.7.0': { en: 'seventy en' },
};

describe('planReleaseNotes', () => {
  it('shows the current version notes once after an upgrade', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.6.11', current: '0.6.12' });
    expect(plan).toEqual({ versions: ['0.6.12'], markSeen: true });
  });

  it('shows every skipped version, newest first', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.6.10', current: '0.6.12' });
    expect(plan.versions).toEqual(['0.6.12', '0.6.11']);
  });

  it('never shows notes for versions newer than the running build', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.6.11', current: '0.6.12' });
    expect(plan.versions).not.toContain('0.7.0');
  });

  it('compares versions numerically, not lexicographically', () => {
    const plan = planReleaseNotes({
      bundle: { '0.6.9': { en: 'nine' }, '0.6.10': { en: 'ten' } },
      freshInstall: false,
      lastSeen: '0.6.9',
      current: '0.6.10',
    });
    expect(plan.versions).toEqual(['0.6.10']);
  });

  it('shows only the current version when upgrading from a build that never recorded one', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: null, current: '0.6.12' });
    expect(plan.versions).toEqual(['0.6.12']);
  });

  it('shows nothing and changes nothing on a normal launch of the same version', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.6.12', current: '0.6.12' });
    expect(plan).toEqual({ versions: [], markSeen: false });
  });

  it('records the version silently on a fresh install', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: true, lastSeen: null, current: '0.6.12' });
    expect(plan).toEqual({ versions: [], markSeen: true });
  });

  it('records the version silently after a downgrade so a later upgrade announces again', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.7.0', current: '0.6.12' });
    expect(plan).toEqual({ versions: [], markSeen: true });
  });

  it('records the version silently when the upgrade ships no notes', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.6.12', current: '0.6.13' });
    expect(plan).toEqual({ versions: [], markSeen: true });
  });

  it('treats an unparseable stored version like a missing one', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: 'garbage', current: '0.6.12' });
    expect(plan.versions).toEqual(['0.6.12']);
  });

  it('shows nothing when the running version is unparseable', () => {
    const plan = planReleaseNotes({ bundle, freshInstall: false, lastSeen: '0.6.11', current: 'dev' });
    expect(plan).toEqual({ versions: [], markSeen: false });
  });
});

describe('releaseNoteMarkdown', () => {
  it('returns the notes in the requested language', () => {
    expect(releaseNoteMarkdown(bundle, '0.6.12', 'es')).toBe('twelve es');
  });

  it('falls back to English when the language is missing', () => {
    expect(releaseNoteMarkdown(bundle, '0.7.0', 'de')).toBe('seventy en');
  });
});

describe('bundleFromModules', () => {
  it('groups raw markdown modules by version folder and language file', () => {
    const result = bundleFromModules({
      '/src/releaseNotes/0.7.0/en.md': 'hello',
      '/src/releaseNotes/0.7.0/es.md': 'hola',
      '/src/releaseNotes/README.md': 'ignored',
      '/src/releaseNotes/0.7.0/xx.md': 'unsupported language',
    });
    expect(result).toEqual({ '0.7.0': { en: 'hello', es: 'hola' } });
  });
});

describe('bundled release notes', () => {
  // Fails on a version bump until the release ships its notes in every UI
  // language — the dialog would otherwise skip the release silently.
  it.each(SUPPORTED_LANGUAGES)('ships notes for the current version (%s)', (language) => {
    const markdown = RELEASE_NOTES[packageJson.version]?.[language];
    expect(markdown?.trim()).toBeTruthy();
  });
});
