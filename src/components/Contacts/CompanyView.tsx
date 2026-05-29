import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import type { CompanyContactsGroup } from '@/types';
import { ContactRow } from './ContactRow';

interface CompanyViewProps {
  groups: CompanyContactsGroup[];
  selectedAddress: string | null;
  onSelect: (addr: string) => void;
  onFilterToCompany: (company: string) => void;
}

/**
 * Contacts grouped by their derived `company` value, with a per-group header
 * that links back to the flat list filtered to that company.
 */
export function CompanyView({ groups, selectedAddress, onSelect, onFilterToCompany }: CompanyViewProps) {
  const { t } = useTranslation(['contacts']);
  const fmt = useFormatters();
  if (groups.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-sm text-gray-500">
        {t('contacts:view.noGroupings')}
      </div>
    );
  }
  return (
    <div className="divide-y divide-gray-200">
      {groups.map((g) => (
        <div key={g.company ?? '__none__'}>
          <div className="px-6 py-3 bg-gray-50 sticky top-0 flex items-center justify-between border-b border-gray-200">
            <div className="flex items-center gap-3">
              <span className="text-sm font-semibold text-gray-900">
                {g.company ? g.company : <span className="italic text-gray-500">{t('contacts:view.noCompany')}</span>}
              </span>
              <span className="text-xs text-gray-500">
                {t(
                  g.contacts.length === 1
                    ? 'contacts:view.contactsAndEmailsOne'
                    : 'contacts:view.contactsAndEmailsOther',
                  { count: g.contacts.length, emails: fmt.number(g.totalEmails) },
                )}
              </span>
            </div>
            {g.company && (
              <button
                type="button"
                onClick={() => onFilterToCompany(g.company!)}
                className="text-xs text-primary-600 hover:text-primary-700"
              >
                {t('contacts:view.filterToCompany')}
              </button>
            )}
          </div>
          <div className="divide-y divide-gray-100">
            {g.contacts.map((c) => (
              <ContactRow
                key={c.email}
                contact={c}
                selected={c.email === selectedAddress}
                onClick={() => onSelect(c.email)}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
