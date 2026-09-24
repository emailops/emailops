import type { SettingsTab } from '@/components/Settings/SettingsDialog';
import type { ViewMode } from '@/components/Sidebar/Sidebar';
import { plainTextToHtml } from '@/lib/composeHtml';

/** Settings tabs a guide section may open. Mirrors `SETTINGS_TABS` in
 *  `src-tauri/src/services/help_docs/nav.rs`; typed against `SettingsTab` so
 *  a tab that stops existing fails `tsc`, not the user. */
export const NAVIGABLE_SETTINGS_TABS: readonly SettingsTab[] = [
  'appearance',
  'calendar',
  'ai',
  'classification',
  'junk',
  'tasks',
  'memory',
  'lenses',
  'aidrafts',
  'aitranslation',
  'aisearch',
  'privacy',
];

/** Main views a guide section may open. Mirrors `VIEWS` in `nav.rs`. */
export const NAVIGABLE_VIEWS: readonly ViewMode[] = [
  'inbox',
  'attachments',
  'contacts',
  'drafts',
  'sent',
  'spam',
  'deleted',
  'calendar',
  'chat',
  'tasks',
  'memory',
  'lenses',
  'tagboard',
  'dashboard',
];

export type NavTarget = { kind: 'settings'; tab: SettingsTab } | { kind: 'view'; view: ViewMode };

/** Parse a `navigateTo` target (`settings/<tab>` | `view/<mode>`). `null`
 *  for anything outside the allowlists — the backend validates too, but a
 *  version skew must never open something that does not exist. */
export function parseNavTarget(target: string): NavTarget | null {
  const slash = target.indexOf('/');
  if (slash === -1) return null;
  const kind = target.slice(0, slash);
  const name = target.slice(slash + 1);
  if (kind === 'settings') {
    const tab = NAVIGABLE_SETTINGS_TABS.find((t) => t === name);
    return tab ? { kind: 'settings', tab } : null;
  }
  if (kind === 'view') {
    const view = NAVIGABLE_VIEWS.find((v) => v === name);
    return view ? { kind: 'view', view } : null;
  }
  return null;
}

/**
 * Side-effect a chat tool can ask the frontend to perform after a
 * successful tool run. Mirrors `services::chat::tools::ToolEffect` on the
 * Rust side (serde tags the variant with `kind`, fields are camelCase).
 * Adding a new effect = add a variant on the Rust side and a case here.
 */
export type ChatToolEffectPayload =
  | {
      kind: 'openComposer';
      draftId: string;
      accountId: string;
      /** Present on reply drafts (id of the inbound email being replied to);
       *  absent on brand-new drafts. The dispatcher routes on this — replies
       *  open inline inside the matching thread, new mails open in a
       *  standalone compose tab. */
      emailId?: string;
      toAddresses: string[];
      subject: string;
      body: string;
    }
  | {
      /** Open a part of the app after an answer that cites a guide section
       *  carrying a `nav:` target (see `services::help_docs::nav`). */
      kind: 'navigateTo';
      /** `settings/<tab>` or `view/<mode>`. */
      target: string;
      /** The cited section, e.g. "AI features › Choosing a backend". */
      title: string;
    }
  | {
      /** Open one of the app's forms with the fields the model filled in, for
       *  the user to review and submit. Fired after the query planner returns
       *  a `form` verdict (see `services::chat::form_turn`). */
      kind: 'fillForm';
      /** A key of `services::forms::registry::FORMS`, e.g. `lens.create`. */
      formId: string;
      /** `view/<mode>#<anchor>` — where the form lives. */
      target: string;
      /** Only keys the form declares, already coerced to their declared kinds. */
      values: Record<string, unknown>;
      /** Required fields the model could not fill, so the UI can focus the
       *  first one instead of the user hunting for it. */
      missingRequired: string[];
    }
  // Unknown kinds are passed through so the handler can log them without
  // throwing — future variants on the backend shouldn't crash an older UI.
  | { kind: string; [field: string]: unknown };

/** Forms the chat can fill. Mirrors `FORMS` in
 *  `src-tauri/src/services/forms/registry.rs`; an id outside this list is
 *  ignored rather than routed, so a version skew cannot open something that
 *  does not exist. */
export const FILLABLE_FORM_IDS = ['lens.create'] as const;
export type FillableFormId = (typeof FILLABLE_FORM_IDS)[number];

export interface ChatToolEffectHandlers {
  /** Open the composer tab pre-loaded with these fields. Pass `bodyHtml`
   *  (already HTML-converted) so the rich-text editor renders correctly. */
  openComposeTab: (accountId: string, toAddresses: string[], subject: string, bodyHtml: string) => void;
  /** Open the thread of `emailId` and seed an inline reply with `body`.
   *  Mirrors clicking Reply on the thread — the wired implementation
   *  stashes the body on `emailStore.pendingChatDraft` and calls
   *  `navigateToEmail`; `EmailView` consumes the pending draft once the
   *  thread mounts. Passed the plain-text body since the inline
   *  ReplyCompose's textarea is plain text (HTML conversion happens at
   *  send time). */
  openThreadReply: (accountId: string, emailId: string, body: string) => void;
  /** Switch the main view so the freshly-opened compose tab is actually
   *  visible. Without this the tab is created but stays hidden behind the
   *  chat view (the email tab bar only renders for the inbox-family views). */
  navigateToInbox: () => void;
  /** Open the Settings dialog on `tab`. Optional so older call sites and
   *  tests that never navigate keep compiling. */
  openSettingsTab?: (tab: SettingsTab) => void;
  /** Switch the main view. Same optionality as `openSettingsTab`. */
  navigateToView?: (view: ViewMode) => void;
  /** Open `formId` pre-filled with `values`, leaving the chat panel visible
   *  and usable. `missingRequired` names the fields to highlight. Optional so
   *  older call sites and tests that never fill a form keep compiling. */
  openFilledForm?: (formId: FillableFormId, values: Record<string, unknown>, missingRequired: string[]) => void;
  /** Optional logger — info/success/error/debug. Matches `useLogStore.addLog`. */
  log?: (level: 'info' | 'success' | 'error' | 'debug', source: 'ai', message: string) => void;
}

/**
 * Pure dispatcher for a single `chat-tool-effect` event payload.
 *
 * Extracted from `App.tsx`'s listener so the routing logic is testable
 * without spinning up the whole Tauri runtime / React tree.
 */
export function handleChatToolEffect(payload: ChatToolEffectPayload, handlers: ChatToolEffectHandlers): void {
  const log = handlers.log ?? (() => undefined);
  switch (payload.kind) {
    case 'openComposer': {
      const p = payload as Extract<ChatToolEffectPayload, { kind: 'openComposer' }>;
      // Backend types the fields as non-optional but defensively guard for
      // an older UI receiving a newer-shape payload.
      if (typeof p.accountId !== 'string' || typeof p.subject !== 'string' || typeof p.body !== 'string') {
        log('error', 'ai', `openComposer effect missing fields: ${JSON.stringify(payload)}`);
        return;
      }
      // Reply path — emailId is set when the chat tool replied to an
      // existing inbound. Open the inline reply inside that thread so the
      // UX matches clicking "Reply" on the thread itself. Seed the
      // pending-draft slot BEFORE navigating: EmailView reads it during
      // the same render pass that mounts the loaded thread, so getting
      // that order wrong would race with the navigation finishing and
      // open a stale empty Reply.
      if (typeof p.emailId === 'string' && p.emailId.length > 0) {
        handlers.openThreadReply(p.accountId, p.emailId, p.body);
        handlers.navigateToInbox();
        log('success', 'ai', `Reply opened in thread from chat (draft ${p.draftId ?? '?'})`);
        return;
      }
      // New-mail path — no thread to attach to, so fall back to the
      // standalone Compose tab. Switch BEFORE opening so the new compose
      // tab is the active visible tab the moment the inbox view paints —
      // otherwise the tab is appended but the user only sees the chat
      // panel and the draft looks like a no-op.
      handlers.navigateToInbox();
      handlers.openComposeTab(p.accountId, p.toAddresses ?? [], p.subject, plainTextToHtml(p.body));
      log('success', 'ai', `Composer opened from chat (draft ${p.draftId ?? '?'})`);
      return;
    }
    case 'navigateTo': {
      const p = payload as Extract<ChatToolEffectPayload, { kind: 'navigateTo' }>;
      if (typeof p.target !== 'string') {
        log('error', 'ai', `navigateTo effect missing target: ${JSON.stringify(payload)}`);
        return;
      }
      const target = parseNavTarget(p.target);
      if (!target) {
        log('error', 'ai', `navigateTo effect ignored — unknown target "${p.target}"`);
        return;
      }
      const title = typeof p.title === 'string' && p.title.length > 0 ? p.title : p.target;
      if (target.kind === 'settings') {
        if (!handlers.openSettingsTab) {
          log('debug', 'ai', 'navigateTo effect ignored — no settings handler wired');
          return;
        }
        handlers.openSettingsTab(target.tab);
        log('success', 'ai', `Opened Settings › ${target.tab} from chat (${title})`);
        return;
      }
      if (!handlers.navigateToView) {
        log('debug', 'ai', 'navigateTo effect ignored — no view handler wired');
        return;
      }
      handlers.navigateToView(target.view);
      log('success', 'ai', `Opened ${target.view} from chat (${title})`);
      return;
    }
    case 'fillForm': {
      const p = payload as Extract<ChatToolEffectPayload, { kind: 'fillForm' }>;
      const formId = FILLABLE_FORM_IDS.find((id) => id === p.formId);
      if (!formId) {
        log('error', 'ai', `fillForm effect ignored — unknown form "${String(p.formId)}"`);
        return;
      }
      if (typeof p.values !== 'object' || p.values === null || Array.isArray(p.values)) {
        log('error', 'ai', `fillForm effect ignored — values is not an object: ${JSON.stringify(payload)}`);
        return;
      }
      if (!handlers.openFilledForm) {
        log('debug', 'ai', 'fillForm effect ignored — no form handler wired');
        return;
      }
      const missing = Array.isArray(p.missingRequired) ? p.missingRequired.filter((m) => typeof m === 'string') : [];
      handlers.openFilledForm(formId, p.values, missing);
      log('success', 'ai', `Opened ${formId} pre-filled from chat (${Object.keys(p.values).length} field(s))`);
      return;
    }
    default:
      log('debug', 'ai', `Unhandled chat-tool-effect kind: ${payload.kind}`);
  }
}
