import React from 'react';
import ReactDOM from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import App from './App';
import { ErrorBoundary } from './components/ErrorBoundary/ErrorBoundary';
import { FALLBACK_LANGUAGE, i18n, initI18n, isSupportedLanguage } from './i18n';
import { getPref, getSystemLocale } from './lib/api';
import './index.css';

// Resolve the initial UI language before mounting React so the very first
// frame uses the correct bundle (no "flash of English"). Chain:
//   1. `ui_language` preference in SQLite (explicit user choice)
//   2. OS locale via the `get_system_locale` Tauri command
//   3. "en" hard fallback
//
// Failures at any step quietly degrade to the next link — the app must boot
// even if the DB or backend command is momentarily unavailable.
async function resolveInitialLanguage(): Promise<string> {
  try {
    const stored = await getPref('ui_language');
    if (isSupportedLanguage(stored)) return stored;
  } catch (err) {
    console.error('[i18n] failed to read ui_language preference', err);
  }
  try {
    const osLocale = await getSystemLocale();
    if (isSupportedLanguage(osLocale)) return osLocale;
  } catch (err) {
    console.error('[i18n] failed to read OS locale', err);
  }
  return FALLBACK_LANGUAGE;
}

async function bootstrap() {
  const initialLanguage = await resolveInitialLanguage();
  await initI18n(initialLanguage);

  const rootEl = document.getElementById('root');
  if (!rootEl) {
    // Fail loud rather than silently — if the root mount point is missing,
    // something is very wrong with index.html.
    throw new Error('Root element #root not found in document');
  }

  ReactDOM.createRoot(rootEl).render(
    <React.StrictMode>
      <I18nextProvider i18n={i18n}>
        <ErrorBoundary>
          <App />
        </ErrorBoundary>
      </I18nextProvider>
    </React.StrictMode>,
  );
}

bootstrap().catch((err) => {
  // Surface bootstrap failures visibly — the React tree never mounted, so a
  // console.error is the only signal we have.
  console.error('[bootstrap] failed to start app', err);
});
