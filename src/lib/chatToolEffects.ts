import { plainTextToHtml } from '@/lib/composeHtml';

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
  // Unknown kinds are passed through so the handler can log them without
  // throwing — future variants on the backend shouldn't crash an older UI.
  | { kind: string; [field: string]: unknown };

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
    default:
      log('debug', 'ai', `Unhandled chat-tool-effect kind: ${payload.kind}`);
  }
}
