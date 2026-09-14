import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EmailActionsMenu, type EmailActionsMenuProps } from '@/components/Inbox/EmailActionsMenu';
import { useFormatters } from '@/hooks/useFormatters';
import { senderTextColorClass } from '@/lib/colors';
import { senderLabel } from '@/lib/tagBoard';
import type { Email } from '@/types';

export interface TagEmailCardProps {
  email: Email;
  /** Everyone else in the thread, most recently active first. Empty for a
   *  one-to-one message where the sender line already says it all. */
  participants?: string[];
  /** Address of the account this block belongs to, so the user's own messages
   *  read as "Me" rather than their own address. */
  ownerEmail: string;
  isSelected: boolean;
  /** Per-account stripe, shown only in the unified ("All accounts") view.
   *  Same colour source as the unified inbox's row indicator. */
  accountBadge?: { colorClass: string; label: string };
  onSelect: (email: Email) => void;
  /** The inbox row's ⋮ actions, offered here too. */
  actions?: Omit<EmailActionsMenuProps, 'email' | 'onStatus'>;
}

/** One thread inside a tag column. Denser than `EmailRow` — a board column is
 *  ~17rem wide — but not smaller-typed: everything here is body text on a
 *  white card, so sizes and contrast match the inbox rather than shrinking to
 *  fit more rows. */
export function TagEmailCard({
  email,
  participants = [],
  ownerEmail,
  isSelected,
  accountBadge,
  onSelect,
  actions,
}: TagEmailCardProps) {
  const { t } = useTranslation(['tagboard']);
  const [status, setStatus] = useState<string | null>(null);
  const fmt = useFormatters();
  const senderColor = senderTextColorClass(email.senderEmail || email.sender);
  const from = senderLabel(email, ownerEmail, t('tagboard:you'));
  // The sender leads the line, so don't repeat them among the others.
  const others = participants.filter((p) => p !== email.sender && p !== email.senderEmail);

  return (
    // A div, not a <button>: the ⋮ menu is itself a button and buttons cannot
    // nest. Keyboard activation is restored by hand below.
    <div
      role="button"
      tabIndex={0}
      onClick={() => onSelect(email)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onSelect(email);
        }
      }}
      aria-current={isSelected}
      className={`relative block w-full flex-shrink-0 cursor-pointer rounded-lg border py-2.5 pr-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary-500 ${
        accountBadge ? 'pl-4' : 'pl-3'
      } ${
        isSelected
          ? 'border-primary-400 bg-primary-50'
          : 'border-gray-200/70 bg-white hover:border-gray-300 hover:bg-gray-50'
      }`}
    >
      {accountBadge && (
        <span
          className={`absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r ${accountBadge.colorClass}`}
          title={accountBadge.label}
        />
      )}

      <div className="flex items-baseline gap-2">
        {/* Sender and the rest of the thread share one line, comma-separated
            and truncated as a whole. One colour for the whole line — it names
            one conversation — with only weight separating the sender. */}
        <span className="flex min-w-0 flex-1 items-baseline overflow-hidden text-sm">
          <span
            className={`flex-shrink-0 ${senderColor} ${email.isRead ? 'font-medium' : 'font-semibold'}`}
            title={`${email.sender} <${email.senderEmail}>`}
          >
            {from}
          </span>
          {others.length > 0 && (
            <span className={`min-w-0 truncate ${senderColor} font-normal`} title={others.join(', ')}>
              {`, ${others.join(', ')}`}
            </span>
          )}
        </span>
        <span className="flex-shrink-0 text-xs text-gray-600">{fmt.relativeTime(email.timestamp)}</span>
        {actions && (
          <span className="-my-1 flex-shrink-0 self-center">
            <EmailActionsMenu email={email} onStatus={setStatus} {...actions} />
          </span>
        )}
      </div>

      <div
        className={`mt-1 truncate text-[15px] leading-snug ${
          email.isRead ? 'text-gray-800' : 'font-semibold text-gray-900'
        }`}
        title={email.subject}
      >
        {email.subject || '—'}
      </div>

      {email.snippet && (
        <div className="mt-1 line-clamp-2 text-[13px] leading-relaxed text-gray-600">{email.snippet}</div>
      )}
      {status && <div className="mt-1 text-xs text-gray-500">{status}</div>}
    </div>
  );
}
