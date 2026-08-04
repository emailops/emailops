/**
 * Per-calendar colours for the calendar view.
 *
 * Events are tinted by the calendar they belong to, using the colour the
 * provider reports (Google `calendarList.backgroundColor`, Graph `hexColor`).
 * Providers do not always give one — Graph's named presets have no documented
 * hex — so calendars without a colour get a deterministic slot from a fallback
 * palette instead: same calendar, same colour, every launch.
 *
 * All functions here are pure so they can be unit-tested without React.
 */

/** Fallback palette for calendars the provider gave no colour for. Chosen to
 *  stay distinguishable from each other and legible in light and dark mode. */
export const FALLBACK_CALENDAR_COLORS = [
  '#039be5', // blue
  '#33b679', // green
  '#f4511e', // orange
  '#8e24aa', // purple
  '#e67c73', // salmon
  '#f6bf26', // yellow
  '#0b8043', // dark green
  '#3f51b5', // indigo
  '#d81b60', // pink
  '#616161', // grey
] as const;

const HEX_RE = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i;

/** Whether a provider colour string is a usable hex colour. */
export function isHexColor(value: string | null | undefined): boolean {
  return typeof value === 'string' && HEX_RE.test(value.trim());
}

/** Expand `#abc` to `#aabbcc`; pass `#aabbcc` through. Input must be valid. */
function expand(hex: string): string {
  const raw = hex.trim().toLowerCase();
  if (raw.length === 4) {
    return `#${raw[1]}${raw[1]}${raw[2]}${raw[2]}${raw[3]}${raw[3]}`;
  }
  return raw;
}

/** Stable non-negative hash of a string — FNV-1a, so the same calendar id
 *  always lands on the same palette slot across launches and machines. */
function hashString(value: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * The colour to paint a calendar with: the provider's own colour when it gave
 * one, otherwise a deterministic fallback keyed on the calendar's id.
 */
export function calendarColor(providerColor: string | null | undefined, calendarId: string): string {
  if (isHexColor(providerColor)) {
    return expand(providerColor as string);
  }
  return FALLBACK_CALENDAR_COLORS[hashString(calendarId) % FALLBACK_CALENDAR_COLORS.length];
}

/** Relative luminance per WCAG 2.1, for a valid `#rrggbb` colour. */
function luminance(hex: string): number {
  const full = expand(hex);
  const channels = [full.slice(1, 3), full.slice(3, 5), full.slice(5, 7)].map((pair) => {
    const srgb = parseInt(pair, 16) / 255;
    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

/**
 * Black or white text for a solid fill of `hex`, whichever contrasts more.
 * Used by the filled chips (all-day bars, month chips, legend swatches) where
 * the calendar colour is the background.
 */
export function readableTextColor(hex: string): string {
  // 0.179 is the luminance at which white and black text contrast equally
  // against the fill; above it, black wins.
  return luminance(hex) > 0.179 ? '#111827' : '#ffffff';
}

/** An 8-digit hex with `alpha` (0–1) applied, for translucent fills. */
export function withAlpha(hex: string, alpha: number): string {
  const clamped = Math.min(1, Math.max(0, alpha));
  const byte = Math.round(clamped * 255)
    .toString(16)
    .padStart(2, '0');
  return `${expand(hex)}${byte}`;
}

/** Inline styles for one event block, given its calendar colour.
 *
 *  Timed blocks use a translucent tint plus a solid left rule rather than a
 *  solid fill: a full-strength fill of ten different calendar colours makes a
 *  busy week unreadable, and a tint keeps the app's own text colour legible in
 *  both light and dark mode without per-colour contrast maths. Tentative
 *  events keep the existing dashed treatment, just in the calendar's colour. */
export function eventBlockStyle(color: string, tentative: boolean): Record<string, string> {
  return {
    backgroundColor: withAlpha(color, tentative ? 0.1 : 0.22),
    borderColor: color,
    borderLeftWidth: '3px',
    borderStyle: tentative ? 'dashed' : 'solid',
  };
}

/** Inline styles for a solid-filled chip (all-day bars, month-view chips). */
export function eventChipStyle(color: string): Record<string, string> {
  return { backgroundColor: color, color: readableTextColor(color) };
}

/** Minimal shape `calendarColorMap` / `hiddenCalendarIds` need — keeps these
 *  helpers testable without building a whole `Calendar`. */
export interface CalendarColorSource {
  providerCalendarId: string;
  color: string;
  isVisible: boolean;
}

/** Resolved colour per calendar id, for the grids' `colorFor` lookup. */
export function calendarColorMap(calendars: readonly CalendarColorSource[]): Map<string, string> {
  return new Map(calendars.map((c) => [c.providerCalendarId, calendarColor(c.color, c.providerCalendarId)]));
}

/** Ids of the calendars the user switched off in Settings → Calendar. */
export function hiddenCalendarIds(calendars: readonly CalendarColorSource[]): Set<string> {
  return new Set(calendars.filter((c) => !c.isVisible).map((c) => c.providerCalendarId));
}

/**
 * Drop events belonging to hidden calendars.
 *
 * Hidden calendars keep syncing, so this render-time filter is what actually
 * hides them — and it makes re-showing a calendar instant instead of waiting
 * for the next sync.
 */
export function visibleEvents<T extends { calendarId: string }>(
  events: readonly T[],
  hidden: ReadonlySet<string>,
): T[] {
  if (hidden.size === 0) return events as T[];
  return events.filter((e) => !hidden.has(e.calendarId));
}
