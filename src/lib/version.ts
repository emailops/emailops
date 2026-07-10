import type { BuildInfo } from '@/lib/api';

/**
 * Human label for the sidebar version line. Releases show "v0.6.2"; anything
 * built from an untagged commit appends the short sha — "v0.6.2 (05ae613)" —
 * so a screenshot or bug report identifies the exact code. Builds without git
 * metadata (source tarball) degrade to the bare version.
 */
export function formatVersionLabel(info: BuildInfo): string {
  const base = `v${info.version}`;
  return !info.isRelease && info.commit ? `${base} (${info.commit})` : base;
}
