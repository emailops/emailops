import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import type { ContactDetail } from '@/types';
import { CompanyBadge, ContactAvatar } from './ContactRow';

interface ContactDetailPanelProps {
  detail: ContactDetail;
  onClose: () => void;
  onComposeTo: (addr: string) => void;
  onViewEmailsFrom: (addr: string) => void;
}

export function ContactDetailPanel({ detail, onClose, onComposeTo, onViewEmailsFrom }: ContactDetailPanelProps) {
  const { t } = useTranslation(['contacts']);
  const fmt = useFormatters();
  const c = detail.contact;
  const displayName = c.name && c.name !== c.email ? c.name : c.email;
  return (
    <div className="w-96 flex-shrink-0 border-l border-gray-200 bg-white flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-5 py-3 border-b border-gray-200 flex-shrink-0">
        <span className="text-xs font-medium text-gray-500 uppercase tracking-wide">{t('contacts:detail.title')}</span>
        <button
          type="button"
          onClick={onClose}
          className="text-gray-400 hover:text-gray-600 text-lg leading-none"
          aria-label={t('contacts:detail.closeAria')}
        >
          ×
        </button>
      </div>

      <div className="overflow-y-auto flex-1">
        <div className="px-5 py-5 border-b border-gray-200">
          <div className="flex items-start gap-4">
            <ContactAvatar contact={c} size={12} />
            <div className="flex-1 min-w-0">
              <div className="text-base font-semibold text-gray-900 truncate">{displayName}</div>
              <div className="text-sm text-gray-500 truncate">{c.email}</div>
              <div className="mt-2 flex items-center gap-2 flex-wrap">
                <CompanyBadge company={c.company} />
                {c.kind === 'automated' && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-500 uppercase tracking-wide">
                    {t('contacts:detail.automated')}
                  </span>
                )}
                {c.domain && (
                  <span className="text-[11px] px-1.5 py-0.5 rounded bg-gray-50 text-gray-500">@{c.domain}</span>
                )}
              </div>
            </div>
          </div>

          {detail.aliases.length > 0 && (
            <div className="mt-4">
              <div className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">
                {t('contacts:detail.otherAddresses')}
              </div>
              <ul className="space-y-1">
                {detail.aliases.map((a) => (
                  <li key={a} className="text-sm text-gray-700 truncate">
                    {a}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>

        <div className="px-5 py-4 border-b border-gray-200 grid grid-cols-2 gap-3 text-sm">
          <Stat label={t('contacts:detail.received')} value={fmt.number(c.receivedCount ?? 0)} />
          <Stat label={t('contacts:detail.sent')} value={fmt.number(c.sentCount ?? 0)} />
          <Stat label={t('contacts:detail.total')} value={fmt.number(c.emailCount ?? 0)} />
          <Stat label={t('contacts:detail.lastContact')} value={fmt.relativeTime(c.lastTimestamp)} />
        </div>

        <div className="px-5 py-4 border-b border-gray-200">
          <div className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-2">
            {t('contacts:detail.relationship')}
          </div>
          <ScoreBar score={c.relationshipScore ?? 0} />
        </div>

        <div className="px-5 py-4 space-y-2">
          <button
            type="button"
            onClick={() => onComposeTo(c.email)}
            className="w-full px-3 py-2 text-sm font-medium bg-primary-600 hover:bg-primary-700 text-white rounded-md transition-colors"
          >
            {t('contacts:detail.composeEmail')}
          </button>
          <button
            type="button"
            onClick={() => onViewEmailsFrom(c.email)}
            className="w-full px-3 py-2 text-sm font-medium bg-gray-100 hover:bg-gray-200 text-gray-800 rounded-md transition-colors"
          >
            {t('contacts:detail.viewAllEmails')}
          </button>
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-gray-500">{label}</div>
      <div className="text-sm text-gray-900 font-medium tabular-nums">{value}</div>
    </div>
  );
}

function ScoreBar({ score }: { score: number }) {
  const pct = Math.max(0, Math.min(100, score));
  const tone = pct >= 70 ? 'bg-emerald-500' : pct >= 40 ? 'bg-blue-500' : pct >= 20 ? 'bg-amber-500' : 'bg-gray-300';
  return (
    <div className="flex items-center gap-2">
      <div className="w-16 h-1.5 bg-gray-100 rounded-full overflow-hidden">
        <div className={`h-full ${tone}`} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-xs text-gray-500 tabular-nums w-7 text-right">{Math.round(pct)}</span>
    </div>
  );
}
