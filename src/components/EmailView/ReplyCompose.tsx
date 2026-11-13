import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineError } from '@/components/common/InlineError';
import { AttachMenu } from '@/components/shared/AttachMenu';
import { RichTextEditor } from '@/components/shared/RichTextEditor';
import { Select } from '@/components/shared/Select';
import { SendSplitButton } from '@/components/shared/SendSplitButton';
import { TranslateComposeControl } from '@/components/shared/TranslateComposeControl';
import { useComposerSendKey } from '@/hooks/useComposerSendKey';
import { useComposerSignature } from '@/hooks/useComposerSignature';
import { useDraftFiles } from '@/hooks/useDraftFiles';
import { useFileDrop } from '@/hooks/useFileDrop';
import { useFormatters } from '@/hooks/useFormatters';
import { useSenderOptions } from '@/hooks/useSenderOptions';
import type { DraftSource, EmailAttachment, RecipientSuggestion } from '@/lib/api';
import * as api from '@/lib/api';
import {
  type ComposeDraftState,
  canSend,
  createDebouncedDraftSaver,
  createDraftAutosaver,
  type DebouncedDraftSaver,
  type DraftAutosaver,
  replySubject,
  shouldAutosaveDraft,
} from '@/lib/composeDraft';
import { htmlToPlainText, plainTextToHtml, prepareOutgoingHtml } from '@/lib/composeHtml';
import { mergePendingRecipient } from '@/lib/composeRecipients';
import { eoDocAttachment } from '@/lib/eoDocAttachment';
import { errorText } from '@/lib/errors';
import { findSendWarnings, type SendWarning } from '@/lib/sendWarnings';
import { hasSignature, insertSignature, replaceBodyKeepingSignature, withoutSignature } from '@/lib/signature';
import { useLogStore } from '@/stores/logStore';
import { useTranslationStore } from '@/stores/translationStore';
import type { Account, Draft, Email } from '@/types';
import { AiInstructionBar } from './AiInstructionBar';
import { forwardSubject } from './forward';
import { SendWarningBanner } from './SendWarningBanner';

interface ReplyComposeProps {
  email: Email;
  threadEmails: Email[];
  accounts: Account[];
  defaultAccountId: string;
  onSend: (params: {
    fromAccountId: string;
    toEmails: string[];
    ccEmails: string[];
    bccEmails: string[];
    /** As shown in the panel: "Re: …" or "Fwd: …", possibly edited. */
    subject: string;
    body: string;
    bodyHtml?: string;
    inlineImages?: EmailAttachment[];
    /** Files held only here (EO Docs); the draft holds the rest. */
    attachments?: EmailAttachment[];
    /** The draft holds files that must go with the message: it is queued
     *  from the draft, which adds them. */
    draftHoldsFiles: boolean;
    /** Set when the user picked "Schedule send": the message goes out then. */
    scheduleAt: Date | null;
    /** The saved reply draft this send replaces; the caller deletes it. */
    draftId?: string;
  }) => Promise<void>;
  onCancel: () => void;
  /** A reply draft saved earlier for this thread: the panel opens with its
   *  recipients and text, and keeps saving into the same draft. */
  restoredDraft?: Draft | null;
  /** The draft the chat saved for this reply. The panel takes it over: edits
   *  save into it, and sending or cancelling removes it. */
  adoptDraftId?: string;
  /** Called with the draft after every successful save. */
  onDraftSaved?: (draft: Draft) => void;
  initialBody: string;
  mode: 'reply' | 'reply-all' | 'forward';
  /** Email id → its `Reply-To` addresses, where a reply to it goes instead of
   *  its sender. */
  replyTo?: Record<string, string[]>;
  /** Attachments carried over from the message being forwarded. Pre-attached
   *  rather than offered, because a forward that silently drops the boarding
   *  pass is worse than useless. */
  initialAttachments?: EmailAttachment[];
  /** True while `initialAttachments` are on their way: Send waits for them. */
  attachmentsLoading?: boolean;
  /** True while an AI draft is being generated for this reply. Disables the
   *  textarea and shows a spinner inline so the user knows the draft is
   *  being produced on the AI queue. */
  isLoadingDraft?: boolean;
  /** Past threads the AI used as precedent for the current draft. Rendered as
   *  collapsed cards at the bottom of the reply panel for transparency. */
  draftSources?: DraftSource[];
  /** (Re)generate the AI draft with an optional free-text instruction. When
   *  set (AI drafts enabled, not a forward) an instruction bar sits above the
   *  editor. */
  onGenerateDraft?: (instructions: string) => void;
}

const NO_REPLY_TO: Record<string, string[]> = {};

function getDomain(email: string): string {
  return extractEmail(email).split('@')[1]?.toLowerCase() || '';
}

/** Extract bare email from "Name <email>" or plain email string */
function extractEmail(raw: string): string {
  const match = raw.match(/<([^>]+)>/);
  return (match ? match[1] : raw).trim().toLowerCase();
}

/**
 * Who a plain Reply should be addressed to.
 *
 * Usually the sender. The exception is a message **I** sent — replying to my
 * own last word in a thread means writing to the people I wrote to, not to
 * myself. Using the sender unconditionally addressed those replies back to the
 * user's own mailbox.
 *
 * Falls back to the most recent other party in the thread when my message has
 * no usable recipients. Only a note the user only ever sent to themselves is
 * addressed back to them.
 *
 * `replyTo` maps an email id to its `Reply-To` addresses: a message that has
 * one is answered there, not at its sender.
 */
export function computeReplyRecipients(
  email: Email,
  threadEmails: Email[],
  selfEmails: string[],
  replyTo: Record<string, string[]> = {},
): string[] {
  const self = new Set(selfEmails.map((e) => e.trim().toLowerCase()));
  const sender = extractEmail(email.senderEmail);

  if (!self.has(sender)) return replyTo[email.id]?.length ? replyTo[email.id] : [sender];

  // My own message: reply to whoever it was addressed to.
  const mine = [...email.recipients, ...email.cc].map(extractEmail).filter((r) => r.includes('@') && !self.has(r));
  if (mine.length > 0) return dedupe(mine);

  // Nothing addressable there — use the latest other party in the thread.
  const lastOther = [...threadEmails]
    .sort((a, b) => b.timestamp - a.timestamp)
    .map((m) => extractEmail(m.senderEmail))
    .find((addr) => addr.includes('@') && !self.has(addr));

  if (lastOther) return [lastOther];

  // A note only ever sent to myself goes back to me, as Gmail does: the To is
  // visible before sending, and I may be answering it from another account.
  return dedupe([...email.recipients, ...email.cc].map(extractEmail).filter((r) => r.includes('@')));
}

/**
 * Recipients for Reply All: the senders (or their `Reply-To`) and recipients
 * of every message in the thread, minus the user's own addresses. Only the
 * latest email misses participants from earlier in the thread. A note only
 * ever sent to myself leaves nobody, and goes back to its recipients as Reply
 * does.
 */
export function computeReplyAllRecipients(
  email: Email,
  threadEmails: Email[],
  selfEmails: string[],
  replyTo: Record<string, string[]> = {},
): string[] {
  const all = new Set<string>();
  // The latest email too, in case threadEmails is empty.
  for (const msg of [...threadEmails, email]) {
    for (const answerTo of replyTo[msg.id]?.length ? replyTo[msg.id] : [msg.senderEmail]) {
      all.add(extractEmail(answerTo));
    }
    for (const r of [...msg.recipients, ...msg.cc]) {
      const clean = extractEmail(r);
      if (clean.includes('@')) all.add(clean);
    }
  }
  for (const self of selfEmails) {
    all.delete(self);
  }
  return all.size > 0 ? [...all] : computeReplyRecipients(email, threadEmails, selfEmails, replyTo);
}

type RecipientField = 'to' | 'cc' | 'bcc';

function dedupe(values: string[]): string[] {
  return [...new Set(values)];
}

function detectUnusualRecipients(recipients: string[], selfEmails: string[]): string[] {
  const nonSelf = recipients.filter((r) => !selfEmails.includes(r.toLowerCase()));
  if (nonSelf.length < 2) return [];

  const domains = nonSelf.map(getDomain);
  const domainCounts: Record<string, number> = {};
  for (const d of domains) {
    domainCounts[d] = (domainCounts[d] || 0) + 1;
  }

  const maxCount = Math.max(...Object.values(domainCounts));
  const majorityDomains = new Set(
    Object.entries(domainCounts)
      .filter(([, c]) => c === maxCount)
      .map(([d]) => d),
  );

  return nonSelf.filter((r) => !majorityDomains.has(getDomain(r)));
}

/** The text the user wrote, signature left out: inserting the signature once
 *  it loads is not an edit worth saving a draft for. */
function typedText(html: string): string {
  return htmlToPlainText(withoutSignature(html)).trim();
}

export function ReplyCompose({
  email,
  threadEmails,
  accounts,
  defaultAccountId,
  onSend,
  onCancel,
  adoptDraftId,
  initialBody,
  mode,
  replyTo = NO_REPLY_TO,
  initialAttachments = [],
  attachmentsLoading = false,
  isLoadingDraft = false,
  draftSources = [],
  onGenerateDraft,
  restoredDraft = null,
  onDraftSaved,
}: ReplyComposeProps) {
  const addLog = useLogStore((s) => s.addLog);
  const { t } = useTranslation(['compose', 'common']);
  const selfEmails = accounts.map((a) => a.email.toLowerCase());
  const [fromAccountId, setFromAccountId] = useState(defaultAccountId);
  const senderOptions = useSenderOptions(accounts);
  // Editor holds HTML. AI drafts and the initial empty state are plain text;
  // we wrap them in <p>...</p> so Tiptap renders them as one paragraph each.
  const [bodyHtml, setBodyHtml] = useState<string>(() =>
    restoredDraft ? (restoredDraft.bodyHtml ?? plainTextToHtml(restoredDraft.body)) : plainTextToHtml(initialBody),
  );
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  // Thread-language detection (populated by the reading view when the email
  // was expanded). When the thread's language differs from the user's
  // preferred one, offer to translate the drafted reply into it.
  const threadDetection = useTranslationStore((s) => s.detectedByEmail[email.id]);

  // The From account's signature: below the reply, above a forwarded message;
  // swapped when the account changes.
  const signatureKind = mode === 'forward' ? 'forward' : 'reply';
  const currentSignature = useComposerSignature({
    accountId: fromAccountId,
    kind: signatureKind,
    // A restored reply draft already holds its signature block.
    insertOnOpen: restoredDraft === null,
    setBodyHtml,
  });

  // Sync body when the parent updates initialBody (e.g. AI draft generation
  // replaces the "Generating draft..." placeholder with the actual draft, a
  // forward brings the forwarded message). Only on a change: on mount the body
  // is already set, and a restored draft must not be replaced by the empty
  // template. The signature stays, edits included.
  const appliedInitialBody = useRef(initialBody);
  useEffect(() => {
    if (appliedInitialBody.current === initialBody) return;
    appliedInitialBody.current = initialBody;
    const next = plainTextToHtml(initialBody);
    setBodyHtml((current) =>
      hasSignature(current)
        ? replaceBodyKeepingSignature(current, next, signatureKind)
        : insertSignature(next, currentSignature(), signatureKind),
    );
  }, [initialBody, signatureKind, currentSignature]);

  // Compute initial recipients
  const initialTo = (() => {
    // A forward goes to someone the original had nothing to do with — the whole
    // point is that the user chooses. Prefilling anyone here risks sending
    // someone else's mail back to its own sender.
    if (mode === 'forward') {
      return [];
    }
    if (mode === 'reply') {
      return computeReplyRecipients(email, threadEmails, selfEmails, replyTo);
    }
    return computeReplyAllRecipients(email, threadEmails, selfEmails, replyTo);
  })();

  const [toRecipients, setToRecipients] = useState<string[]>(() => restoredDraft?.toAddresses ?? initialTo);
  const [ccRecipients, setCcRecipients] = useState<string[]>(() => restoredDraft?.ccAddresses ?? []);
  const [showCc, setShowCc] = useState(() => (restoredDraft?.ccAddresses.length ?? 0) > 0);
  const [bccRecipients, setBccRecipients] = useState<string[]>(() => restoredDraft?.bccAddresses ?? []);
  const [showBcc, setShowBcc] = useState(() => (restoredDraft?.bccAddresses?.length ?? 0) > 0);
  // Prefilled as it will go out; the user may edit it. The threading headers
  // still tie a reply to its thread.
  const [subject, setSubject] = useState(
    () => restoredDraft?.subject ?? (mode === 'forward' ? forwardSubject(email.subject) : replySubject(email.subject)),
  );

  // A reply is kept as a draft tied to the email it answers, in that email's
  // account, so reopening the thread brings it back and a later send still
  // goes out as a reply. A forward is a new message: its draft belongs to the
  // sending account and to no thread, and is found in Drafts.
  const isForward = mode === 'forward';
  const draftAccountId = isForward ? fromAccountId : (restoredDraft?.accountId ?? email.accountId);
  // Files are stored for the draft as they are attached, so it keeps them.
  const draftFiles = useDraftFiles(
    draftAccountId,
    restoredDraft?.attachments?.map(({ filePath, filename, mimeType }) => ({ filePath, filename, mimeType })) ?? [],
  );
  const onDraftSavedRef = useRef(onDraftSaved);
  onDraftSavedRef.current = onDraftSaved;
  const autosaverRef = useRef<DraftAutosaver | null>(null);
  if (autosaverRef.current === null) {
    autosaverRef.current = createDraftAutosaver(
      async (req) => {
        const saved = await api.saveDraft(req);
        onDraftSavedRef.current?.(saved);
        return saved;
      },
      (err) => addLog('error', 'sync', `Could not save the reply draft: ${errorText(err)}`),
      restoredDraft?.id,
    );
  }
  const debouncedRef = useRef<DebouncedDraftSaver>(
    createDebouncedDraftSaver((state) => autosaverRef.current?.save(state) ?? Promise.resolve(), 800),
  );
  useEffect(() => {
    if (adoptDraftId) autosaverRef.current?.adopt(adoptDraftId);
  }, [adoptDraftId]);
  // What the panel opened with. Opening Reply prefills the recipients (and an
  // AI draft may fill the body), so only a change from this is the user
  // writing something worth keeping.
  const openedWith = useRef({
    to: toRecipients,
    cc: ccRecipients,
    bcc: bccRecipients,
    subject,
    body: typedText(bodyHtml),
    // A forward's own files arrive after it opens; they are part of what it
    // opened with, not an edit (counted in when they arrive, below).
    files: draftFiles.files.length,
  });
  const sentRef = useRef(false);

  // Reply and Reply All share this panel: moving between them readdresses it,
  // unless the user already chose the recipients.
  const addressedMode = useRef(mode);
  // biome-ignore lint/correctness/useExhaustiveDependencies: on a change of mode only
  useEffect(() => {
    if (addressedMode.current === mode) return;
    addressedMode.current = mode;
    const opened = openedWith.current;
    if (toRecipients.join() !== opened.to.join() || ccRecipients.join() !== opened.cc.join()) return;
    openedWith.current = { ...opened, to: initialTo };
    setToRecipients(initialTo);
  }, [mode]);

  /** The draft as the panel holds it now. */
  const draftState = (): ComposeDraftState => ({
    emailId: isForward ? undefined : (restoredDraft?.emailId ?? email.id),
    accountId: draftAccountId,
    toAddresses: toRecipients,
    ccAddresses: ccRecipients,
    bccAddresses: bccRecipients,
    subject,
    plainBody: htmlToPlainText(bodyHtml),
    bodyHtml,
    attachments: draftFiles.forSave,
    isSending,
    sent: sentRef.current,
  });

  // biome-ignore lint/correctness/useExhaustiveDependencies: draftState reads exactly these
  useEffect(() => {
    if (isLoadingDraft) return;
    const opened = openedWith.current;
    const edited =
      typedText(bodyHtml) !== opened.body ||
      toRecipients.join() !== opened.to.join() ||
      ccRecipients.join() !== opened.cc.join() ||
      bccRecipients.join() !== opened.bcc.join() ||
      subject !== opened.subject ||
      draftFiles.files.length !== opened.files;
    if (!edited) return;
    const state = draftState();
    if (!shouldAutosaveDraft(state)) {
      // Sending / sent: a save still waiting must not resurrect the draft.
      debouncedRef.current.cancel();
      return;
    }
    debouncedRef.current.schedule(state);
  }, [
    isLoadingDraft,
    bodyHtml,
    toRecipients,
    ccRecipients,
    bccRecipients,
    subject,
    draftFiles.forSave,
    draftAccountId,
    isSending,
    email,
    restoredDraft,
  ]);

  // Leaving the thread must not drop the edit still inside the quiet period.
  useEffect(() => {
    const debounced = debouncedRef.current;
    return () => debounced.flushPending();
  }, []);

  /** Stop autosaving and return the id of the draft saved so far, if any. */
  const settleDraft = async (): Promise<string | undefined> => {
    debouncedRef.current.flushPending();
    const id = await autosaverRef.current?.flush();
    debouncedRef.current.cancel();
    return id;
  };

  const handleCancel = async () => {
    sentRef.current = true;
    const draftId = await settleDraft();
    if (draftId) {
      await api
        .deleteDraft(draftId, draftAccountId)
        .catch((err) => addLog('error', 'sync', `Could not delete the reply draft: ${errorText(err)}`));
    }
    onCancel();
  };
  // Files held only here: EO Docs, which are shared rather than attached.
  const [attachments, setAttachments] = useState<EmailAttachment[]>([]);
  // Pre-send warnings awaiting "send anyway"; any edit dismisses them.
  const [sendWarnings, setSendWarnings] = useState<SendWarning[] | null>(null);
  // The schedule time the warnings interrupted, so "send anyway" keeps it.
  const warnedScheduleAt = useRef<Date | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on every edit of the body or attachments
  useEffect(() => {
    setSendWarnings(null);
  }, [bodyHtml, attachments, draftFiles.files]);
  // The forwarded files are fetched and base64-encoded AFTER this panel opens,
  // so they arrive as a prop change rather than as an initial value. Stored
  // once, by identity, for the draft — anything the user attached in the
  // meantime survives.
  const appliedInitialAttachments = useRef<EmailAttachment[]>([]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: once per set of forwarded files
  useEffect(() => {
    if (appliedInitialAttachments.current === initialAttachments || initialAttachments.length === 0) return;
    appliedInitialAttachments.current = initialAttachments;
    openedWith.current = { ...openedWith.current, files: openedWith.current.files + initialAttachments.length };
    draftFiles.addAttachments(initialAttachments);
  }, [initialAttachments]);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Autocomplete state
  const [toInput, setToInput] = useState('');
  const [ccInput, setCcInput] = useState('');
  const [bccInput, setBccInput] = useState('');
  const [suggestions, setSuggestions] = useState<RecipientSuggestion[]>([]);
  const [activeField, setActiveField] = useState<RecipientField | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const suggestionsRef = useRef(0);

  // Unusual recipient detection
  const allRecipients = [...toRecipients, ...ccRecipients, ...bccRecipients];
  const unusualRecipients = detectUnusualRecipients(allRecipients, selfEmails);

  // Context domain for autocomplete boosting
  const contextDomain = (() => {
    const domains = allRecipients.filter((r) => !selfEmails.includes(r.toLowerCase())).map(getDomain);
    if (domains.length === 0) return undefined;
    const counts: Record<string, number> = {};
    for (const d of domains) counts[d] = (counts[d] || 0) + 1;
    return Object.entries(counts).sort((a, b) => b[1] - a[1])[0]?.[0];
  })();

  const fetchSuggestions = useCallback(
    async (prefix: string) => {
      if (prefix.length < 2) {
        setSuggestions([]);
        return;
      }
      const reqId = ++suggestionsRef.current;
      const account = accounts.find((a) => a.id === fromAccountId);
      if (!account) return;
      try {
        const results = await api.autocompleteRecipients(account.id, prefix, contextDomain, 8);
        if (suggestionsRef.current !== reqId) return;
        // Filter out already-added recipients
        const existing = new Set([...toRecipients, ...ccRecipients, ...bccRecipients].map((r) => r.toLowerCase()));
        setSuggestions(results.filter((r) => !existing.has(r.email.toLowerCase())));
        setSelectedIdx(0);
      } catch {
        setSuggestions([]);
      }
    },
    [fromAccountId, accounts, toRecipients, ccRecipients, bccRecipients, contextDomain],
  );

  const recipientFields = {
    to: { list: toRecipients, setList: setToRecipients, setInput: setToInput },
    cc: { list: ccRecipients, setList: setCcRecipients, setInput: setCcInput },
    bcc: { list: bccRecipients, setList: setBccRecipients, setInput: setBccInput },
  };

  const addRecipient = (field: RecipientField, email: string) => {
    const trimmed = email.trim().toLowerCase();
    if (!trimmed.includes('@')) return;
    const { list, setList, setInput } = recipientFields[field];
    if (!list.includes(trimmed)) setList([...list, trimmed]);
    setInput('');
    setSuggestions([]);
  };

  const removeRecipient = (field: RecipientField, email: string) => {
    const { list, setList } = recipientFields[field];
    setList(list.filter((r) => r !== email));
  };

  const handleFiles = (files: FileList | null) => {
    draftFiles.addFiles(files);
    if (fileInputRef.current) fileInputRef.current.value = '';
  };
  // A file dropped anywhere on the composer is attached, not opened.
  const { isOver: fileOver, dropProps } = useFileDrop(handleFiles);

  const handleKeyDown = (field: RecipientField, e: React.KeyboardEvent<HTMLInputElement>, input: string) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIdx((i) => Math.min(i + 1, suggestions.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIdx((i) => Math.max(i - 1, 0));
    } else if ((e.key === 'Enter' || e.key === 'Tab') && suggestions.length > 0) {
      e.preventDefault();
      addRecipient(field, suggestions[selectedIdx].email);
    } else if (e.key === 'Enter' && input.includes('@')) {
      e.preventDefault();
      addRecipient(field, input);
    } else if (e.key === 'Backspace' && input === '') {
      e.preventDefault();
      const { list } = recipientFields[field];
      if (list.length > 0) removeRecipient(field, list[list.length - 1]);
    }
  };

  /** Send (through the undo window when it is on) or, with `scheduleAt`,
   *  schedule the reply. The one send entry point of this composer. */
  const handleSend = async (force = false, scheduleAt: Date | null = null) => {
    const prepared = prepareOutgoingHtml(bodyHtml);
    const plain = prepared.plainText.trim();
    // A valid address still in the input box (typed, not tokenised) is a
    // recipient the user means — it must not be dropped from the send.
    const to = mergePendingRecipient(toRecipients, toInput);
    const cc = mergePendingRecipient(ccRecipients, ccInput);
    const bcc = mergePendingRecipient(bccRecipients, bccInput);
    const ready = canSend({
      to,
      cc,
      bcc,
      subject: '',
      requireSubject: false,
      plainBody: plain,
      inlineImages: prepared.inlineImages.length,
      attachments: attachments.length + draftFiles.files.length,
    });
    if (!ready || attachmentsLoading || draftFiles.isStoring) return;
    // A forward quotes someone else's text, which may well say "attached".
    if (!force && mode !== 'forward') {
      // The signature is not the user's text: "attached" in it is no promise.
      const typed = prepareOutgoingHtml(withoutSignature(bodyHtml)).plainText;
      const warnings = findSendWarnings(typed, attachments.length + draftFiles.files.length);
      if (warnings.length > 0) {
        warnedScheduleAt.current = scheduleAt;
        setSendWarnings(warnings);
        return;
      }
    }
    setSendWarnings(null);
    setSendError(null);
    const draftHoldsFiles = draftFiles.files.length > 0;
    // The files go out from the draft: save it now, edited or not (a forward
    // nobody typed in has not been saved yet).
    if (draftHoldsFiles) debouncedRef.current.schedule(draftState());
    setIsSending(true);
    try {
      const draftId = await settleDraft();
      if (draftHoldsFiles && !draftId) {
        setSendError(t('compose:draftFilesNotSaved'));
        return;
      }
      sentRef.current = true;
      await onSend({
        fromAccountId,
        toEmails: to,
        ccEmails: cc,
        bccEmails: bcc,
        subject,
        body: plain,
        bodyHtml: prepared.bodyHtml,
        inlineImages: prepared.inlineImages,
        attachments,
        draftHoldsFiles,
        scheduleAt,
        draftId,
      });
    } catch (err) {
      sentRef.current = false;
      // A failed send must never be silent: keep the compose open with the
      // user's text and show what went wrong (auth expired, SMTP down, ...).
      setSendError(errorText(err));
    } finally {
      setIsSending(false);
    }
  };

  const handleSendKey = useComposerSendKey(
    () => void handleSend(),
    isSending || isLoadingDraft || attachmentsLoading || draftFiles.isStoring,
  );
  const outgoing = useMemo(() => prepareOutgoingHtml(bodyHtml), [bodyHtml]);
  const readyToSend = canSend({
    to: mergePendingRecipient(toRecipients, toInput),
    cc: mergePendingRecipient(ccRecipients, ccInput),
    bcc: mergePendingRecipient(bccRecipients, bccInput),
    subject: '',
    requireSubject: false,
    plainBody: outgoing.plainText,
    inlineImages: outgoing.inlineImages.length,
    attachments: attachments.length + draftFiles.files.length,
  });

  const renderTokenInput = (
    field: RecipientField,
    label: string,
    recipients: string[],
    input: string,
    setInput: (v: string) => void,
  ) => (
    <div className="flex items-start gap-2 mb-2">
      <span className="text-sm text-gray-500 mt-1.5 w-8 flex-shrink-0">{label}</span>
      <div
        className="flex-1 flex flex-wrap gap-1 items-center border border-gray-300 rounded-lg px-2 py-1.5 bg-white min-h-[36px] focus-within:border-primary-500 focus-within:ring-2 focus-within:ring-primary-100"
        onClick={() => document.getElementById(`${field}-input`)?.focus()}
      >
        {recipients.map((r) => {
          const isUnusual = unusualRecipients.includes(r);
          return (
            <span
              key={r}
              className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium ${
                isUnusual ? 'bg-amber-100 text-amber-800 border border-amber-300' : 'bg-gray-100 text-gray-700'
              }`}
              title={isUnusual ? t('compose:unusualDomain') : r}
            >
              {r}
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  removeRecipient(field, r);
                }}
                className="hover:text-red-500"
              >
                x
              </button>
            </span>
          );
        })}
        <div className="relative flex-1 min-w-[120px]">
          <input
            id={`${field}-input`}
            type="text"
            value={input}
            onChange={(e) => {
              setInput(e.target.value);
              setActiveField(field);
              void fetchSuggestions(e.target.value);
            }}
            onFocus={() => setActiveField(field)}
            onBlur={() => setTimeout(() => setActiveField(null), 200)}
            onKeyDown={(e) => handleKeyDown(field, e, input)}
            className="w-full text-sm outline-none bg-transparent py-0.5"
            placeholder={recipients.length === 0 ? t('compose:recipientsPlaceholder') : ''}
          />
          {activeField === field && suggestions.length > 0 && (
            <div className="absolute top-full left-0 mt-1 w-72 bg-white border border-gray-200 rounded-lg shadow-lg z-50 max-h-48 overflow-y-auto">
              {suggestions.map((s, i) => (
                <button
                  key={s.email}
                  type="button"
                  className={`w-full text-left px-3 py-2 text-sm hover:bg-gray-50 flex items-center gap-2 ${
                    i === selectedIdx ? 'bg-primary-50' : ''
                  }`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    addRecipient(field, s.email);
                  }}
                >
                  <div className="flex-1 min-w-0">
                    <div className="truncate text-gray-900">{s.email}</div>
                    {s.name && <div className="truncate text-xs text-gray-500">{s.name}</div>}
                  </div>
                  {s.domainMatch && (
                    <span className="text-[10px] px-1.5 py-0.5 bg-green-100 text-green-700 rounded flex-shrink-0">
                      {t('compose:sameDomain')}
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );

  return (
    <div
      className={`mt-4 rounded-lg border border-gray-200 bg-gray-50 p-4 ${fileOver ? 'ring-2 ring-primary-500' : ''}`}
      onKeyDownCapture={handleSendKey}
      {...dropProps}
    >
      {/* Pinned while the panel scrolls: with a long draft the Send button and
          what comes after it can be far from what is on screen. */}
      <InlineError message={sendError} className="sticky top-0 z-10 mb-3" />

      {/* From selector */}
      <div className="flex items-center gap-2 mb-2">
        <span className="text-sm text-gray-500 w-8 flex-shrink-0">{t('compose:from')}</span>
        <div className="flex-1">
          <Select
            value={fromAccountId}
            onChange={setFromAccountId}
            options={senderOptions}
            ariaLabel={t('compose:from')}
            fullWidth
          />
        </div>
      </div>

      {/* To field */}
      {renderTokenInput('to', t('compose:to'), toRecipients, toInput, setToInput)}

      {/* Cc / Bcc toggles + fields */}
      {(!showCc || !showBcc) && (
        <div className="flex gap-3 mb-2 ml-10">
          {!showCc && (
            <button
              type="button"
              onClick={() => setShowCc(true)}
              className="text-xs text-primary-500 hover:text-primary-600"
            >
              + {t('compose:cc')}
            </button>
          )}
          {!showBcc && (
            <button
              type="button"
              onClick={() => setShowBcc(true)}
              className="text-xs text-primary-500 hover:text-primary-600"
            >
              + {t('compose:bcc')}
            </button>
          )}
        </div>
      )}
      {showCc && renderTokenInput('cc', t('compose:cc'), ccRecipients, ccInput, setCcInput)}
      {showBcc && renderTokenInput('bcc', t('compose:bcc'), bccRecipients, bccInput, setBccInput)}

      {/* Subject */}
      <div className="flex items-center gap-2 mb-2">
        <span className="w-8 flex-shrink-0" />
        <input
          type="text"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          placeholder={t('compose:subject')}
          aria-label={t('compose:subject')}
          data-testid="reply-subject"
          className="flex-1 border border-gray-300 rounded-lg px-2 py-1.5 text-sm bg-white outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-100"
        />
      </div>

      {/* Unusual recipient warning */}
      {unusualRecipients.length > 0 && (
        <div className="mb-3 ml-10 p-2 bg-amber-50 border border-amber-200 rounded-lg flex items-start gap-2">
          <svg className="w-4 h-4 text-amber-500 flex-shrink-0 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
            <path
              fillRule="evenodd"
              d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z"
              clipRule="evenodd"
            />
          </svg>
          <div className="text-xs text-amber-700">
            <strong>{t('compose:unusualRecipients')}</strong>{' '}
            {t('compose:unusualRecipientsHint', { recipients: unusualRecipients.join(', ') })}
          </div>
        </div>
      )}

      {onGenerateDraft && mode !== 'forward' && (
        <AiInstructionBar
          onGenerate={onGenerateDraft}
          isGenerating={isLoadingDraft}
          hasDraft={
            withoutSignature(bodyHtml)
              .replace(/<[^>]*>/g, '')
              .trim().length > 0
          }
        />
      )}

      {/* Body */}
      <div className="relative">
        <RichTextEditor
          value={bodyHtml}
          onChange={setBodyHtml}
          disabled={isLoadingDraft}
          placeholder={isLoadingDraft ? t('compose:generatingDraft') : t('compose:replyPlaceholder')}
          contentClassName="min-h-[180px] max-h-[40vh] overflow-y-auto"
        />
        {isLoadingDraft && (
          <div className="pointer-events-none absolute inset-0 flex items-start justify-center pt-4">
            <div className="flex items-center gap-2 rounded-full bg-white/90 border border-gray-200 px-3 py-1.5 text-xs text-gray-600 shadow-sm">
              <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-primary-600" />
              <span>{t('compose:aiDraft.generatingLong')}</span>
            </div>
          </div>
        )}
      </div>

      {/* Attachments: the draft's files, then EO Docs */}
      <InlineError message={draftFiles.error} className="mt-2" />
      {draftFiles.files.length + attachments.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {draftFiles.files.map((file, i) => (
            <div
              key={file.filePath}
              className="flex items-center gap-1.5 px-2.5 py-1 bg-gray-100 border border-gray-200 rounded-lg text-xs text-gray-700"
            >
              <span className="max-w-[180px] truncate" title={file.filename ?? undefined}>
                {file.filename}
              </span>
              <button
                type="button"
                onClick={() => draftFiles.remove(i)}
                aria-label={t('compose:removeAttachment', { name: file.filename ?? '' })}
                className="ml-0.5 text-gray-400 hover:text-red-500"
              >
                ×
              </button>
            </div>
          ))}
          {attachments.map((att, i) => (
            <div
              key={att.filename}
              className="flex items-center gap-1.5 px-2.5 py-1 bg-gray-100 border border-gray-200 rounded-lg text-xs text-gray-700"
            >
              <span className="max-w-[180px] truncate">{att.filename}</span>
              <button
                type="button"
                onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))}
                className="ml-0.5 text-gray-400 hover:text-red-500"
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Actions */}
      <div className="mt-3 flex items-center gap-2">
        <AttachMenu
          accountId={fromAccountId}
          disabled={isSending}
          onPickFiles={() => fileInputRef.current?.click()}
          onPickEoDocs={(docs) => setAttachments((prev) => [...prev, ...docs.map(eoDocAttachment)])}
          className="p-2 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded-lg transition-colors disabled:opacity-50"
          title={t('compose:attachFiles')}
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
            />
          </svg>
        </AttachMenu>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => handleFiles(e.target.files)}
        />
        {threadDetection?.needsTranslation && (
          <TranslateComposeControl
            bodyHtml={bodyHtml}
            onApply={setBodyHtml}
            fixedTargetCode={threadDetection.language}
            disabled={isSending || isLoadingDraft}
          />
        )}
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => void handleCancel()}
          disabled={isSending}
          className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-lg hover:bg-gray-100 disabled:opacity-50"
        >
          {t('common:actions.cancel')}
        </button>
        <SendSplitButton
          testId="reply-send"
          label={
            isSending
              ? t('compose:sending')
              : attachmentsLoading || draftFiles.isStoring
                ? t('compose:loadingFiles')
                : mode === 'forward'
                  ? t('compose:forward')
                  : mode === 'reply-all'
                    ? t('compose:sendReplyAll')
                    : t('compose:sendReply')
          }
          onSend={() => void handleSend()}
          onSchedule={(at) => void handleSend(false, at)}
          disabled={isSending || isLoadingDraft || attachmentsLoading || draftFiles.isStoring || !readyToSend}
        />
      </div>

      {sendWarnings && (
        <div className="mt-3">
          <SendWarningBanner
            warnings={sendWarnings}
            onSendAnyway={() => void handleSend(true, warnedScheduleAt.current)}
            onReview={() => setSendWarnings(null)}
          />
        </div>
      )}

      {/* RAG sources — past threads the AI used as precedent. Collapsed by
          default so they don't push the action buttons off-screen; the user
          opens individual cards to audit retrieval quality. */}
      {draftSources.length > 0 && <DraftSourcesPanel sources={draftSources} />}
    </div>
  );
}

function DraftSourcesPanel({ sources }: { sources: DraftSource[] }) {
  const { t } = useTranslation(['compose']);
  return (
    <div className="mt-4 pt-3 border-t border-gray-200">
      <div className="text-xs font-medium text-gray-500 mb-2">
        {t('compose:draftSources', { total: sources.length })}
      </div>
      <div className="flex flex-col gap-1.5">
        {sources.map((src) => (
          <DraftSourceCard key={src.emailId} source={src} />
        ))}
      </div>
    </div>
  );
}

function DraftSourceCard({ source }: { source: DraftSource }) {
  const { t } = useTranslation(['compose', 'common']);
  const fmt = useFormatters();
  const [open, setOpen] = useState(false);
  const dateStr = fmt.date(source.timestamp);
  return (
    <div className="rounded border border-gray-200 bg-white">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center gap-2 px-3 py-2 text-left text-xs hover:bg-gray-50"
      >
        <svg
          className={`w-3 h-3 text-gray-400 flex-shrink-0 transition-transform ${open ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        <span className="flex-1 min-w-0 truncate font-medium text-gray-800">
          {source.subject || t('common:labels.noSubject')}
        </span>
        {source.sentByUser && (
          <span className="flex-shrink-0 px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-primary-50 text-primary-700 border border-primary-100">
            {t('compose:yourReply')}
          </span>
        )}
        <span className="flex-shrink-0 text-gray-400">{dateStr}</span>
      </button>
      {open && (
        <div className="px-3 pb-3 pt-1 text-xs text-gray-600 border-t border-gray-100">
          <div className="text-gray-500 mb-1">
            {source.sender} &lt;{source.senderEmail}&gt;
          </div>
          <div className="whitespace-pre-wrap break-words text-gray-700">{source.snippet}</div>
        </div>
      )}
    </div>
  );
}
