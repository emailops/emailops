// Sidebar density: the vertical spacing of the sidebar's scrolling list
// (accounts, views, folders, AI features, lenses, smart filters). Compact, the
// default, tightens padding and gaps only, never text size, icons, horizontal
// padding, or the fixed header and footer; Comfortable is the spacing the
// sidebar had before.
//
// Stored in the `sidebar_density` SQLite preference as the plain string
// `comfortable` or `compact`. That is a persisted format: do not rename the
// key or the values without a migration.
//
// Every class is a complete literal: Tailwind only generates classes it finds
// written out in the source, so never build them from parts.

export const SIDEBAR_DENSITIES = ['comfortable', 'compact'] as const;

export type SidebarDensity = (typeof SIDEBAR_DENSITIES)[number];

/** What an installation without a saved choice gets. The default is never
 *  written to the database, so a saved Comfortable stays. */
export const DEFAULT_SIDEBAR_DENSITY: SidebarDensity = 'compact';

export function isSidebarDensity(value: unknown): value is SidebarDensity {
  return typeof value === 'string' && (SIDEBAR_DENSITIES as readonly string[]).includes(value);
}

/** The vertical-spacing class of each sidebar role. Each one replaces only the
 *  spacing class at its call site; the rest of the class string is unchanged. */
export interface SidebarDensityClasses {
  /** The scrolling `<nav>`: its padding and the gap between sections. */
  nav: string;
  /** Margin under each section header. */
  sectionHeader: string;
  /** Gap between rows in the views, other views, folders and AI lists. */
  list: string;
  /** Padding of a view, folder or AI row (`text-sm`). */
  viewRow: string;
  /** Padding of an account row (`text-xs`). */
  accountRow: string;
  /** Padding of a smart-filter row (`text-sm`). */
  filterRow: string;
  /** Margin above each smart-filter tag group. */
  tagGroup: string;
}

// Compact keeps every row it changes at 24 px or taller (WCAG 2.5.8):
// view rows 28 px, account and filter rows 24 px. Account rows need the
// min-h-6 floor: WebKit lays their truncated text-xs label out at 15 px, so
// py-1 alone leaves them at 23 px.
const CLASSES: Record<SidebarDensity, SidebarDensityClasses> = {
  comfortable: {
    nav: 'p-4 space-y-6',
    sectionHeader: 'mb-2',
    list: 'space-y-1',
    viewRow: 'py-2',
    accountRow: 'py-1.5',
    filterRow: 'py-1.5',
    tagGroup: 'mt-3',
  },
  compact: {
    nav: 'px-4 py-2 space-y-3',
    sectionHeader: 'mb-1.5',
    list: 'space-y-0.5',
    viewRow: 'py-1',
    accountRow: 'py-1 min-h-6',
    filterRow: 'py-0.5',
    tagGroup: 'mt-1.5',
  },
};

export function sidebarDensityClasses(density: SidebarDensity): SidebarDensityClasses {
  return CLASSES[density];
}
