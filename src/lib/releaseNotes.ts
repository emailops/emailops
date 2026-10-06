import { isSupportedLanguage, type Language } from '@/i18n';

/**
 * Release notes shown once after an update, bundled with the app as
 * `src/releaseNotes/<version>/<lang>.md` (one file per UI language). The
 * release skill writes them next to the CHANGELOG section.
 */
export type ReleaseNotesBundle = Record<string, Partial<Record<Language, string>>>;

/** Preference holding the last version whose notes were shown (or skipped). */
export const PREF_RELEASE_NOTES_SEEN_VERSION = 'release_notes_seen_version';

/** Group `import.meta.glob` raw modules by version folder and language file. */
export function bundleFromModules(modules: Record<string, string>): ReleaseNotesBundle {
  const bundle: ReleaseNotesBundle = {};
  for (const [path, markdown] of Object.entries(modules)) {
    const match = /\/releaseNotes\/([^/]+)\/([a-z]+)\.md$/.exec(path);
    if (!match) continue;
    const [, version, language] = match;
    if (!isSupportedLanguage(language)) continue;
    bundle[version] = { ...bundle[version], [language]: markdown };
  }
  return bundle;
}

export const RELEASE_NOTES: ReleaseNotesBundle = bundleFromModules(
  import.meta.glob<string>('/src/releaseNotes/*/*.md', { query: '?raw', import: 'default', eager: true }),
);

/** `0.7.0` → `[0, 7, 0]`; null for anything else (same rule as the backend's
 *  `services::updates::parse_version`, minus the `v` prefix). */
function parseVersion(raw: string): [number, number, number] | null {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(raw.trim());
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
}

function compareVersions(a: [number, number, number], b: [number, number, number]): number {
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

export interface ReleaseNotesPlan {
  /** Versions whose notes to show, newest first. */
  versions: string[];
  /** Whether to record the running version as seen (after the dialog closes
   *  when `versions` is non-empty, at once otherwise). */
  markSeen: boolean;
}

/**
 * Decide which release notes to show at startup. Notes appear once, on the
 * first launch after an upgrade, for every bundled version newer than the
 * last one seen up to the running one. A fresh install (the onboarding wizard
 * is showing) has nothing to catch up on; a build that predates this feature
 * never recorded a version, so only the running version's notes are shown.
 */
export function planReleaseNotes(input: {
  bundle: ReleaseNotesBundle;
  freshInstall: boolean;
  lastSeen: string | null;
  current: string;
}): ReleaseNotesPlan {
  const current = parseVersion(input.current);
  if (!current) return { versions: [], markSeen: false };
  if (input.lastSeen === input.current) return { versions: [], markSeen: false };
  if (input.freshInstall) return { versions: [], markSeen: true };

  const lastSeen = input.lastSeen ? parseVersion(input.lastSeen) : null;
  const versions = Object.keys(input.bundle)
    .map((version) => ({ version, parsed: parseVersion(version) }))
    .filter(({ parsed }) => {
      if (!parsed || compareVersions(parsed, current) > 0) return false;
      return lastSeen ? compareVersions(parsed, lastSeen) > 0 : compareVersions(parsed, current) === 0;
    })
    .sort((a, b) => compareVersions(b.parsed as [number, number, number], a.parsed as [number, number, number]))
    .map(({ version }) => version);
  return { versions, markSeen: true };
}

/** The notes of `version` in `language`, falling back to English. */
export function releaseNoteMarkdown(bundle: ReleaseNotesBundle, version: string, language: Language): string {
  return bundle[version]?.[language] ?? bundle[version]?.en ?? '';
}
