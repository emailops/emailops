import { open as openExternal } from '@tauri-apps/plugin-shell';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import ReactMarkdown from 'react-markdown';
import { FALLBACK_LANGUAGE, isSupportedLanguage } from '@/i18n';
import * as api from '@/lib/api';
import { getSafeExternalUrl } from '@/lib/emailFormatting';
import { errorText } from '@/lib/errors';
import {
  PREF_RELEASE_NOTES_SEEN_VERSION,
  planReleaseNotes,
  RELEASE_NOTES,
  releaseNoteMarkdown,
} from '@/lib/releaseNotes';
import { useLogStore } from '@/stores/logStore';
import { Modal } from './Modal';

interface ReleaseNotesDialogProps {
  /** `App`'s onboarding gate: null while unknown, false on a fresh install
   *  (the wizard is showing), true otherwise. */
  onboardingCompleted: boolean | null;
}

interface PendingNotes {
  current: string;
  versions: string[];
}

/** Links in the notes open in the external browser, never in the webview. */
function NoteLink({ href, children }: { href?: string; children?: ReactNode }) {
  const addLog = useLogStore((s) => s.addLog);
  const safeUrl = href ? getSafeExternalUrl(href) : null;
  if (!safeUrl) return <>{children}</>;
  return (
    <button
      type="button"
      className="text-primary-300 underline hover:text-primary-200"
      onClick={() => {
        void openExternal(safeUrl).catch((err) => {
          addLog('error', 'system', `Failed to open link: ${errorText(err)}`);
        });
      }}
    >
      {children}
    </button>
  );
}

const MARKDOWN_COMPONENTS = {
  h3: ({ children }: { children?: ReactNode }) => (
    <h4 className="mb-1.5 mt-4 text-xs font-semibold uppercase tracking-wide text-gray-400 first:mt-0">{children}</h4>
  ),
  p: ({ children }: { children?: ReactNode }) => <p className="mb-2 text-sm text-gray-200">{children}</p>,
  ul: ({ children }: { children?: ReactNode }) => (
    <ul className="mb-2 list-disc space-y-1 pl-5 text-sm text-gray-200">{children}</ul>
  ),
  strong: ({ children }: { children?: ReactNode }) => (
    <strong className="font-semibold text-gray-100">{children}</strong>
  ),
  code: ({ children }: { children?: ReactNode }) => (
    <code className="rounded bg-gray-800 px-1 py-0.5 font-mono text-xs text-gray-100">{children}</code>
  ),
  a: NoteLink,
};

/**
 * "What's new" dialog: on the first launch after an update, shows the bundled
 * release notes (`src/releaseNotes/<version>/<lang>.md`) of every version
 * since the last one seen, in the UI language. Closing it records the running
 * version in the `release_notes_seen_version` pref so it shows only once.
 */
export function ReleaseNotesDialog({ onboardingCompleted }: ReleaseNotesDialogProps) {
  const { t, i18n } = useTranslation(['notifications']);
  const addLog = useLogStore((s) => s.addLog);
  const [pending, setPending] = useState<PendingNotes | null>(null);
  const decided = useRef(false);

  // Decide once per launch, as soon as the onboarding gate has resolved: the
  // wizard finishing later must not turn a fresh install into an "upgrade".
  useEffect(() => {
    if (onboardingCompleted === null || decided.current) return;
    decided.current = true;
    void (async () => {
      try {
        const [lastSeen, build] = await Promise.all([api.getPref(PREF_RELEASE_NOTES_SEEN_VERSION), api.getBuildInfo()]);
        const plan = planReleaseNotes({
          bundle: RELEASE_NOTES,
          freshInstall: !onboardingCompleted,
          lastSeen,
          current: build.version,
        });
        if (plan.versions.length > 0) {
          setPending({ current: build.version, versions: plan.versions });
        } else if (plan.markSeen) {
          await api.setPref(PREF_RELEASE_NOTES_SEEN_VERSION, build.version);
        }
      } catch (err) {
        addLog('error', 'system', `Failed to load release notes: ${errorText(err)}`);
      }
    })();
  }, [onboardingCompleted, addLog]);

  const close = useCallback(() => {
    if (!pending) return;
    setPending(null);
    api.setPref(PREF_RELEASE_NOTES_SEEN_VERSION, pending.current).catch((err) => {
      addLog('error', 'system', `Failed to save release notes as seen: ${errorText(err)}`);
    });
  }, [pending, addLog]);

  if (!pending) return null;
  const language = isSupportedLanguage(i18n.language) ? i18n.language : FALLBACK_LANGUAGE;
  const showVersionHeadings = pending.versions.length > 1;

  return (
    <Modal
      open
      onClose={close}
      title={t('notifications:releaseNotes.title', { version: pending.current })}
      size="xl"
      zIndex={60}
      footer={
        <button
          type="button"
          data-testid="release-notes-close"
          onClick={close}
          className="rounded bg-primary-600 px-4 py-1.5 text-sm font-medium text-white transition-colors hover:bg-primary-500"
        >
          {t('notifications:releaseNotes.close')}
        </button>
      }
    >
      <div data-testid="release-notes">
        {pending.versions.map((version) => (
          <section key={version} className="mb-4 last:mb-0">
            {showVersionHeadings && (
              <h3 className="mb-2 text-sm font-semibold text-gray-100">
                {t('notifications:releaseNotes.version', { version })}
              </h3>
            )}
            <ReactMarkdown components={MARKDOWN_COMPONENTS}>
              {releaseNoteMarkdown(RELEASE_NOTES, version, language)}
            </ReactMarkdown>
          </section>
        ))}
      </div>
    </Modal>
  );
}
