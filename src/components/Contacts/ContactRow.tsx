import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import type { Contact } from '@/types';
import { avatarColors } from './utils';

/** Round avatar with first-initial coloured by hash of the email address. */
export function ContactAvatar({ contact, size = 8 }: { contact: Contact; size?: 8 | 10 | 12 }) {
  const initial = (contact.name || contact.email).charAt(0).toUpperCase();
  const { bg, fg } = avatarColors(contact.email);
  const sizeCls = size === 12 ? 'w-12 h-12 text-base' : size === 10 ? 'w-10 h-10 text-sm' : 'w-8 h-8 text-sm';
  return (
    <div className={`${sizeCls} rounded-full ${bg} ${fg} flex items-center justify-center font-medium flex-shrink-0`}>
      {initial}
    </div>
  );
}

/** Small grey pill rendering a derived company name; nothing if absent. */
export function CompanyBadge({ company }: { company?: string | null }) {
  if (!company) return null;
  return (
    <span className="inline-flex items-center px-1.5 py-0.5 text-[11px] rounded bg-gray-100 text-gray-700 font-medium dark:bg-surface-hover dark:text-gray-300">
      {company}
    </span>
  );
}

interface ContactRowProps {
  contact: Contact;
  selected: boolean;
  onClick: () => void;
}

export function ContactRow({ contact, selected, onClick }: ContactRowProps) {
  const fmt = useFormatters();
  const { t } = useTranslation(['contacts']);
  return (
    <button
      type="button"
      onClick={onClick}
      className={`w-full text-left px-6 py-3 flex items-center gap-3 border-l-2 transition-colors ${
        selected
          ? 'bg-primary-50 border-primary-500 dark:bg-primary-900/20'
          : 'border-transparent hover:bg-gray-50 dark:hover:bg-surface-raised'
      }`}
    >
      <ContactAvatar contact={contact} />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-gray-900 truncate text-sm dark:text-gray-100">
            {contact.name && contact.name !== contact.email ? contact.name : contact.email}
          </span>
          <CompanyBadge company={contact.company} />
          {contact.kind === 'automated' && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-500 uppercase tracking-wide dark:bg-surface-hover dark:text-gray-400">
              auto
            </span>
          )}
        </div>
        {contact.name && contact.name !== contact.email && (
          <div className="text-xs text-gray-500 truncate dark:text-gray-400">{contact.email}</div>
        )}
      </div>
      <div className="flex flex-col items-end gap-1 flex-shrink-0">
        <div className="text-xs text-gray-500 flex items-center gap-2 tabular-nums dark:text-gray-400">
          <span title={t('contacts:detail.received')}>↓ {fmt.number(contact.receivedCount ?? 0)}</span>
          <span title={t('contacts:detail.sent')}>↑ {fmt.number(contact.sentCount ?? 0)}</span>
        </div>
        <div className="text-xs text-gray-400 dark:text-gray-500">{fmt.relativeTime(contact.lastTimestamp)}</div>
      </div>
    </button>
  );
}

export function SkeletonRows({ count = 8 }: { count?: number }) {
  return (
    <div className="divide-y divide-gray-100 dark:divide-gray-800">
      {Array.from({ length: count }, (_, i) => `skeleton-${i}`).map((key) => (
        <div key={key} className="px-6 py-3 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-gray-100 animate-pulse dark:bg-surface-hover" />
          <div className="flex-1 space-y-2">
            <div className="h-3 w-1/3 bg-gray-100 rounded animate-pulse dark:bg-surface-hover" />
            <div className="h-2 w-1/2 bg-gray-100 rounded animate-pulse dark:bg-surface-hover" />
          </div>
        </div>
      ))}
    </div>
  );
}
