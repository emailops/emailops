import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArchiveIcon, ClockIcon, InboxIcon, StarIcon } from '@/components/common/MailIcons';
import { TagChips } from '@/components/common/TagChips';
import { SnoozeMenuButton } from '@/components/Inbox/SnoozePicker';
import { useFormatters } from '@/hooks/useFormatters';
import { useShortcutHint } from '@/hooks/useShortcutHint';
import type { DraftFailedEvent, DraftGeneratedEvent, DraftSource, EmailAttachment, OutgoingMessage } from '@/lib/api';
import * as api from '@/lib/api';
import { findThreadReplyDraft } from '@/lib/composeDraft';
import { createDraftRequestTracker, type DraftOutcome } from '@/lib/draftRequest';
import { errorText } from '@/lib/errors';
import { formatShortcut } from '@/lib/platform';
import type { PaneCommand } from '@/lib/shortcutPlan';
import { getThreadViewItems } from '@/lib/threadCollapse';
import { buildOccurrenceSlots, getThreadSearchMatches, stepMatchIndex } from '@/lib/threadSearch';
import { beginLeave, finishLeave } from '@/stores/autoAdvanceStore';
import { isSnoozed, isThreadStarred, isThreadUnread, threadRefOf, useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';
import { useOutboxStore } from '@/stores/outboxStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import { useTagStore } from '@/stores/tagStore';
import type { Account, Draft, Email, EmailAttachmentMeta } from '@/types';
import { AttachmentLightbox } from './AttachmentLightbox';
import { forwardQuote, forwardSubject, loadForwardBody } from './forward';
import { ReplyCompose } from './ReplyCompose';
import { ThreadEmailItem } from './ThreadEmailItem';

/** Date + time, because a forwarded header that says only the day loses the
 *  ordering the recipient needs to read a thread. */
const EMAIL_DATE_OPTIONS: Intl.DateTimeFormatOptions = {
  year: 'numeric',
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
};

/** Total attachment bytes a forward will carry. Providers reject well before
 *  this; the cap exists so one huge file cannot lock the webview while it is
 *  base64-encoded. Anything skipped is logged — never dropped silently. */
const MAX_FORWARD_BYTES = 20 * 1024 * 1024;

/** Stable empty array — the compose panel applies `initialAttachments` by
 *  identity, so a fresh `[]` on every render would re-run that effect. */
const EMPTY_ATTACHMENTS: EmailAttachment[] = [];

interface EmailViewProps {
  threadEmails: Email[];
  isLoading: boolean;
  onClose: () => void;
  accounts: Account[];
  activeAccountId: string | null;
  /** When true, renders at full width (used in full-width inbox layout). */
  fullWidth?: boolean;
  onOpenInTab?: () => void;
  /** Open a chat seeded with this thread. */
  onChatAboutThread?: (email: Email) => void;
  /** The email to expand and keep visible; a thread tab passes its own,
   *  otherwise the store's (set by navigating to an email) applies. */
  focusEmailId?: string | null;
}

/**
 * Combine an AI/chat-generated draft body with whatever the user (or a
 * previous draft event) has already typed into the inline reply. The body
 * lands on top so the suggestion is visible above the cursor; the user's
 * prior text stays below, separated by a blank line. When `existing` is
 * empty we return just `body` so the textbox doesn't open with a trailing
 * pair of newlines.
 *
 * Exported (and tested directly) so changes to the spacing rules don't
 * need to mount the full EmailView to verify.
 */
export function prependDraftBody(body: string, existing: string): string {
  return existing ? `${body}\n\n${existing}` : body;
}

/**
 * Whether the pending chat-generated draft should be consumed into the
 * inline reply on this render. We need the *thread* to be loaded, and the
 * inbound the draft was written for to be inside it. The previous
 * implementation compared only against the latest message in the thread,
 * which silently dropped the body whenever a later reply had arrived
 * between the chat turn and the click. Matching against any message in
 * the loaded thread restores the "click → open reply with body prepended"
 * UX the user expects.
 *
 * Exported (and accepting an `id` getter) so the unit tests in
 * `EmailView.test.ts` can pin the predicate without rendering React.
 */
export function shouldConsumePendingChatDraft(
  pendingChatDraft: { emailId: string } | null,
  threadEmailIds: readonly string[],
): boolean {
  if (!pendingChatDraft) return false;
  if (threadEmailIds.length === 0) return false;
  return threadEmailIds.includes(pendingChatDraft.emailId);
}

// Stable empty reference for the tag selector's missing-key fallback.
// zustand 5 dropped auto-shallow on selector results, so returning `|| []`
// inline produces a new array every render and trips React's
// useSyncExternalStore "getSnapshot should be cached" guard.
const EMPTY_TAGS: readonly string[] = Object.freeze([]);

export function EmailView({
  threadEmails,
  isLoading,
  onClose,
  accounts,
  activeAccountId,
  fullWidth,
  onOpenInTab,
  onChatAboutThread,
  focusEmailId: focusEmailIdProp,
}: EmailViewProps) {
  const { t } = useTranslation(['inbox', 'compose']);
  const fmt = useFormatters();
  const hint = useShortcutHint();
  const [expandedEmails, setExpandedEmails] = useState<Set<string>>(new Set());
  const [threadExpanded, setThreadExpanded] = useState(false);
  const [lightboxMeta, setLightboxMeta] = useState<EmailAttachmentMeta | null>(null);
  const storeFocusEmailId = useEmailStore((s) => s.focusEmailId);
  const focusEmailId = focusEmailIdProp === undefined ? storeFocusEmailId : focusEmailIdProp;
  const searchQuery = useEmailStore((s) => s.searchQuery);
  const deleteThreads = useEmailStore((s) => s.deleteThreads);
  const setThreadsRead = useEmailStore((s) => s.setThreadsRead);
  const setThreadsStarred = useEmailStore((s) => s.setThreadsStarred);
  const archiveThreads = useEmailStore((s) => s.archiveThreads);
  const moveThreadsToInbox = useEmailStore((s) => s.moveThreadsToInbox);
  const snoozeThreads = useEmailStore((s) => s.snoozeThreads);
  const unsnoozeThreads = useEmailStore((s) => s.unsnoozeThreads);
  const threadSnoozed = useEmailStore((s) => threadEmails.length > 0 && isSnoozed(s.snoozes, threadEmails[0]));
  const openAttachmentTab = useEmailStore((s) => s.openAttachmentTab);
  // Chat-generated reply draft waiting for its thread to mount. The chat
  // dispatcher seeds this before navigating; consuming it here is what
  // makes the chat draft land inside the inline ReplyCompose (same shape
  // as the AI Draft button) instead of a standalone compose tab.
  const pendingChatDraft = useEmailStore((s) => s.pendingChatDraft);
  const consumePendingChatDraft = useEmailStore((s) => s.consumePendingChatDraft);
  const refreshThread = useEmailStore((s) => s.refreshThread);
  const bumpSentRefresh = useEmailStore((s) => s.bumpSentRefresh);
  // In-thread search: matches are whole messages (subject/sender/snippet/body
  // text); the active match is highlighted in-body and scrolled into view.
  const [threadSearchOpen, setThreadSearchOpen] = useState(false);
  const [threadSearchQuery, setThreadSearchQuery] = useState('');
  const [threadMatchIdx, setThreadMatchIdx] = useState(0);
  const threadSearchInputRef = useRef<HTMLInputElement>(null);
  const [isReplyOpen, setIsReplyOpen] = useState(false);
  const [replyMode, setReplyMode] = useState<'reply' | 'reply-all' | 'forward'>('reply');
  // Attachments carried over from the message being forwarded, loaded on demand
  // — a forward that drops the boarding pass is worse than useless.
  const [forwardAttachments, setForwardAttachments] = useState<EmailAttachment[]>(EMPTY_ATTACHMENTS);
  // The reply draft saved for this thread, if any: restored when the thread
  // opens, and kept current as the reply panel saves, so reopening Reply
  // continues the same draft instead of starting a second one.
  const [threadDraft, setThreadDraft] = useState<Draft | null>(null);
  const [replyBody, setReplyBody] = useState('');
  const addLog = useLogStore((s) => s.addLog);
  // AI draft state. The request id is held in a ref so the event listener
  // (registered once on mount) can match incoming events without re-binding
  // every time a draft is requested.
  const draftTrackerRef = useRef(createDraftRequestTracker());
  const [isGeneratingDraft, setIsGeneratingDraft] = useState(false);
  const [draftSources, setDraftSources] = useState<DraftSource[]>([]);
  const [aiDraftsEnabled, setAiDraftsEnabled] = useState(true);

  useEffect(() => {
    api
      .getPref('ai_drafts_enabled')
      .then((val) => setAiDraftsEnabled(val !== 'false'))
      .catch(() => setAiDraftsEnabled(true));
  }, []);

  const applyDraftOutcomeRef = useRef<(outcome: DraftOutcome) => void>(() => {});
  applyDraftOutcomeRef.current = (outcome) => {
    setIsGeneratingDraft(false);
    if (outcome.kind === 'failed') {
      setDraftSources([]);
      addLog('error', 'ai', `AI draft failed: ${outcome.event.error}`);
      return;
    }
    const { body, sources } = outcome.event;
    setDraftSources(sources ?? []);
    // Prepend the AI body above anything the user has already typed,
    // so the suggested reply sits at the top of the textbox.
    setReplyBody((existing) => prependDraftBody(body, existing));
    addLog('success', 'ai', `AI draft ready (${sources?.length ?? 0} sources)`);
  };

  // Subscribe once to draft-generated / draft-failed. The tracker matches them
  // to the current request, so an event from a previous click that landed
  // after the user dismissed the compose doesn't mutate the textarea.
  useEffect(() => {
    let unlistenGen: UnlistenFn | undefined;
    let unlistenFail: UnlistenFn | undefined;
    void (async () => {
      unlistenGen = await listen<DraftGeneratedEvent>('draft-generated', (event) => {
        const outcome = draftTrackerRef.current.accept({ kind: 'generated', event: event.payload });
        if (outcome) applyDraftOutcomeRef.current(outcome);
      });
      unlistenFail = await listen<DraftFailedEvent>('draft-failed', (event) => {
        const outcome = draftTrackerRef.current.accept({ kind: 'failed', event: event.payload });
        if (outcome) applyDraftOutcomeRef.current(outcome);
      });
    })();
    return () => {
      unlistenGen?.();
      unlistenFail?.();
    };
  }, [addLog]);

  // AI Draft always opens in reply-all so the suggested body lands in a
  // compose with every thread participant prefilled. A new request replaces
  // whatever the composer holds; `instructions` steers the reply.
  const requestAiDraft = async (instructions?: string) => {
    const target = threadEmails[threadEmails.length - 1];
    if (!target) return;
    setReplyMode('reply-all');
    setReplyBody('');
    setDraftSources([]);
    setIsReplyOpen(true);
    setIsGeneratingDraft(true);
    addLog('info', 'ai', instructions ? 'Requesting AI draft with instructions…' : 'Requesting AI draft…');
    draftTrackerRef.current.begin();
    try {
      const requestId = await api.generateDraft(target.accountId, target.id, instructions || null);
      const early = draftTrackerRef.current.resolve(requestId);
      if (early) applyDraftOutcomeRef.current(early);
    } catch (err) {
      setIsGeneratingDraft(false);
      draftTrackerRef.current.cancel();
      addLog('error', 'ai', `Failed to start AI draft: ${err}`);
    }
  };

  const handleOpenAttachment = useCallback(
    (meta: EmailAttachmentMeta) => {
      if (meta.mimeType.startsWith('image/')) {
        setLightboxMeta(meta);
      } else {
        openAttachmentTab(meta);
      }
    },
    [openAttachmentTab],
  );

  const latestEmailId = threadEmails.length > 0 ? threadEmails[threadEmails.length - 1].id : '';
  const emailTags = useTagStore((s) => s.tagsByEmail[latestEmailId] || EMPTY_TAGS);
  const latestEmailForEffect = threadEmails.length > 0 ? threadEmails[threadEmails.length - 1] : null;
  // Read inside the draft-restore effect, which must run once per thread (keyed
  // on the latest email) rather than on every refresh of these values.
  const threadEmailsRef = useRef(threadEmails);
  threadEmailsRef.current = threadEmails;
  const isReplyOpenRef = useRef(isReplyOpen);
  isReplyOpenRef.current = isReplyOpen;
  const isThread = threadEmails.length > 1;

  const threadSearchActive = threadSearchOpen && threadSearchQuery.trim().length > 0;
  const threadMatches = useMemo(
    () => (threadSearchActive ? getThreadSearchMatches(threadEmails, threadSearchQuery) : []),
    [threadSearchActive, threadSearchQuery, threadEmails],
  );
  // Occurrence counts per email, reported asynchronously by each rendered
  // body frame after it applies the highlight. Slots flatten those counts
  // into one entry per occurrence so prev/next walks occurrences, not just
  // messages, and the counter can say "3 of 7".
  const [occurrenceCounts, setOccurrenceCounts] = useState<Record<string, number>>({});
  const occurrenceSlots = useMemo(
    () => buildOccurrenceSlots(threadMatches, occurrenceCounts),
    [threadMatches, occurrenceCounts],
  );
  const activeSlot =
    occurrenceSlots.length > 0 ? occurrenceSlots[Math.min(threadMatchIdx, occurrenceSlots.length - 1)] : null;

  const handleSearchMatches = useCallback((emailId: string, count: number) => {
    setOccurrenceCounts((prev) => (prev[emailId] === count ? prev : { ...prev, [emailId]: count }));
  }, []);

  const openThreadSearch = useCallback(() => {
    setThreadSearchOpen(true);
    // The input mounts on this state change; focus it on the next frame.
    requestAnimationFrame(() => threadSearchInputRef.current?.focus());
  }, []);

  const closeThreadSearch = useCallback(() => {
    setThreadSearchOpen(false);
    setThreadSearchQuery('');
    setThreadMatchIdx(0);
    setOccurrenceCounts({});
  }, []);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'f') return;
      // Don't hijack Cmd+F (nor steal focus) while the user is typing in some
      // other text field — e.g. the chat panel or a compose form that can be
      // focused while this view is mounted alongside them.
      const target = e.target as HTMLElement | null;
      const isOtherTextField =
        !!target &&
        (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) &&
        target !== threadSearchInputRef.current;
      if (isOtherTextField) return;
      e.preventDefault();
      openThreadSearch();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [openThreadSearch]);

  // When a search is active, find the oldest email in the thread whose subject,
  // snippet, or (already-loaded) body contains the query. We highlight that email
  // and scroll its first in-body match into view.
  const searchHighlightEmailId = useMemo(() => {
    const q = searchQuery?.trim().toLowerCase();
    if (!q) return null;
    const match = threadEmails.find(
      (e) =>
        e.subject.toLowerCase().includes(q) ||
        e.snippet.toLowerCase().includes(q) ||
        (e.body ?? '').toLowerCase().includes(q),
    );
    return match?.id ?? null;
  }, [threadEmails, searchQuery]);

  // Reset compose state when the user moves to a different thread / a new
  // latest message. Keyed on the latest email's ID, NOT its object identity:
  // refreshThread (after a send, and on sync batch/complete events) replaces
  // threadEmails with fresh object identities even when the content is
  // unchanged, and an identity-keyed reset closed the reply box the moment
  // it opened ("clicking Reply All does nothing").
  useEffect(() => {
    if (!latestEmailId) {
      setIsReplyOpen(false);
      setReplyBody('');
      setDraftSources([]);
      setIsGeneratingDraft(false);
      draftTrackerRef.current.cancel();
      return;
    }

    setIsReplyOpen(false);
    setReplyBody('');
    setThreadDraft(null);
    setThreadExpanded(false);
    setDraftSources([]);
    setIsGeneratingDraft(false);
    draftTrackerRef.current.cancel();
    setThreadSearchOpen(false);
    setThreadSearchQuery('');
    setThreadMatchIdx(0);
    setOccurrenceCounts({});
  }, [latestEmailId]);

  // Chat-generated reply draft: once the matching thread is loaded, open
  // the inline ReplyCompose with the AI body prepended on top of the
  // quoted template — same shape the AI Draft button produces. Runs after
  // the reset effect above (declaration order = execution order), so the
  // AI body lands on top of a freshly-rebuilt template instead of fighting
  // the reset. Consume clears the slot so re-rendering the same thread
  // (e.g. via account switch and back) does not re-open a stale draft.
  useEffect(() => {
    if (
      !shouldConsumePendingChatDraft(
        pendingChatDraft,
        threadEmails.map((e) => e.id),
      )
    )
      return;
    setReplyMode('reply');
    const draft = pendingChatDraft!;
    setReplyBody((existing) => prependDraftBody(draft.body, existing));
    setIsReplyOpen(true);
    setDraftSources([]);
    setIsGeneratingDraft(false);
    draftTrackerRef.current.cancel();
    consumePendingChatDraft();
  }, [pendingChatDraft, threadEmails, consumePendingChatDraft]);

  // Keyboard shortcuts (r, a, f, e, #, s, Shift+U/I, b) arrive as pane
  // commands and run through the same handlers as the toolbar buttons.
  // Only commands issued while this view is shown count: one from before it
  // mounted must not be replayed onto a different conversation.
  const [snoozeSignal, setSnoozeSignal] = useState(0);
  const paneCommand = useShortcutStore((s) => s.paneCommand);
  const seenPaneCommandRef = useRef(paneCommand?.nonce ?? 0);
  const runPaneCommandRef = useRef<(command: PaneCommand) => void>(() => {});
  runPaneCommandRef.current = () => {};
  useEffect(() => {
    if (!paneCommand || paneCommand.nonce <= seenPaneCommandRef.current) return;
    seenPaneCommandRef.current = paneCommand.nonce;
    runPaneCommandRef.current(paneCommand.command);
  }, [paneCommand]);

  // A reply the user started and left is saved as a draft of this thread:
  // opening the thread again brings it back in the reply panel. Runs after
  // the reset above; a reply already opened meanwhile (e.g. a chat draft) wins.
  // biome-ignore lint/correctness/useExhaustiveDependencies: once per thread, keyed on its latest email
  useEffect(() => {
    const emails = threadEmailsRef.current;
    const latest = emails[emails.length - 1];
    if (!latest) return;
    let cancelled = false;
    api
      .listDrafts(latest.accountId)
      .then((drafts) => {
        if (cancelled) return;
        const draft = findThreadReplyDraft(
          drafts,
          emails.map((e) => e.id),
        );
        if (!draft) return;
        setThreadDraft(draft);
        if (isReplyOpenRef.current) return;
        setReplyMode('reply');
        setReplyBody('');
        setForwardAttachments(EMPTY_ATTACHMENTS);
        setIsReplyOpen(true);
      })
      .catch((err) => addLog('error', 'sync', `Could not load this thread's reply draft: ${errorText(err)}`));
    return () => {
      cancelled = true;
    };
  }, [latestEmailId, addLog]);

  if (threadEmails.length === 0 && !isLoading) {
    return (
      <div className="flex-1 bg-white flex items-center justify-center">
        <div className="text-center p-8">
          <svg className="mx-auto h-12 w-12 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1}
              d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"
            />
          </svg>
          <h3 className="mt-2 text-sm font-medium text-gray-900">{t('inbox:noEmailSelected')}</h3>
          <p className="mt-1 text-sm text-gray-500">{t('inbox:selectEmailHint')}</p>
        </div>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="flex-1 bg-white flex items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600 mx-auto"></div>
          <p className="mt-2 text-sm text-gray-500">{t('inbox:loadingThread')}</p>
        </div>
      </div>
    );
  }

  const latestEmail = latestEmailForEffect!;

  const openReply = (mode: 'reply' | 'reply-all', toggle: boolean) => {
    setReplyMode(mode);
    setReplyBody('');
    setForwardAttachments(EMPTY_ATTACHMENTS);
    setIsReplyOpen((value) => (toggle ? !value : true));
  };
  const openForward = async () => {
    const body = await loadForwardBody(
      latestEmail,
      () => api.getEmailBody(latestEmail.accountId, latestEmail.id),
      (err) => addLog('error', 'sync', `Could not load the original message to forward: ${err}`),
    );
    setReplyMode('forward');
    setReplyBody(
      forwardQuote(
        { ...latestEmail, body },
        {
          header: t('compose:forwarded.header'),
          from: t('compose:forwarded.from'),
          date: t('compose:forwarded.date'),
          subject: t('compose:forwarded.subject'),
          to: t('compose:forwarded.to'),
          cc: t('compose:forwarded.cc'),
        },
        (ts) => fmt.date(ts, EMAIL_DATE_OPTIONS),
      ),
    );
    setForwardAttachments(EMPTY_ATTACHMENTS);
    setIsReplyOpen(true);
    void loadForwardAttachments();
  };
  const thread = [threadRefOf(latestEmail)];
  const inInbox = threadEmails.some((e) => e.mailbox === 'inbox');
  /** The conversation leaves the list: open the next one (or the previous,
   *  or go back to the list — Settings → Appearance). */
  const leave = (run: () => void) => {
    const ticket = beginLeave();
    run();
    finishLeave(ticket, { close: onClose });
  };
  const archive = () => leave(() => void archiveThreads(thread));
  const markUnread = () => {
    // Back to the list, like Gmail: staying on the thread would read it again
    // at once.
    void setThreadsRead(thread, false);
    onClose();
  };
  // Leaves at once; the provider call waits out the undo window and a
  // refusal brings the thread back (emailStore.deleteThreads).
  const deleteThread = () => leave(() => void deleteThreads(thread));
  runPaneCommandRef.current = (command) => {
    switch (command) {
      case 'reply':
        openReply('reply', false);
        return;
      case 'replyAll':
        openReply('reply-all', false);
        return;
      case 'forward':
        void openForward();
        return;
      case 'archive':
        if (inInbox) archive();
        return;
      case 'delete':
        deleteThread();
        return;
      case 'star':
        void setThreadsStarred(thread, !isThreadStarred(threadEmails));
        return;
      case 'markRead':
        void setThreadsRead(thread, true);
        return;
      case 'markUnread':
        markUnread();
        return;
      case 'snooze':
        if (inInbox && !threadSnoozed) setSnoozeSignal((n) => n + 1);
        return;
    }
  };

  /** Pull the message's own attachments in so the forward carries them.
   *
   *  Runs after the compose panel is already open: the user can start typing
   *  while a large PDF is still being encoded, and a failure here degrades to
   *  "forward without attachments" plus a log line rather than blocking the
   *  send. */
  const loadForwardAttachments = async () => {
    try {
      const metas = await api.getEmailAttachmentMetas(latestEmail.accountId, latestEmail.id);
      if (metas.length === 0) return;
      const carried: EmailAttachment[] = [];
      const skipped: string[] = [];
      let bytes = 0;
      for (const meta of metas) {
        if (bytes + meta.fileSize > MAX_FORWARD_BYTES) {
          skipped.push(meta.filename);
          continue;
        }
        const data = await api.fetchEmailAttachmentBytes(
          latestEmail.accountId,
          latestEmail.id,
          meta.providerAttachmentId,
        );
        carried.push({ filename: meta.filename, mimeType: meta.mimeType, data });
        bytes += meta.fileSize;
      }
      setForwardAttachments(carried);
      if (skipped.length > 0) {
        addLog('error', 'sync', `Too large to forward, attach manually: ${skipped.join(', ')}`);
      }
    } catch (err) {
      addLog('error', 'sync', `Could not attach the original files to the forward: ${err}`);
    }
  };

  const toggleEmailExpanded = (emailId: string) => {
    setExpandedEmails((prev) => {
      const next = new Set(prev);
      if (next.has(emailId)) {
        next.delete(emailId);
      } else {
        next.add(emailId);
      }
      return next;
    });
  };

  return (
    <div className="flex-1 bg-white flex flex-col overflow-hidden">
      {lightboxMeta && <AttachmentLightbox meta={lightboxMeta} onClose={() => setLightboxMeta(null)} />}
      {/* Capped at 60% so a tall reply (an AI draft plus its RAG sources) can
          never take the whole pane: the compose scrolls inside the header and
          the thread below keeps room to scroll. */}
      <header className="px-4 py-2 border-b border-gray-200 flex-shrink-0 flex flex-col max-h-[60%]">
        {/* Row 1: subject + inline tags on the left, window controls on the right.
            `flex-wrap` on both the row and the control cluster is load-bearing:
            with the chat panel docked the email pane can narrow to ~180px, and
            an unshrinkable single-line cluster overflowed the header — the
            ancestor's `overflow-hidden` then clipped its right edge, silently
            eating the LAST control, which is Close. Wrapping makes the header
            grow taller instead of hiding actions. `flex-1 basis-0` on the title
            keeps its long content from forcing a wrap at normal widths. */}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 min-w-0">
          {/*
           * A real flex basis, not 0: with basis-0 the title counted for
           * nothing when the row decided what fits, so on a laptop-width
           * full-width pane the chips and buttons took the line and the
           * subject shrank to one letter. Now the title keeps ~16rem and the
           * controls wrap under it when there is no room.
           */}
          <h1 className="flex-[1_1_16rem] min-w-0 text-lg font-semibold text-gray-900 truncate">
            {latestEmail.subject || '(No subject)'}
          </h1>
          {emailTags.length > 0 && (
            <div className="min-w-0">
              <TagChips tags={emailTags} />
            </div>
          )}
          {isThread && <span className="flex-shrink-0 text-xs text-gray-400">{threadEmails.length} msgs</span>}
          <div className="ml-auto flex flex-wrap items-center justify-end gap-1">
            <button
              onClick={() => openReply('reply', true)}
              className="px-3 py-1 bg-primary-600 text-white text-sm font-medium rounded hover:bg-primary-700 transition-colors"
              title={hint(t('compose:reply'), 'compose.reply')}
            >
              {t('compose:reply')}
            </button>
            <button
              onClick={() => openReply('reply-all', true)}
              className="px-3 py-1 bg-primary-500 text-white text-sm font-medium rounded hover:bg-primary-600 transition-colors"
              title={hint(t('compose:replyAll'), 'compose.replyAll')}
            >
              {t('inbox:emailView.replyAll')}
            </button>
            <button
              onClick={() => void openForward()}
              className="flex items-center gap-1.5 px-3 py-1 bg-gray-100 text-gray-700 text-sm font-medium rounded border border-gray-300 hover:bg-gray-200 transition-colors"
              title={hint(t('compose:forward'), 'compose.forward')}
            >
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 5l7 7-7 7M4 5l7 7-7 7" />
              </svg>
              {t('compose:forward')}
            </button>
            {aiDraftsEnabled && (
              <button
                onClick={() => void requestAiDraft()}
                disabled={isGeneratingDraft}
                className="flex items-center gap-1.5 px-3 py-1 bg-purple-600 text-white text-sm font-medium rounded hover:bg-purple-700 transition-colors disabled:opacity-60 disabled:cursor-not-allowed"
                title={t('inbox:emailView.aiDraftTitle')}
              >
                {isGeneratingDraft ? (
                  <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-white" />
                ) : (
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 17.657l.707.707M12 21v-1m-3-7a3 3 0 116 0c0 1.657-1.5 2.5-1.5 4h-3c0-1.5-1.5-2.343-1.5-4z"
                    />
                  </svg>
                )}
                {t('inbox:emailView.aiDraft')}
              </button>
            )}
            {onChatAboutThread && (
              <button
                onClick={() => onChatAboutThread(latestEmail)}
                className="p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailRow.chatAboutThread')}
                aria-label={t('inbox:emailRow.chatAboutThread')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                  />
                </svg>
              </button>
            )}
            <button
              onClick={() => (threadSearchOpen ? closeThreadSearch() : openThreadSearch())}
              className={`p-1.5 rounded transition-colors ${
                threadSearchOpen
                  ? 'text-primary-600 bg-primary-50 hover:bg-primary-100'
                  : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'
              }`}
              title={t('inbox:emailView.searchInThread', { shortcut: formatShortcut(api.currentPlatform(), 'F') })}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
                />
              </svg>
            </button>
            {onOpenInTab && (
              <button
                onClick={onOpenInTab}
                className="p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailView.openInNewTab')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                  />
                </svg>
              </button>
            )}
            <ThreadToolbarActions
              threadEmails={threadEmails}
              onArchive={archive}
              onMoveToInbox={() => {
                void moveThreadsToInbox([threadRefOf(latestEmail)]);
                onClose();
              }}
              onMarkUnread={markUnread}
              onMarkRead={() => void setThreadsRead([threadRefOf(latestEmail)], true)}
              onToggleStar={(starred) => void setThreadsStarred([threadRefOf(latestEmail)], starred)}
              snoozed={threadSnoozed}
              onSnooze={(until) => {
                // Out of the inbox until then, like archive.
                leave(() => void snoozeThreads([threadRefOf(latestEmail)], until));
              }}
              onUnsnooze={() => void unsnoozeThreads([threadRefOf(latestEmail)])}
              snoozeSignal={snoozeSignal}
            />
            <button
              onClick={deleteThread}
              className="p-1.5 text-gray-400 hover:text-red-500 hover:bg-red-50 rounded transition-colors disabled:opacity-50"
              title={hint(t('inbox:emailView.deleteThread'), 'thread.delete')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                />
              </svg>
            </button>
            {fullWidth ? (
              <button
                onClick={onClose}
                className="flex items-center gap-1 px-2 py-1 text-sm text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailView.back')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                </svg>
                Back
              </button>
            ) : (
              <button
                onClick={onClose}
                className="p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
                title={t('inbox:emailView.close')}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            )}
          </div>
        </div>
        {isReplyOpen && (
          <div className="min-h-0 overflow-y-auto">
            <ReplyCompose
              email={latestEmail}
              threadEmails={threadEmails}
              accounts={accounts}
              defaultAccountId={activeAccountId || latestEmail.accountId}
              mode={replyMode}
              restoredDraft={replyMode === 'forward' ? null : threadDraft}
              onDraftSaved={setThreadDraft}
              initialBody={replyBody}
              initialAttachments={forwardAttachments}
              isLoadingDraft={isGeneratingDraft}
              draftSources={draftSources}
              onGenerateDraft={aiDraftsEnabled ? (instructions) => void requestAiDraft(instructions) : undefined}
              onCancel={() => {
                setIsReplyOpen(false);
                setThreadDraft(null);
                setReplyBody('');
                setDraftSources([]);
                setIsGeneratingDraft(false);
                draftTrackerRef.current.cancel();
              }}
              onSend={async ({
                fromAccountId,
                toEmails,
                ccEmails,
                body: replyText,
                bodyHtml,
                inlineImages,
                attachments,
                scheduleAt,
                draftId,
              }) => {
                const isForward = replyMode === 'forward';
                // A forward is a NEW message, not a reply: it must not carry
                // In-Reply-To/References, or the recipient's client files it
                // into a conversation they were never part of.
                const message: OutgoingMessage = {
                  accountId: fromAccountId,
                  replyToEmailId: isForward ? null : latestEmail.id,
                  to: toEmails,
                  cc: ccEmails,
                  // A reply takes its parent's subject ("Re: …") backend-side.
                  subject: isForward ? forwardSubject(latestEmail.subject) : '',
                  body: replyText,
                  bodyHtml: bodyHtml ?? null,
                  inlineImages: inlineImages ?? [],
                  attachments: attachments ?? [],
                };
                const closeReply = () => {
                  setThreadDraft(null);
                  setForwardAttachments(EMPTY_ATTACHMENTS);
                  setIsReplyOpen(false);
                };
                const outbox = useOutboxStore.getState();
                if (scheduleAt) {
                  await outbox.schedule(message, scheduleAt, draftId);
                  closeReply();
                  return;
                }
                await outbox.send(message, {
                  // A queued reply's draft leaves Drafts with the queueing.
                  draftId,
                  sendDirect: async () => {
                    addLog(
                      'info',
                      'sync',
                      `${isForward ? 'Forwarding' : 'Sending reply'} to ${toEmails.join(', ')}...`,
                    );
                    if (isForward) {
                      await api.sendNewEmail(
                        fromAccountId,
                        toEmails,
                        ccEmails,
                        message.subject,
                        replyText,
                        attachments,
                        bodyHtml,
                        inlineImages,
                      );
                    } else {
                      await api.sendReply(
                        latestEmail.id,
                        replyText,
                        fromAccountId,
                        toEmails,
                        ccEmails,
                        bodyHtml,
                        inlineImages,
                        attachments,
                      );
                    }
                    // The reply is out: its draft goes now, before the thread
                    // refresh below would find and reopen it.
                    if (draftId) {
                      await api
                        .deleteDraft(draftId, latestEmail.accountId)
                        .catch((err) =>
                          addLog('error', 'sync', `Could not delete the sent reply's draft: ${errorText(err)}`),
                        );
                    }
                    // The backend inserted the optimistic Sent row before the send
                    // command returned (and already enqueued the follow-up account
                    // sync) — refetching the thread shows the reply instantly.
                    await refreshThread(latestEmail.accountId, latestEmail.threadId);
                    bumpSentRefresh();
                    addLog('success', 'sync', `${isForward ? 'Forwarded' : 'Reply sent'} to ${toEmails.join(', ')}`);
                  },
                });
                // Queued or sent, the panel closes; a queued reply refreshes the
                // thread when the dispatcher reports it sent (App listener).
                closeReply();
              }}
            />
          </div>
        )}
      </header>

      <div className="relative flex-1 flex flex-col overflow-hidden">
        {threadSearchOpen && (
          <div className="absolute top-2 right-4 z-20 flex items-center gap-1 px-2 py-1.5 bg-white border border-gray-200 rounded-lg shadow-lg">
            <svg className="w-4 h-4 text-gray-400 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
              />
            </svg>
            <input
              ref={threadSearchInputRef}
              type="text"
              value={threadSearchQuery}
              onChange={(e) => {
                setThreadSearchQuery(e.target.value);
                setThreadMatchIdx(0);
                setOccurrenceCounts({});
              }}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  closeThreadSearch();
                } else if (e.key === 'Enter' && occurrenceSlots.length > 0) {
                  e.preventDefault();
                  setThreadMatchIdx((idx) => stepMatchIndex(idx, e.shiftKey ? -1 : 1, occurrenceSlots.length));
                }
              }}
              placeholder={t('inbox:emailView.searchInThreadPlaceholder')}
              className="w-48 bg-transparent text-sm text-gray-900 placeholder-gray-400 outline-none"
            />
            {threadSearchActive && (
              <span className="text-xs text-gray-400 flex-shrink-0 tabular-nums pr-1">
                {occurrenceSlots.length > 0
                  ? t('inbox:emailView.searchMatchCount', {
                      current: Math.min(threadMatchIdx, occurrenceSlots.length - 1) + 1,
                      total: occurrenceSlots.length,
                    })
                  : t('inbox:emailView.searchNoMatches')}
              </span>
            )}
            <div className="w-px h-4 bg-gray-200" />
            <button
              onClick={() => setThreadMatchIdx((idx) => stepMatchIndex(idx, -1, occurrenceSlots.length))}
              disabled={occurrenceSlots.length === 0}
              className="p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors disabled:opacity-40 disabled:hover:bg-transparent"
              title={t('inbox:emailView.searchPrevMatch')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 15l7-7 7 7" />
              </svg>
            </button>
            <button
              onClick={() => setThreadMatchIdx((idx) => stepMatchIndex(idx, 1, occurrenceSlots.length))}
              disabled={occurrenceSlots.length === 0}
              className="p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors disabled:opacity-40 disabled:hover:bg-transparent"
              title={t('inbox:emailView.searchNextMatch')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>
            <button
              onClick={closeThreadSearch}
              className="p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
              title={t('inbox:emailView.close')}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        )}
        <div className="flex-1 overflow-y-auto">
          {getThreadViewItems(threadEmails, threadExpanded || threadSearchActive, focusEmailId).map((item) => {
            if (item.type === 'collapsed') {
              return (
                <button
                  key="collapsed"
                  type="button"
                  onClick={() => setThreadExpanded(true)}
                  className="w-full px-6 py-3 text-sm text-primary-600 hover:bg-primary-50 border-b border-gray-100 transition-colors text-left"
                >
                  Show {item.count} more message{item.count !== 1 ? 's' : ''}
                </button>
              );
            }

            const { email, index } = item;
            const isLast = index === threadEmails.length - 1;
            const isFocused = focusEmailId === email.id;
            // In-thread search takes over highlighting from the global search
            // while it is active; the email holding the active occurrence
            // drives scroll-into-view.
            const isSearchMatch = threadSearchActive
              ? activeSlot?.emailId === email.id
              : searchHighlightEmailId === email.id;
            // Which occurrence inside THIS email is active. For the global
            // search there is no occurrence navigation — the first occurrence
            // in the matching email is the one scrolled to.
            const searchActiveMatchIndex = threadSearchActive
              ? activeSlot?.emailId === email.id
                ? activeSlot.indexInEmail
                : null
              : isSearchMatch
                ? 0
                : null;
            const isExpanded =
              isLast ||
              isFocused ||
              isSearchMatch ||
              expandedEmails.has(email.id) ||
              (threadSearchActive && threadMatches.includes(email.id));

            return (
              <ThreadEmailItem
                key={email.id}
                email={email}
                isExpanded={isExpanded}
                isLast={isLast}
                isFocused={isFocused}
                isSearchMatch={isSearchMatch}
                highlightQuery={threadSearchActive ? threadSearchQuery : searchQuery}
                searchActiveMatchIndex={searchActiveMatchIndex}
                onSearchMatches={threadSearchActive ? handleSearchMatches : undefined}
                onToggle={() => toggleEmailExpanded(email.id)}
                onOpenAttachment={handleOpenAttachment}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

interface ThreadToolbarActionsProps {
  threadEmails: Email[];
  onArchive: () => void;
  onMoveToInbox: () => void;
  onMarkUnread: () => void;
  onMarkRead: () => void;
  onToggleStar: (starred: boolean) => void;
  /** The conversation is snoozed (opened from the Snoozed view or search). */
  snoozed: boolean;
  onSnooze: (until: number) => void;
  onUnsnooze: () => void;
  /** Opens the snooze picker when it changes (keyboard `b`). */
  snoozeSignal: number;
}

/** Archive (or move back to the inbox), snooze (or unsnooze), read/unread and
 *  star for the open conversation. Failures are reported by the store (toast
 *  + log). */
function ThreadToolbarActions({
  threadEmails,
  onArchive,
  onMoveToInbox,
  onMarkUnread,
  onMarkRead,
  onToggleStar,
  snoozed,
  onSnooze,
  onUnsnooze,
  snoozeSignal,
}: ThreadToolbarActionsProps) {
  const { t } = useTranslation(['inbox']);
  const hint = useShortcutHint();
  const starred = isThreadStarred(threadEmails);
  const unread = isThreadUnread(threadEmails);
  const inInbox = threadEmails.some((e) => e.mailbox === 'inbox');
  const archived = !inInbox && threadEmails.some((e) => e.mailbox === 'archive');
  const buttonClass = 'p-1.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors';
  return (
    <>
      {inInbox && (
        <button
          onClick={onArchive}
          className={buttonClass}
          title={hint(t('inbox:emailView.archive'), 'thread.archive')}
        >
          <ArchiveIcon className="w-4 h-4" />
        </button>
      )}
      {archived && (
        <button onClick={onMoveToInbox} className={buttonClass} title={t('inbox:emailView.moveToInbox')}>
          <InboxIcon className="w-4 h-4" />
        </button>
      )}
      {snoozed ? (
        <button
          data-testid="thread-unsnooze"
          onClick={onUnsnooze}
          className={buttonClass}
          title={t('inbox:snooze.unsnooze')}
          aria-label={t('inbox:snooze.unsnooze')}
        >
          <ClockIcon className="w-4 h-4 text-primary-600" />
        </button>
      ) : (
        inInbox && (
          <SnoozeMenuButton
            testId="thread-snooze"
            onPick={onSnooze}
            className={buttonClass}
            align="right"
            openSignal={snoozeSignal}
            title={hint(t('inbox:snooze.button'), 'thread.snooze')}
          />
        )
      )}
      <button
        onClick={unread ? onMarkRead : onMarkUnread}
        className={buttonClass}
        title={
          unread
            ? hint(t('inbox:emailView.markAsRead'), 'thread.markRead')
            : hint(t('inbox:emailView.markAsUnread'), 'thread.markUnread')
        }
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
          />
        </svg>
      </button>
      <button
        onClick={() => onToggleStar(!starred)}
        className={`p-1.5 rounded transition-colors hover:bg-gray-100 ${
          starred ? 'text-amber-400 hover:text-amber-500' : 'text-gray-400 hover:text-gray-600'
        }`}
        title={hint(starred ? t('inbox:emailView.unstar') : t('inbox:emailView.star'), 'thread.star')}
        aria-pressed={starred}
      >
        <StarIcon filled={starred} className="w-4 h-4" />
      </button>
    </>
  );
}
