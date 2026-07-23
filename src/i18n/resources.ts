// Static bundle of all locale resources. We import the JSON at build time
// rather than fetching it at runtime — i18n bundles are small (~5-10 KB per
// language) and the desktop bundle is already shipping every file anyway.
//
// Adding a new namespace: drop a new JSON in `src/locales/<lang>/<ns>.json`,
// import it here, and add it under both `resources[lang]` and the `NAMESPACES`
// array. Then add it to the `Resources` interface at the bottom for typed keys.

import deAttachments from '../locales/de/attachments.json';
import deAuth from '../locales/de/auth.json';
import deCalendar from '../locales/de/calendar.json';
import deChat from '../locales/de/chat.json';
import deCommon from '../locales/de/common.json';
import deCompose from '../locales/de/compose.json';
import deContacts from '../locales/de/contacts.json';
import deDashboard from '../locales/de/dashboard.json';
import deErrors from '../locales/de/errors.json';
import deInbox from '../locales/de/inbox.json';
import deLenses from '../locales/de/lenses.json';
import deMemory from '../locales/de/memory.json';
import deModal from '../locales/de/modal.json';
import deNotifications from '../locales/de/notifications.json';
import deSettings from '../locales/de/settings.json';
import deSidebar from '../locales/de/sidebar.json';
import deTasks from '../locales/de/tasks.json';
import enAttachments from '../locales/en/attachments.json';
import enAuth from '../locales/en/auth.json';
import enCalendar from '../locales/en/calendar.json';
import enChat from '../locales/en/chat.json';
import enCommon from '../locales/en/common.json';
import enCompose from '../locales/en/compose.json';
import enContacts from '../locales/en/contacts.json';
import enDashboard from '../locales/en/dashboard.json';
import enErrors from '../locales/en/errors.json';
import enInbox from '../locales/en/inbox.json';
import enLenses from '../locales/en/lenses.json';
import enMemory from '../locales/en/memory.json';
import enModal from '../locales/en/modal.json';
import enNotifications from '../locales/en/notifications.json';
import enSettings from '../locales/en/settings.json';
import enSidebar from '../locales/en/sidebar.json';
import enTasks from '../locales/en/tasks.json';
import esAttachments from '../locales/es/attachments.json';
import esAuth from '../locales/es/auth.json';
import esCalendar from '../locales/es/calendar.json';
import esChat from '../locales/es/chat.json';
import esCommon from '../locales/es/common.json';
import esCompose from '../locales/es/compose.json';
import esContacts from '../locales/es/contacts.json';
import esDashboard from '../locales/es/dashboard.json';
import esErrors from '../locales/es/errors.json';
import esInbox from '../locales/es/inbox.json';
import esLenses from '../locales/es/lenses.json';
import esMemory from '../locales/es/memory.json';
import esModal from '../locales/es/modal.json';
import esNotifications from '../locales/es/notifications.json';
import esSettings from '../locales/es/settings.json';
import esSidebar from '../locales/es/sidebar.json';
import esTasks from '../locales/es/tasks.json';
import frAttachments from '../locales/fr/attachments.json';
import frAuth from '../locales/fr/auth.json';
import frCalendar from '../locales/fr/calendar.json';
import frChat from '../locales/fr/chat.json';
import frCommon from '../locales/fr/common.json';
import frCompose from '../locales/fr/compose.json';
import frContacts from '../locales/fr/contacts.json';
import frDashboard from '../locales/fr/dashboard.json';
import frErrors from '../locales/fr/errors.json';
import frInbox from '../locales/fr/inbox.json';
import frLenses from '../locales/fr/lenses.json';
import frMemory from '../locales/fr/memory.json';
import frModal from '../locales/fr/modal.json';
import frNotifications from '../locales/fr/notifications.json';
import frSettings from '../locales/fr/settings.json';
import frSidebar from '../locales/fr/sidebar.json';
import frTasks from '../locales/fr/tasks.json';

export const SUPPORTED_LANGUAGES = ['en', 'es', 'fr', 'de'] as const;

export type Language = (typeof SUPPORTED_LANGUAGES)[number];

export const FALLBACK_LANGUAGE: Language = 'en';

/** True when `code` is one of the supported UI languages. */
export function isSupportedLanguage(code: string | null | undefined): code is Language {
  return typeof code === 'string' && (SUPPORTED_LANGUAGES as readonly string[]).includes(code);
}

/**
 * Ordered list of namespaces — keep in sync with `resources[lang]` below and
 * with the `Resources` interface used for typed `t()` keys.
 */
export const NAMESPACES = [
  'common',
  'sidebar',
  'settings',
  'modal',
  'inbox',
  'chat',
  'memory',
  'tasks',
  'contacts',
  'compose',
  'auth',
  'calendar',
  'errors',
  'notifications',
  'lenses',
  'dashboard',
  'attachments',
] as const;

export type Namespace = (typeof NAMESPACES)[number];

export const resources = {
  en: {
    common: enCommon,
    sidebar: enSidebar,
    settings: enSettings,
    modal: enModal,
    inbox: enInbox,
    chat: enChat,
    memory: enMemory,
    tasks: enTasks,
    contacts: enContacts,
    compose: enCompose,
    auth: enAuth,
    calendar: enCalendar,
    errors: enErrors,
    notifications: enNotifications,
    lenses: enLenses,
    dashboard: enDashboard,
    attachments: enAttachments,
  },
  es: {
    common: esCommon,
    sidebar: esSidebar,
    settings: esSettings,
    modal: esModal,
    inbox: esInbox,
    chat: esChat,
    memory: esMemory,
    tasks: esTasks,
    contacts: esContacts,
    compose: esCompose,
    auth: esAuth,
    calendar: esCalendar,
    errors: esErrors,
    notifications: esNotifications,
    lenses: esLenses,
    dashboard: esDashboard,
    attachments: esAttachments,
  },
  fr: {
    common: frCommon,
    sidebar: frSidebar,
    settings: frSettings,
    modal: frModal,
    inbox: frInbox,
    chat: frChat,
    memory: frMemory,
    tasks: frTasks,
    contacts: frContacts,
    compose: frCompose,
    auth: frAuth,
    calendar: frCalendar,
    errors: frErrors,
    notifications: frNotifications,
    lenses: frLenses,
    dashboard: frDashboard,
    attachments: frAttachments,
  },
  de: {
    common: deCommon,
    sidebar: deSidebar,
    settings: deSettings,
    modal: deModal,
    inbox: deInbox,
    chat: deChat,
    memory: deMemory,
    tasks: deTasks,
    contacts: deContacts,
    compose: deCompose,
    auth: deAuth,
    calendar: deCalendar,
    errors: deErrors,
    notifications: deNotifications,
    lenses: deLenses,
    dashboard: deDashboard,
    attachments: deAttachments,
  },
} as const;

export const defaultNS = 'common' as const;

/**
 * Native names used in the language selector. Showing the language in its
 * own script lets a user stuck on the wrong language still find their own.
 */
export const NATIVE_NAMES: Record<Language, string> = {
  en: 'English',
  es: 'Español',
  fr: 'Français',
  de: 'Deutsch',
};
