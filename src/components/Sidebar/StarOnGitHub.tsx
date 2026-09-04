import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useTranslation } from 'react-i18next';

/**
 * Where the star link lands. The stargazers page rather than the repo root:
 * it opens on the repository with the star action already in view.
 */
export const REPO_URL = 'https://github.com/emailops/emailops/stargazers';

/**
 * Sidebar "star us on GitHub" link, sitting under the feedback button.
 *
 * EmailOps is downloaded far more often than its repository is visited —
 * people arrive at a release asset from a download page and never see the
 * project itself — so this is the only place in the product that asks. It is
 * deliberately quieter than the feedback button beside it: a plain text link,
 * no filled background, easy to ignore.
 */
export function StarOnGitHub() {
  const { t } = useTranslation(['sidebar']);

  return (
    <button
      type="button"
      onClick={() => {
        // Informational: a browser that refuses to open is not worth an error
        // state in the sidebar, but the rejection must not escape either.
        void openExternal(REPO_URL).catch(() => {});
      }}
      className="w-full mt-2 flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs text-gray-500 hover:text-yellow-400 transition-colors"
    >
      <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.196-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.783-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z"
        />
      </svg>
      <span>{t('sidebar:starOnGitHub')}</span>
    </button>
  );
}
