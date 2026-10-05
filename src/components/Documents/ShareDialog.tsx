import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

const PRIMARY =
  'rounded bg-primary-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-primary-700 disabled:opacity-50';
const SECONDARY = 'rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700';

/** Pure: the addresses typed in the recipients box (commas, semicolons or new lines). */
export function parseRecipients(text: string): string[] {
  return text
    .split(/[,;\n]/)
    .map((a) => a.trim())
    .filter(Boolean);
}

const SEPARATOR = /[,;\n]/;

/** Pure: the address being typed — what follows the last separator. */
export function lastRecipientToken(text: string): string {
  const parts = text.split(SEPARATOR);
  return (parts[parts.length - 1] ?? '').trim();
}

/** Pure: `text` with the address being typed replaced by `address`, ready for the next one. */
export function withRecipient(text: string, address: string): string {
  const parts = text.split(SEPARATOR);
  parts.pop();
  const kept = parts.map((p) => p.trim()).filter((p) => p && p.toLowerCase() !== address.toLowerCase());
  return `${[...kept, address].join(', ')}, `;
}

interface Suggestion {
  email: string;
  name: string;
  sameOrganization: boolean;
}

interface ShareDialogProps {
  /** The account sharing: suggestions come from its contacts. */
  accountId: string;
  title: string;
  /** Addresses already sharing the document: never suggested again. */
  exclude: string[];
  /** The account the document's mail goes out from. */
  fromAddress: string;
  onShare: (recipients: string[]) => Promise<void>;
  onClose: () => void;
}

/**
 * Sharing is the consent to automatic mail: the dialog names the recipients
 * and the sending account, and Share stays disabled until the user ticks that
 * they understand their changes will be emailed to those addresses.
 */
export function ShareDialog({ accountId, title, exclude, fromAddress, onShare, onClose }: ShareDialogProps) {
  const { t } = useTranslation(['documents']);
  const [text, setText] = useState('');
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const recipients = parseRecipients(text);
  const token = lastRecipientToken(text);
  const [orgDomain, setOrgDomain] = useState<string | null>(null);
  const [colleagues, setColleagues] = useState<Suggestion[]>([]);
  const [matches, setMatches] = useState<Suggestion[]>([]);
  const addLog = useLogStore((st) => st.addLog);

  // People of the sharer's own organization come first: listed before typing,
  // and ranked first among the matches while typing. A free provider such as
  // gmail.com is no organization, so then nobody is singled out.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const domain = await api.getOrganizationDomain(accountId);
        if (cancelled) return;
        setOrgDomain(domain);
        if (!domain) return;
        const page = await api.listContacts(accountId, { domain, kind: 'person', sort: 'last', limit: 6 });
        if (!cancelled)
          setColleagues(page.items.map((c) => ({ email: c.email, name: c.name, sameOrganization: true })));
      } catch (err) {
        // Suggestions are a convenience: typing an address still works without them.
        addLog('error', 'account', `Contact suggestions unavailable: ${errorText(err)}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accountId, addLog]);

  useEffect(() => {
    if (!token) {
      setMatches([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      api
        .autocompleteRecipients(accountId, token, orgDomain ?? undefined, 6)
        .then((found) => {
          if (!cancelled)
            setMatches(found.map((f) => ({ email: f.email, name: f.name, sameOrganization: f.domainMatch })));
        })
        .catch((err) => {
          if (!cancelled) setMatches([]);
          addLog('error', 'account', `Contact suggestions unavailable: ${errorText(err)}`);
        });
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [accountId, token, orgDomain, addLog]);

  const chosen = new Set([...recipients, ...exclude].map((r) => r.toLowerCase()));
  const suggestions = (token ? matches : colleagues).filter((s) => !chosen.has(s.email.toLowerCase()));

  const handleShare = async () => {
    setBusy(true);
    setError(null);
    try {
      await onShare(recipients);
      onClose();
    } catch (err) {
      setError(errorText(err));
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={onClose}
      size="md"
      title={t('documents:shareDialog.title', { title })}
      // The error sits in the fixed header, never below the fold.
      subtitle={
        error ? (
          <span data-testid="share-error" className="text-red-300">
            {error}
          </span>
        ) : (
          t('documents:shareDialog.from', { address: fromAddress })
        )
      }
      footer={
        <>
          <button type="button" onClick={onClose} className={SECONDARY}>
            {t('documents:cancel')}
          </button>
          <button
            type="button"
            data-testid="share-submit"
            disabled={busy || !consent || recipients.length === 0}
            onClick={() => void handleShare()}
            className={PRIMARY}
          >
            {busy ? t('documents:shareDialog.sharing') : t('documents:shareDialog.submit')}
          </button>
        </>
      }
    >
      <label className="block text-sm font-medium text-gray-300" htmlFor="share-recipients">
        {t('documents:shareDialog.recipients')}
      </label>
      <textarea
        id="share-recipients"
        data-testid="share-recipients"
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={2}
        className="mt-1 w-full rounded border border-gray-600 bg-gray-800 px-2 py-1 text-sm text-gray-200"
      />
      {suggestions.length > 0 && (
        <div className="mt-2" data-testid="share-suggestions">
          {!token && <p className="mb-1 text-xs text-gray-400">{t('documents:shareDialog.suggested')}</p>}
          <ul className="flex flex-col gap-1">
            {suggestions.map((s) => (
              <li key={s.email}>
                <button
                  type="button"
                  data-testid={`share-suggestion-${s.email}`}
                  onClick={() => setText((current) => withRecipient(current, s.email))}
                  className="w-full flex items-center gap-2 px-2 py-1 rounded text-left text-sm text-gray-200 hover:bg-gray-700"
                >
                  <span className="truncate">{s.name || s.email}</span>
                  {s.name && <span className="truncate text-xs text-gray-500">{s.email}</span>}
                  {s.sameOrganization && (
                    <span className="ml-auto flex-shrink-0 px-1.5 py-0.5 rounded bg-primary-900/50 text-[10px] text-primary-200">
                      {t('documents:shareDialog.sameOrganization')}
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
      <p className="mt-1 text-xs text-gray-500">{t('documents:shareDialog.recipientsHint')}</p>
      <label className="mt-4 flex items-start gap-2 text-sm text-gray-300">
        <input
          type="checkbox"
          data-testid="share-consent"
          checked={consent}
          onChange={(e) => setConsent(e.target.checked)}
          className="mt-0.5"
        />
        <span>{t('documents:shareDialog.consent')}</span>
      </label>
    </Modal>
  );
}
