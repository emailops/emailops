// Which navigation entries the sidebar shows, and under which heading.
//
// Pure and data-only so the membership rules are table-testable without a DOM:
// the Sidebar renders one <li> per returned entry and never decides visibility
// itself. Ordering inside a section is the render order.

/** A navigable destination in the sidebar. Matches `ViewMode` for every entry
 *  that navigates; `chat` is the exception — it toggles the docked panel. */
export type SidebarEntry =
  | 'inbox'
  | 'chat'
  | 'attachments'
  | 'drafts'
  | 'sent'
  | 'calendar'
  | 'spam'
  | 'deleted'
  | 'contacts'
  | 'dashboard'
  | 'tasks'
  | 'memory'
  | 'lenses';

export type SidebarSectionId = 'views' | 'otherViews';

export interface SidebarSection {
  id: SidebarSectionId;
  /** i18n key for the collapsible header. A literal union, not `string`, so
   *  `t()` keeps checking the key exists in every locale file. */
  titleKey: 'sidebar:views' | 'sidebar:otherViews';
  entries: SidebarEntry[];
}

export interface SidebarFeatureFlags {
  /** Master AI switch. When off, every AI-backed entry disappears. */
  aiEnabled: boolean;
  tasksEnabled: boolean;
  memoriesEnabled: boolean;
  lensesEnabled: boolean;
  /** True when at least one account has calendar integration enabled. */
  calendarEnabled: boolean;
}

/**
 * The sidebar's sections, in render order.
 *
 * There is deliberately no "AI Features" section. An AI-backed view is still a
 * view, and a third header cost a whole row of a phone's drawer while splitting
 * navigation by *implementation* rather than by what the user is looking for.
 * Chat is promoted to the top of **Views**, right below Inbox, because it is a
 * primary destination; the rest (Tasks, Memory, Lenses) join Dashboard under
 * **Other Views**.
 *
 * The master AI switch still hides all of them wholesale, and each experimental
 * feature is additionally gated on its own flag — both must be on.
 */
export function sidebarSections(flags: SidebarFeatureFlags): SidebarSection[] {
  const views: SidebarEntry[] = ['inbox'];
  if (flags.aiEnabled) views.push('chat');
  views.push('attachments', 'drafts', 'sent');
  if (flags.calendarEnabled) views.push('calendar');

  // Dashboard leads the AI group's slot rather than trailing it: it is not AI
  // output (it reports account stats) and so must not move when the switch does.
  const otherViews: SidebarEntry[] = ['spam', 'deleted', 'contacts', 'dashboard'];
  if (flags.aiEnabled && flags.tasksEnabled) otherViews.push('tasks');
  if (flags.aiEnabled && flags.memoriesEnabled) otherViews.push('memory');
  if (flags.aiEnabled && flags.lensesEnabled) otherViews.push('lenses');

  return [
    { id: 'views', titleKey: 'sidebar:views', entries: views },
    { id: 'otherViews', titleKey: 'sidebar:otherViews', entries: otherViews },
  ];
}
