// What chrome the current view shows around its content: the title bar's
// label, and whether the inbox's category tab strip earns its row.
//
// Pure and string-keyed so the decisions are table-testable without a DOM and
// without i18next — callers pass the returned key to `t()`.

/** i18n key naming a view. A literal union, not `string`, so `t()` keeps
 *  checking that every one of them actually exists in the locale files. */
export type ViewTitleKey =
  | 'sidebar:inbox'
  | 'sidebar:attachments'
  | 'sidebar:contacts'
  | 'sidebar:drafts'
  | 'sidebar:sent'
  | 'sidebar:spam'
  | 'sidebar:deleted'
  | 'sidebar:calendar'
  | 'sidebar:chat'
  | 'sidebar:tasks'
  | 'sidebar:memory'
  | 'sidebar:lenses'
  | 'sidebar:dashboard'
  | 'sidebar:logs';

/** Reuses the sidebar's own labels so the title bar and the navigation entry
 *  that led there always read the same. */
const VIEW_TITLE_KEYS: Record<string, ViewTitleKey> = Object.assign(Object.create(null), {
  inbox: 'sidebar:inbox',
  attachments: 'sidebar:attachments',
  contacts: 'sidebar:contacts',
  drafts: 'sidebar:drafts',
  sent: 'sidebar:sent',
  spam: 'sidebar:spam',
  deleted: 'sidebar:deleted',
  calendar: 'sidebar:calendar',
  chat: 'sidebar:chat',
  tasks: 'sidebar:tasks',
  memory: 'sidebar:memory',
  lenses: 'sidebar:lenses',
  dashboard: 'sidebar:dashboard',
  logs: 'sidebar:logs',
  // Null-prototype: a plain literal would resolve 'constructor'/'toString' to
  // inherited members and render them as titles.
});

/**
 * The i18n key titling a view, or `null` when the caller must supply the text.
 *
 * `folder:<path>` views return `null`: their name is user-created and lives in
 * the folder store, not in any locale file.
 */
export function viewTitleKey(viewMode: string): ViewTitleKey | null {
  return VIEW_TITLE_KEYS[viewMode] ?? null;
}

/**
 * Whether the inbox should render its category tab strip.
 *
 * A single category has nothing to switch between — the strip is then a row of
 * chrome restating what the view already is ("Primary" above a list that can
 * only be Primary). It costs a whole row, which on a phone is most of a
 * message. The "All" tab is likewise only offered from two categories up.
 */
export function shouldShowCategoryTabs(showCategoryFilter: boolean, visibleCategoryCount: number): boolean {
  return showCategoryFilter && visibleCategoryCount > 1;
}
