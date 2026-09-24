import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TagChips } from '@/components/common/TagChips';
import { AVATAR_PALETTE, hashColorClass } from '@/lib/colors';
import { writeEmailDragPayload } from '@/lib/emailDrag';
import { senderName } from '@/lib/emailFormatting';
import { useAiStore } from '@/stores/aiStore';
import { useTagStore } from '@/stores/tagStore';
import type { Email, EmailCategory } from '@/types';
import { EmailActionsMenu, type RulePrefill, useMoveTargets } from './EmailActionsMenu';

export type { RulePrefill } from './EmailActionsMenu';

// Stable empty reference for the tag selector's missing-key fallback.
// zustand 5 dropped auto-shallow on selector results, so returning `|| []`
// inline produces a new array every render and trips React's
// useSyncExternalStore "getSnapshot should be cached" guard.
const EMPTY_TAGS: readonly string[] = Object.freeze([]);

interface EmailRowProps {
  email: Email;
  isSelected: boolean;
  onClick: () => void;
  onAddSenderFilter?: (senderEmail: string) => void;
  onBlockSender?: (senderEmail: string) => void;
  onCreateAttachmentRule?: (prefill: RulePrefill) => void;
  onCreateClassificationRule?: (prefill: RulePrefill) => void;
  onOpenInTab?: (email: Email) => void;
  /** Open a new chat session seeded with this email's cleaned thread. */
  onChatAboutThread?: (email: Email) => void;
  /** When true, render a single-line, Gmail-style compact row (used in full-width layout). */
  compact?: boolean;
  /** Unified ("All accounts") mode: colored left-edge bar identifying the
   *  email's account. Rendered absolutely so it never changes row height
   *  (the virtualizer depends on stable measured heights). `chip` also names
   *  the account inline — used for unified search results, where rows from
   *  several accounts are mixed and a colour alone is not enough. */
  accountBadge?: { colorClass: string; label: string; chip?: boolean };
}

export function EmailRow({
  email,
  isSelected,
  onClick,
  onAddSenderFilter,
  onBlockSender,
  onCreateAttachmentRule,
  onCreateClassificationRule,
  onOpenInTab,
  onChatAboutThread,
  compact = false,
  accountBadge,
}: EmailRowProps) {
  const { t } = useTranslation(['inbox']);
  const receivedTime = formatReceptionTime(email.timestamp);
  // Hide classification chips when the master AI switch is off — the tags
  // remain in the DB (so toggling AI back on is lossless), but the user has
  // explicitly opted out of seeing AI-derived metadata in the inbox.
  const aiEnabled = useAiStore((s) => s.enabled);
  const storedTags = useTagStore((s) => s.tagsByEmail[email.id] || EMPTY_TAGS);
  // Exclude the company tag from the right-hand chip list — it's already
  // rendered as an uppercase prefix on the subject, so showing it again on
  // the tasks/tags side is just visual noise.
  // Junk chips survive the AI switch. Junk detection is fully deterministic —
  // header authentication, domain comparison, list markers — with no model and
  // no network, so it is not "AI-derived metadata" the user opted out of. More
  // importantly, hiding a phishing warning because someone turned off AI would
  // be a security regression, not a preference.
  const emailTags = aiEnabled
    ? storedTags.filter((t) => t.tagType !== 'company')
    : storedTags.filter((t) => t.tagType === 'junk');
  // Deprioritize rather than hide. The message stays exactly where the server
  // put it — we never move mail — but a flagged row recedes so the eye skips it.
  // Dimming is dropped while the row is selected, so opening a flagged message
  // never leaves the user reading faded text.
  const junkTag = storedTags.find((t) => t.tagType === 'junk');
  // Company tag is rendered as an uppercase chip prefix on the subject so the
  // user can scan which client/vendor a thread belongs to at a glance. We hide
  // it when the value contains '@' — that's the per-address shape produced by
  // `company_label_for` for personal-mail providers (gmail/outlook/yahoo/…),
  // where the address itself isn't a meaningful "company" badge. Only shown
  // when AI is enabled (consistent with the rest of the classification chips).
  const companyRaw = aiEnabled ? storedTags.find((t) => t.tagType === 'company')?.tagValue : undefined;
  const companyTag = companyRaw && !companyRaw.includes('@') ? companyRaw.toUpperCase() : undefined;
  const [copyMessage, setCopyMessage] = useState<string | null>(null);
  const { canMove } = useMoveTargets(email);

  useEffect(() => {
    if (!copyMessage) return;
    const timeoutId = window.setTimeout(() => setCopyMessage(null), 2000);
    return () => window.clearTimeout(timeoutId);
  }, [copyMessage]);

  // Unified-mode account indicator: absolutely positioned left-edge bar so it
  // adds zero height (virtualized rows must keep their measured height stable).
  const accountBar = accountBadge ? (
    <span
      className={`absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r ${accountBadge.colorClass}`}
      title={t('inbox:emailRow.accountTooltip', { email: accountBadge.label })}
      aria-hidden="true"
    />
  ) : null;

  // Inline chip sits on the sender line, which already has a fixed height, so
  // it does not change the measured row height either.
  const accountChip = accountBadge?.chip ? (
    <span
      data-testid="account-chip"
      className="inline-flex items-center gap-1 flex-shrink min-w-0 max-w-[40%] rounded-full px-1.5 text-[11px] bg-gray-100 text-gray-700"
      title={t('inbox:emailRow.accountTooltip', { email: accountBadge.label })}
    >
      <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${accountBadge.colorClass}`} aria-hidden="true" />
      <span className="truncate">{accountBadge.label}</span>
    </span>
  ) : null;

  if (compact) {
    return (
      <div
        role="button"
        tabIndex={0}
        className={`@container group relative hover:z-10 w-full text-left px-4 py-2 border-b border-gray-100 transition-colors cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-500 ${
          isSelected ? 'bg-primary-50/70 shadow-[inset_3px_0_0_0_theme(colors.primary.600)]' : 'hover:bg-gray-50'
        } ${!email.isRead && !isSelected ? 'bg-blue-50/40' : ''} ${junkTag && !isSelected ? 'opacity-55' : ''}`}
        onClick={onClick}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onClick();
          }
        }}
        draggable={canMove}
        onDragStart={(e) =>
          writeEmailDragPayload(e.dataTransfer, {
            emailId: email.id,
            accountId: email.accountId,
            mailbox: email.mailbox,
          })
        }
      >
        {accountBar}
        {/* Reserve a stable min-height so async tag/triage loading doesn't grow
            the row after measureElement has run — same fix as the non-compact
            branch, which prevents virtualizer translateY desync / overlap. */}
        {/* Width priority when the list is narrow (container queries, so the
            docked chat panel counts too): the subject keeps its space longest,
            the snippet gives way first, and the sender column and tag strip
            shrink to make room. */}
        <div className="flex items-center gap-3 min-w-0 min-h-[1.75rem]">
          <span
            className={`flex-shrink-0 w-1.5 h-1.5 rounded-full ${
              !email.isRead ? 'bg-primary-600 ring-2 ring-primary-100' : 'bg-transparent'
            }`}
            aria-hidden="true"
          />
          <Avatar name={email.sender} email={email.senderEmail} size="sm" />
          <span
            className={`text-sm truncate w-24 @2xl:w-28 @4xl:w-44 flex-shrink-0 ${
              email.isRead ? 'text-gray-700' : 'font-semibold text-gray-900'
            }`}
            title={senderName(email)}
          >
            {senderName(email)}
          </span>
          {accountChip}
          {email.category !== 'primary' && <CategoryBadge category={email.category} />}
          <div className="flex-1 min-w-0 flex items-baseline gap-2 text-sm">
            {/* Its own flex item, so truncating the subject never swallows the chip
                and the chip never swallows the subject. */}
            {companyTag && (
              <span className="flex-shrink-0 max-w-[30%] truncate self-center rounded-full font-medium text-[11px] px-1.5 py-0 bg-slate-100 text-slate-700">
                {companyTag}
              </span>
            )}
            <span
              className={`truncate min-w-0 ${email.isRead ? 'text-gray-800' : 'font-semibold text-gray-900'}`}
              title={
                companyTag ? `${companyTag} — ${email.subject || '(No subject)'}` : email.subject || '(No subject)'
              }
            >
              {email.subject || '(No subject)'}
            </span>
            <span className="text-gray-500 truncate min-w-0 flex-1" title={email.snippet}>
              — {email.snippet}
            </span>
          </div>
          {/* Fixed height + nowrap so multi-tag rows can't wrap and grow the
              row past its measured height (virtualizer would lay subsequent
              rows on top of this one until ResizeObserver caught up). Excess
              chips are clipped horizontally — same trade-off Gmail makes. */}
          <div className="hidden @3xl:flex items-center gap-1 flex-shrink-0 h-6 max-w-[20%] @4xl:max-w-[35%] overflow-hidden">
            {email.triageStatus && <TriageBadge status={email.triageStatus} />}
            {emailTags.length > 0 && <TagChips tags={emailTags} compact nowrap />}
          </div>
          <span className="text-xs text-gray-500 flex-shrink-0 w-20 @2xl:w-24 text-right tabular-nums">
            {receivedTime}
          </span>
          <EmailActionsMenu
            email={email}
            onAddSenderFilter={onAddSenderFilter}
            onBlockSender={onBlockSender}
            onCreateAttachmentRule={onCreateAttachmentRule}
            onCreateClassificationRule={onCreateClassificationRule}
            onOpenInTab={onOpenInTab}
            onChatAboutThread={onChatAboutThread}
            onStatus={setCopyMessage}
          />
        </div>
        {copyMessage && <div className="mt-1 text-xs text-gray-500">{copyMessage}</div>}
      </div>
    );
  }

  return (
    <div
      role="button"
      tabIndex={0}
      className={`group relative hover:z-10 w-full text-left px-4 py-3 border-b border-gray-100 transition-colors cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-500 ${
        isSelected ? 'bg-primary-50/70 shadow-[inset_3px_0_0_0_theme(colors.primary.600)]' : 'hover:bg-gray-50'
      } ${!email.isRead && !isSelected ? 'bg-blue-50/50' : ''}`}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onClick();
        }
      }}
      draggable={canMove}
      onDragStart={(e) =>
        writeEmailDragPayload(e.dataTransfer, {
          emailId: email.id,
          accountId: email.accountId,
          mailbox: email.mailbox,
        })
      }
    >
      {accountBar}
      <div className="flex items-start gap-3">
        <Avatar name={email.sender} email={email.senderEmail} size="md" unread={!email.isRead} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className={`text-sm truncate ${email.isRead ? 'text-gray-700' : 'font-semibold text-gray-900'}`}>
              {senderName(email)}
            </span>
            {accountChip}
            {email.category !== 'primary' && <CategoryBadge category={email.category} />}
            <span className="ml-auto text-[11px] text-gray-500 flex-shrink-0 tabular-nums">{receivedTime}</span>
            <EmailActionsMenu
              email={email}
              onAddSenderFilter={onAddSenderFilter}
              onBlockSender={onBlockSender}
              onCreateAttachmentRule={onCreateAttachmentRule}
              onCreateClassificationRule={onCreateClassificationRule}
              onOpenInTab={onOpenInTab}
              onChatAboutThread={onChatAboutThread}
              onStatus={setCopyMessage}
            />
          </div>
          <h3
            className={`text-sm mt-0.5 truncate ${email.isRead ? 'text-gray-700' : 'font-semibold text-gray-900'}`}
            title={companyTag ? `${companyTag} — ${email.subject || '(No subject)'}` : undefined}
          >
            {companyTag && (
              <span className="inline-block rounded-full font-medium text-sm px-2 py-0.5 bg-slate-100 text-slate-700 mr-1.5 align-middle">
                {companyTag}
              </span>
            )}
            {email.subject || '(No subject)'}
          </h3>
          <p className="text-xs text-gray-500 mt-1 line-clamp-2 leading-relaxed">{email.snippet}</p>
          {/* Tag/triage row is always rendered with a *fixed* height (not min-h)
              and nowrap chips, so async tag loading (loadTags in Inbox.tsx) or a
              multi-chip email cannot change the row's measured height and desync
              the virtualizer's translateY offsets — which causes rows to visually
              overlap while the ResizeObserver catches up. Excess chips clip. */}
          <div className="mt-2 flex items-center gap-2 h-5 overflow-hidden">
            {email.triageStatus && <TriageBadge status={email.triageStatus} />}
            {emailTags.length > 0 && <TagChips tags={emailTags} compact nowrap />}
          </div>
        </div>
      </div>
      {copyMessage && <div className="mt-2 text-xs text-gray-500 pl-12">{copyMessage}</div>}
    </div>
  );
}

/** Deterministic color from a seed string so the same sender always renders with
 *  the same avatar color across the app. Shared hash lives in `@/lib/colors`. */
function avatarColor(seed: string): string {
  return hashColorClass(seed, AVATAR_PALETTE);
}

/** Strip leading non-letter chars (e.g. quotes, < ) so initials come from the
 *  actual name. Falls back to "?" for empty/unparseable input. */
function avatarInitial(name: string, fallback: string): string {
  const source = name?.trim() || fallback?.trim() || '';
  const match = source.match(/[\p{L}\p{N}]/u);
  return (match?.[0] ?? '?').toUpperCase();
}

interface AvatarProps {
  name: string;
  email: string;
  size: 'sm' | 'md';
  unread?: boolean;
}

function Avatar({ name, email, size, unread }: AvatarProps) {
  const color = avatarColor(email || name);
  const initial = avatarInitial(name, email);
  const sizeClasses = size === 'md' ? 'w-9 h-9 text-sm' : 'w-6 h-6 text-[11px]';
  return (
    <div
      className={`relative flex-shrink-0 ${sizeClasses} rounded-full ${color} text-white font-semibold flex items-center justify-center select-none`}
      aria-hidden="true"
    >
      <span>{initial}</span>
      {unread && size === 'md' && (
        <span className="absolute -top-0.5 -right-0.5 w-2.5 h-2.5 bg-primary-600 rounded-full ring-2 ring-white" />
      )}
    </div>
  );
}

function CategoryBadge({ category }: { category: EmailCategory }) {
  const config: Record<EmailCategory, { label: string; color: string }> = {
    primary: { label: 'Primary', color: 'bg-blue-100 text-blue-700' },
    social: { label: 'Social', color: 'bg-pink-100 text-pink-700' },
    updates: { label: 'Updates', color: 'bg-yellow-100 text-yellow-700' },
    forums: { label: 'Forums', color: 'bg-purple-100 text-purple-700' },
    promotions: { label: 'Promo', color: 'bg-green-100 text-green-700' },
  };

  const { label, color } = config[category] || config.primary;

  return <span className={`inline-block px-1.5 py-0.5 text-[10px] rounded ${color}`}>{label}</span>;
}

function TriageBadge({ status }: { status: Email['triageStatus'] }) {
  const config = {
    action_needed: { label: 'Action Needed', color: 'bg-red-100 text-red-800' },
    fyi: { label: 'FYI', color: 'bg-yellow-100 text-yellow-800' },
    low_priority: { label: 'Low Priority', color: 'bg-gray-100 text-gray-600' },
  };

  if (!status) return null;
  const { label, color } = config[status];

  return <span className={`inline-block px-2 py-0.5 text-xs rounded-full ${color}`}>{label}</span>;
}

/** Format a unix timestamp (seconds) as HH:MM for today's emails or DD/MM/YYYY for older ones. */
function formatReceptionTime(timestampSec: number): string {
  const d = new Date(timestampSec * 1000);
  const now = new Date();
  const isToday =
    d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  if (isToday) {
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  }
  return `${String(d.getDate()).padStart(2, '0')}/${String(d.getMonth() + 1).padStart(2, '0')}/${d.getFullYear()}`;
}
