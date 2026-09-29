import type { SkillInfo } from '@/lib/api';

/** Most suggestions shown at once under the chat input. */
const MAX_SUGGESTIONS = 6;

/**
 * The skill name being typed when the message starts with `/` and the first
 * token is not finished yet (`/wee` → `wee`, `/` → `''`). `null` otherwise:
 * a finished `/name ` or a slash later in the text is not an invocation to
 * complete. Mirrors the backend rule that only a leading `/name` counts.
 */
export function slashQuery(value: string): string | null {
  const match = /^\s*\/([A-Za-z0-9-]*)$/.exec(value);
  return match ? match[1].toLowerCase() : null;
}

/** Enabled skills matching `query`: names that start with it first, then names
 *  that contain it, each group alphabetical, capped. */
export function matchSkills(skills: SkillInfo[], query: string): SkillInfo[] {
  const enabled = skills.filter((s) => s.enabled).sort((a, b) => a.name.localeCompare(b.name));
  const prefix = enabled.filter((s) => s.name.startsWith(query));
  const infix = enabled.filter((s) => !s.name.startsWith(query) && s.name.includes(query));
  return [...prefix, ...infix].slice(0, MAX_SUGGESTIONS);
}

/** The input text after picking a skill: the invocation plus a space for the request. */
export function applySuggestion(name: string): string {
  return `/${name} `;
}
