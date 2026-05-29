// Type augmentation for i18next — gives `t('ns:foo.bar')` and `useTranslation('ns')`
// strongly-typed keys derived from the English bundle. We use `en` as the
// source of truth because the key-parity Vitest test guarantees every other
// language has the same key set (see `src/__tests__/i18n.parity.test.ts`).
//
// When you add a new namespace:
//   1. Drop the JSON in `src/locales/<lang>/<ns>.json` for every language.
//   2. Register it in `src/i18n/resources.ts` (imports + `NAMESPACES` + `resources`).
//   3. Add it under `CustomTypeOptions.resources` below so `t()` knows about it.

import 'i18next';

import type attachments from '../locales/en/attachments.json';
import type auth from '../locales/en/auth.json';
import type chat from '../locales/en/chat.json';
import type common from '../locales/en/common.json';
import type compose from '../locales/en/compose.json';
import type contacts from '../locales/en/contacts.json';
import type dashboard from '../locales/en/dashboard.json';
import type errors from '../locales/en/errors.json';
import type inbox from '../locales/en/inbox.json';
import type lenses from '../locales/en/lenses.json';
import type memory from '../locales/en/memory.json';
import type modal from '../locales/en/modal.json';
import type notifications from '../locales/en/notifications.json';
import type settings from '../locales/en/settings.json';
import type sidebar from '../locales/en/sidebar.json';
import type tasks from '../locales/en/tasks.json';

declare module 'i18next' {
  interface CustomTypeOptions {
    defaultNS: 'common';
    resources: {
      common: typeof common;
      sidebar: typeof sidebar;
      settings: typeof settings;
      modal: typeof modal;
      inbox: typeof inbox;
      chat: typeof chat;
      memory: typeof memory;
      tasks: typeof tasks;
      contacts: typeof contacts;
      compose: typeof compose;
      auth: typeof auth;
      errors: typeof errors;
      notifications: typeof notifications;
      lenses: typeof lenses;
      dashboard: typeof dashboard;
      attachments: typeof attachments;
    };
    returnNull: false;
  }
}
