import { describe, expect, it } from 'vitest';
import { type SidebarFeatureFlags, sidebarSections } from './sidebarSections';

/** Everything on. The narrower cases turn one flag off at a time. */
const ALL_ON: SidebarFeatureFlags = {
  aiEnabled: true,
  tasksEnabled: true,
  memoriesEnabled: true,
  lensesEnabled: true,
  calendarEnabled: true,
  stacked: false,
};

const entriesOf = (flags: SidebarFeatureFlags, id: string) =>
  sidebarSections(flags).find((s) => s.id === id)?.entries ?? [];

describe('sidebarSections', () => {
  it('offers exactly two sections, in order', () => {
    // "AI Features" is gone: an AI-backed view is still a view, and a third
    // header pushed the list below the fold on a phone.
    expect(sidebarSections(ALL_ON).map((s) => s.id)).toEqual(['views', 'otherViews']);
  });

  it('titles each section from the sidebar locale keys', () => {
    expect(sidebarSections(ALL_ON).map((s) => s.titleKey)).toEqual(['sidebar:views', 'sidebar:otherViews']);
  });

  it('puts Chat directly below Inbox in Views', () => {
    expect(entriesOf(ALL_ON, 'views').slice(0, 2)).toEqual(['inbox', 'chat']);
  });

  it('keeps the mail views in Views', () => {
    expect(entriesOf(ALL_ON, 'views')).toEqual(['inbox', 'chat', 'attachments', 'drafts', 'sent', 'calendar']);
  });

  it('drops Calendar until an account enables it', () => {
    expect(entriesOf({ ...ALL_ON, calendarEnabled: false }, 'views')).not.toContain('calendar');
  });

  it('moves the remaining AI views into Other Views', () => {
    const other = entriesOf(ALL_ON, 'otherViews');
    for (const entry of ['tasks', 'memory', 'lenses', 'dashboard']) {
      expect(other).toContain(entry);
    }
  });

  it('keeps the pre-existing Other Views entries', () => {
    const other = entriesOf(ALL_ON, 'otherViews');
    for (const entry of ['spam', 'deleted', 'contacts']) {
      expect(other).toContain(entry);
    }
  });

  describe('with the master AI switch off', () => {
    const aiOff: SidebarFeatureFlags = { ...ALL_ON, aiEnabled: false };

    it('does not smuggle Chat into Views', () => {
      // The whole point of the switch: no AI surface anywhere, and promoting
      // Chat to Views must not become a back door around it.
      expect(entriesOf(aiOff, 'views')).not.toContain('chat');
    });

    it('hides Tasks, Memory and Lenses even with their own flags on', () => {
      const other = entriesOf(aiOff, 'otherViews');
      expect(other).not.toContain('tasks');
      expect(other).not.toContain('memory');
      expect(other).not.toContain('lenses');
    });

    it('keeps Dashboard, which reports account stats rather than AI output', () => {
      expect(entriesOf(aiOff, 'otherViews')).toContain('dashboard');
    });
  });

  it('gates each AI view on its own experimental flag too', () => {
    const cases: Array<[Partial<SidebarFeatureFlags>, string]> = [
      [{ tasksEnabled: false }, 'tasks'],
      [{ memoriesEnabled: false }, 'memory'],
      [{ lensesEnabled: false }, 'lenses'],
    ];
    for (const [override, entry] of cases) {
      expect(entriesOf({ ...ALL_ON, ...override }, 'otherViews')).not.toContain(entry);
    }
  });

  it('never lists the same view twice', () => {
    const all = sidebarSections(ALL_ON).flatMap((s) => s.entries);
    expect(new Set(all).size).toBe(all.length);
  });

  it('lists nothing but mail views when every feature is off', () => {
    const allOff: SidebarFeatureFlags = {
      aiEnabled: false,
      tasksEnabled: false,
      memoriesEnabled: false,
      lensesEnabled: false,
      calendarEnabled: false,
      stacked: false,
    };
    expect(entriesOf(allOff, 'views')).toEqual(['inbox', 'attachments', 'drafts', 'sent']);
    expect(entriesOf(allOff, 'otherViews')).toEqual(['spam', 'deleted', 'contacts', 'dashboard']);
  });

  describe('the log view', () => {
    it('is offered on a phone, where there is no docked log panel', () => {
      // The dock is desktop-only (`!isStacked` in App.tsx), so without this the
      // output panel — the only place backend progress and errors surface — is
      // unreachable on the platform hardest to debug by other means.
      expect(entriesOf({ ...ALL_ON, stacked: true }, 'otherViews')).toContain('logs');
    });

    it('is not duplicated on desktop, where the dock already shows it', () => {
      expect(entriesOf({ ...ALL_ON, stacked: false }, 'otherViews')).not.toContain('logs');
    });

    it('does not depend on the AI switch', () => {
      // Logs cover sync, accounts and system too. Hiding them with AI would
      // remove the diagnostic exactly when a user has turned AI off to isolate
      // a problem.
      const aiOff = { ...ALL_ON, stacked: true, aiEnabled: false };
      expect(entriesOf(aiOff, 'otherViews')).toContain('logs');
    });

    it('sits last, after the views a user navigates to on purpose', () => {
      const other = entriesOf({ ...ALL_ON, stacked: true }, 'otherViews');
      expect(other[other.length - 1]).toBe('logs');
    });
  });
});
