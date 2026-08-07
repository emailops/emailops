import { useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useEmailStore } from '@/stores/emailStore';
import { isFlagged, useJunkStore } from '@/stores/junkStore';
import { useLogStore } from '@/stores/logStore';
import { useToastStore } from '@/stores/toastStore';
import type { JunkKind, JunkReason } from '@/types';

interface JunkBannerProps {
  emailId: string;
  accountId: string;
}

/**
 * Reason codes worth showing. The long tail is noise in a banner.
 *
 * Typed as a literal union so the i18n key check is real: adding a code here
 * without a translation is a compile error, not a `junk.reason.x` string leaking
 * into the UI.
 */
const HEADLINE_REASONS = [
  'display_name_impersonation',
  'lookalike_domain',
  'punycode_domain',
  'reply_to_mismatch',
  'dmarc_fail',
  'spf_fail',
  'dangerous_attachment',
  'credential_solicitation',
  'display_name_contains_address',
  'server_spam_flag',
  'mixed_script_display_name',
] as const;

type HeadlineReason = (typeof HEADLINE_REASONS)[number];

function isHeadlineReason(code: string): code is HeadlineReason {
  return (HEADLINE_REASONS as readonly string[]).includes(code);
}

const STYLES: Record<JunkKind, { wrap: string; button: string }> = {
  // Phishing is the strongest claim the app makes about anything, so it gets
  // the only red banner in the reading pane.
  phishing: {
    wrap: 'bg-red-50 border-red-300 text-red-900 dark:bg-red-900/20 dark:text-red-200',
    button: 'text-red-900 dark:text-red-200',
  },
  spam: {
    wrap: 'bg-orange-50 border-orange-200 text-orange-900 dark:bg-orange-900/20 dark:border-orange-800 dark:text-orange-200',
    button: 'text-orange-900 dark:text-orange-200',
  },
  graymail: {
    wrap: 'bg-slate-50 border-slate-200 text-slate-700',
    button: 'text-slate-700',
  },
  legit: { wrap: '', button: '' },
};

function topReasons(reasons: JunkReason[]): HeadlineReason[] {
  const seen = new Set<string>();
  const out: HeadlineReason[] = [];
  for (const r of reasons) {
    if (!isHeadlineReason(r.code) || seen.has(r.code)) continue;
    seen.add(r.code);
    out.push(r.code);
    if (out.length === 3) break;
  }
  return out;
}

/**
 * Warning shown above a flagged message, with the reasons behind it and a way
 * to disagree.
 *
 * Renders nothing when the message is unscored or clean — a verdict of
 * `unknown` (no captured headers) is silence, never reassurance.
 */
export function JunkBanner({ emailId, accountId }: JunkBannerProps) {
  const { t } = useTranslation(['inbox']);
  const verdict = useJunkStore((s) => s.verdictsByEmail[emailId]);
  const loadVerdicts = useJunkStore((s) => s.loadVerdicts);
  const setFeedback = useJunkStore((s) => s.setFeedback);
  const addLog = useLogStore((s) => s.addLog);
  const selectEmail = useEmailStore((s) => s.selectEmail);
  const addToast = useToastStore((s) => s.addToast);

  useEffect(() => {
    void loadVerdicts([emailId]);
  }, [emailId, loadVerdicts]);

  // Same error contract as "Confirm junk" below. The store writes optimistically
  // and rolls back on failure, but a rollback the user cannot see just looks
  // like the button did nothing — so the failure has to be said out loud.
  const handleNotJunk = useCallback(async () => {
    try {
      await setFeedback(accountId, emailId, false);
    } catch (err) {
      const message = `${t('inbox:junk.notJunkFailed')}: ${errorText(err)}`;
      addLog('error', 'system', message);
      addToast({ message, sticky: true });
    }
  }, [accountId, emailId, setFeedback, addLog, addToast, t]);

  // Confirming files the message in the server's Junk folder where the provider
  // supports it, so the server's own filter learns too — the detector never
  // moves mail on its own, but a move the user asked for is a different thing.
  //
  // Then it closes the message: having said "yes, this is junk", the user does
  // not want to keep looking at it. Without this the banner simply stayed put
  // and the click appeared to do nothing.
  const handleConfirmJunk = useCallback(async () => {
    try {
      const filed = await api.reportJunkToProvider(accountId, emailId);
      // The local override is recorded before the server is contacted, so an
      // account with no Junk folder still trains the model.
      const message = filed ? t('inbox:junk.confirmedAndFiled') : t('inbox:junk.confirmedLocally');
      addLog('success', 'system', message);
      // A toast, not just a log line: the log panel is usually closed, and an
      // action that silently succeeds is indistinguishable from one that did
      // nothing at all.
      addToast({ message });
      await selectEmail(null);
    } catch (err) {
      // Never swallow this. The override is already stored, but the user asked
      // for something that did not happen and has to be told — loudly enough to
      // notice without opening the log panel.
      const message = `${t('inbox:junk.confirmFailed')}: ${errorText(err)}`;
      addLog('error', 'system', message);
      addToast({ message, sticky: true });
    }
  }, [accountId, emailId, addLog, addToast, selectEmail, t]);

  if (!isFlagged(verdict) || !verdict) return null;

  const kind = verdict.primaryKind;
  const style = STYLES[kind];
  const reasons = topReasons(verdict.reasons);

  return (
    <div className={`px-4 py-2 border-b text-xs flex-shrink-0 ${style.wrap}`}>
      <div className="flex items-center gap-2 flex-wrap">
        <svg className="w-3.5 h-3.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M12 9v2m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
        <span className="font-medium">{t(`inbox:junk.kind.${kind}` as const)}</span>
        <button
          type="button"
          onClick={() => void handleConfirmJunk()}
          className={`ml-auto underline hover:no-underline font-medium ${style.button}`}
          title={t('inbox:junk.isJunkHint')}
        >
          {t('inbox:junk.isJunk')}
        </button>
        <button
          type="button"
          onClick={() => void handleNotJunk()}
          className={`underline hover:no-underline font-medium ${style.button}`}
        >
          {t('inbox:junk.notJunk')}
        </button>
      </div>
      {reasons.length > 0 && (
        <ul className="mt-1 ml-5 list-disc opacity-90">
          {reasons.map((code) => (
            <li key={code}>{t(`inbox:junk.reason.${code}` as const)}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
