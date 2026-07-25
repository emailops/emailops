import { listen } from '@tauri-apps/api/event';
import { create } from 'zustand';
import type { EmailTranslatedEvent, LanguageDetectedEvent, TranslationFailedEvent } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

/**
 * Session cache for AI translation artifacts (reading view).
 *
 * Detections and translated bodies deliberately live in memory only — on
 * restart everything is re-detected / re-translated (backend detections are
 * additionally cached process-wide on the Rust side, so re-render churn never
 * re-invokes the model). Compose-draft translations are NOT stored here; they
 * update local editor state in the compose components.
 */

export interface DetectionInfo {
  /** ISO 639-1 code, or "und" when detection failed. */
  language: string;
  needsTranslation: boolean;
}

export interface TranslationInfo {
  text: string;
  /** English name of the target language (e.g. "Spanish"). */
  targetLanguage: string;
  truncated: boolean;
}

export interface TranslationStoreState {
  detectedByEmail: Record<string, DetectionInfo>;
  /** Translated bodies, keyed by emailId (reading view always targets the
   *  user's preferred language). */
  translations: Record<string, TranslationInfo>;
  /** Whether the reading view currently shows the translation for an email. */
  showTranslated: Record<string, boolean>;
  /** In-flight detection requestIds by emailId ('' while the invoke settles). */
  pendingDetect: Record<string, string>;
  /** In-flight translation requestIds by emailId ('' while the invoke settles). */
  pendingTranslate: Record<string, string>;
  errorByEmail: Record<string, string | null>;
}

interface TranslationStore extends TranslationStoreState {
  /** Fire-and-forget lazy detection; no-op when already detected or pending. */
  detect: (emailId: string) => Promise<void>;
  /** Translate to the preferred language; shows the cached result when present. */
  translate: (emailId: string) => Promise<void>;
  toggle: (emailId: string) => void;
}

// ── Pure reducers (unit-tested without React or Tauri) ──────────────────────

export function reduceLanguageDetected(
  state: TranslationStoreState,
  ev: LanguageDetectedEvent,
): Partial<TranslationStoreState> {
  const pendingDetect = { ...state.pendingDetect };
  delete pendingDetect[ev.emailId];
  return {
    detectedByEmail: {
      ...state.detectedByEmail,
      [ev.emailId]: { language: ev.language, needsTranslation: ev.needsTranslation },
    },
    pendingDetect,
  };
}

export function reduceEmailTranslated(
  state: TranslationStoreState,
  ev: EmailTranslatedEvent,
): Partial<TranslationStoreState> | null {
  // Only accept the event we are waiting for — a stale event from an earlier
  // request (e.g. after an error + retry) must not overwrite newer state.
  if (state.pendingTranslate[ev.emailId] !== ev.requestId) return null;
  const pendingTranslate = { ...state.pendingTranslate };
  delete pendingTranslate[ev.emailId];
  return {
    translations: {
      ...state.translations,
      [ev.emailId]: { text: ev.text, targetLanguage: ev.targetLanguage, truncated: ev.truncated },
    },
    showTranslated: { ...state.showTranslated, [ev.emailId]: true },
    pendingTranslate,
    errorByEmail: { ...state.errorByEmail, [ev.emailId]: null },
  };
}

export function reduceTranslationFailed(
  state: TranslationStoreState,
  ev: TranslationFailedEvent,
): Partial<TranslationStoreState> | null {
  // Compose failures carry an empty emailId and are handled by the compose
  // components' own listeners.
  if (!ev.emailId) return null;
  if (state.pendingTranslate[ev.emailId] !== ev.requestId) return null;
  const pendingTranslate = { ...state.pendingTranslate };
  delete pendingTranslate[ev.emailId];
  return {
    pendingTranslate,
    errorByEmail: { ...state.errorByEmail, [ev.emailId]: ev.error },
  };
}

// ── Store ────────────────────────────────────────────────────────────────────

export const useTranslationStore = create<TranslationStore>((set, get) => ({
  detectedByEmail: {},
  translations: {},
  showTranslated: {},
  pendingDetect: {},
  pendingTranslate: {},
  errorByEmail: {},

  detect: async (emailId: string) => {
    const s = get();
    if (emailId in s.detectedByEmail || emailId in s.pendingDetect) return;
    // Reserve the slot before awaiting so a re-render can't double-fire.
    set((st) => ({ pendingDetect: { ...st.pendingDetect, [emailId]: '' } }));
    try {
      const requestId = await api.detectEmailLanguage(emailId);
      set((st) => ({ pendingDetect: { ...st.pendingDetect, [emailId]: requestId } }));
    } catch (err) {
      // AI or translation disabled, or the command failed — no button appears.
      set((st) => {
        const pendingDetect = { ...st.pendingDetect };
        delete pendingDetect[emailId];
        return { pendingDetect };
      });
      useLogStore.getState().addLog('debug', 'ai', `Language detection unavailable: ${errorText(err)}`);
    }
  },

  translate: async (emailId: string) => {
    const s = get();
    if (emailId in s.pendingTranslate) return;
    if (emailId in s.translations) {
      set((st) => ({ showTranslated: { ...st.showTranslated, [emailId]: true } }));
      return;
    }
    set((st) => ({
      pendingTranslate: { ...st.pendingTranslate, [emailId]: '' },
      errorByEmail: { ...st.errorByEmail, [emailId]: null },
    }));
    try {
      const requestId = await api.translateEmail(emailId);
      set((st) => ({ pendingTranslate: { ...st.pendingTranslate, [emailId]: requestId } }));
    } catch (err) {
      const message = errorText(err);
      set((st) => {
        const pendingTranslate = { ...st.pendingTranslate };
        delete pendingTranslate[emailId];
        return { pendingTranslate, errorByEmail: { ...st.errorByEmail, [emailId]: message } };
      });
      useLogStore.getState().addLog('error', 'ai', `Translation failed to start: ${message}`);
    }
  },

  toggle: (emailId: string) => {
    set((st) => ({ showTranslated: { ...st.showTranslated, [emailId]: !st.showTranslated[emailId] } }));
  },
}));

// ── Tauri event wiring (subscribed once from App.tsx) ───────────────────────

let listenersStarted = false;

/**
 * Subscribe the store to the backend translation events. Idempotent; call
 * once at app boot next to the other global listeners.
 */
export function initTranslationListeners(): void {
  if (listenersStarted) return;
  listenersStarted = true;
  const addLog = useLogStore.getState().addLog;

  listen<LanguageDetectedEvent>('language-detected', (event) => {
    useTranslationStore.setState((s) => reduceLanguageDetected(s, event.payload));
  }).catch((err) => addLog('error', 'system', `Failed to subscribe to language-detected: ${errorText(err)}`));

  listen<EmailTranslatedEvent>('email-translated', (event) => {
    useTranslationStore.setState((s) => reduceEmailTranslated(s, event.payload) ?? {});
  }).catch((err) => addLog('error', 'system', `Failed to subscribe to email-translated: ${errorText(err)}`));

  listen<TranslationFailedEvent>('translation-failed', (event) => {
    useTranslationStore.setState((s) => reduceTranslationFailed(s, event.payload) ?? {});
  }).catch((err) => addLog('error', 'system', `Failed to subscribe to translation-failed: ${errorText(err)}`));
}
