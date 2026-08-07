import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ComposeTranslatedEvent, TranslationFailedEvent } from '@/lib/api';
import * as api from '@/lib/api';
import { htmlToPlainText, plainTextToHtml } from '@/lib/composeHtml';
import { errorText } from '@/lib/errors';
import { languageDisplayName } from '@/lib/language';
import { useAiStore } from '@/stores/aiStore';
import { useTranslationEnabledStore } from '@/stores/featureToggleStore';

/** Free-text target cap; mirrors `MAX_TARGET_LANGUAGE_CHARS` on the backend. */
const MAX_TARGET_CHARS = 40;

interface TranslateComposeControlProps {
  /** Current editor HTML — read at click time, applied back via `onApply`. */
  bodyHtml: string;
  onApply: (html: string) => void;
  /** ISO code of a fixed target (reply-in-thread-language mode). When absent
   *  the control opens a free-text target input (new compose). */
  fixedTargetCode?: string;
  disabled?: boolean;
}

/**
 * "Translate to …" button for the compose surfaces. Translation is a
 * plain-text roundtrip (Tiptap HTML → text → model → `plainTextToHtml`), so
 * rich formatting is flattened — "Undo translation" restores the exact
 * pre-translation HTML.
 */
export function TranslateComposeControl({
  bodyHtml,
  onApply,
  fixedTargetCode,
  disabled,
}: TranslateComposeControlProps) {
  const { t, i18n } = useTranslation(['compose']);
  const { enabled: aiEnabled } = useAiStore();
  const { enabled: translationEnabled } = useTranslationEnabledStore();
  const [isTranslating, setIsTranslating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [undoHtml, setUndoHtml] = useState<string | null>(null);
  const [showInput, setShowInput] = useState(false);
  const [target, setTarget] = useState('');
  const requestIdRef = useRef<string | null>(null);
  const targetInputRef = useRef<HTMLInputElement>(null);
  // The parent re-creates `onApply` per render; the long-lived listener below
  // must call the latest one, not the closure it mounted with.
  const onApplyRef = useRef(onApply);
  onApplyRef.current = onApply;

  // Focus the target input when it appears (a11y-friendly alternative to
  // `autoFocus`: only fires in response to the user's explicit click).
  useEffect(() => {
    if (showInput) targetInputRef.current?.focus();
  }, [showInput]);

  useEffect(() => {
    let unTranslated: UnlistenFn | undefined;
    let unFailed: UnlistenFn | undefined;
    void (async () => {
      try {
        unTranslated = await listen<ComposeTranslatedEvent>('compose-translated', (event) => {
          if (event.payload.requestId !== requestIdRef.current) return;
          requestIdRef.current = null;
          setIsTranslating(false);
          setShowInput(false);
          onApplyRef.current(plainTextToHtml(event.payload.text));
        });
        unFailed = await listen<TranslationFailedEvent>('translation-failed', (event) => {
          if (event.payload.requestId !== requestIdRef.current) return;
          requestIdRef.current = null;
          setIsTranslating(false);
          setUndoHtml(null);
          setError(event.payload.error);
        });
      } catch (err) {
        setError(errorText(err));
      }
    })();
    return () => {
      unTranslated?.();
      unFailed?.();
    };
  }, []);

  if (!aiEnabled || !translationEnabled) return null;

  const start = async (targetValue: string) => {
    const trimmed = targetValue.trim();
    const plain = htmlToPlainText(bodyHtml);
    if (!trimmed || !plain.trim() || isTranslating) return;
    setError(null);
    setUndoHtml(bodyHtml);
    setIsTranslating(true);
    try {
      requestIdRef.current = await api.translateComposeText(plain, trimmed);
    } catch (err) {
      setIsTranslating(false);
      setUndoHtml(null);
      setError(errorText(err));
    }
  };

  const undo = () => {
    if (undoHtml === null) return;
    onApply(undoHtml);
    setUndoHtml(null);
  };

  const hasBody = htmlToPlainText(bodyHtml).trim().length > 0;
  const buttonDisabled = disabled || isTranslating || !hasBody;
  const buttonClass =
    'inline-flex items-center gap-1.5 px-3 py-2 text-sm font-medium text-primary-700 bg-primary-50 border border-primary-200 rounded-lg hover:bg-primary-100 disabled:cursor-not-allowed disabled:opacity-50 dark:text-primary-300 dark:bg-primary-900/20 dark:border-primary-800 dark:hover:bg-primary-900/30';

  return (
    <div className="flex items-center gap-2 min-w-0">
      {fixedTargetCode ? (
        <button
          type="button"
          onClick={() => void start(fixedTargetCode)}
          disabled={buttonDisabled}
          className={buttonClass}
        >
          {isTranslating && <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-primary-600" />}
          {isTranslating
            ? t('compose:translate.translating')
            : t('compose:translate.toLanguage', {
                language: languageDisplayName(fixedTargetCode, i18n.language),
              })}
        </button>
      ) : showInput ? (
        <form
          className="flex items-center gap-1.5"
          onSubmit={(e) => {
            e.preventDefault();
            void start(target);
          }}
        >
          <input
            ref={targetInputRef}
            type="text"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            maxLength={MAX_TARGET_CHARS}
            placeholder={t('compose:translate.placeholder')}
            className="w-44 text-sm border border-gray-300 rounded-lg px-2.5 py-1.5 bg-white focus:border-primary-500 outline-none dark:border-gray-600 dark:bg-surface"
            onKeyDown={(e) => {
              if (e.key === 'Escape') setShowInput(false);
            }}
          />
          <button type="submit" disabled={isTranslating || !target.trim()} className={buttonClass}>
            {isTranslating && <div className="h-3 w-3 animate-spin rounded-full border-b-2 border-primary-600" />}
            {isTranslating ? t('compose:translate.translating') : t('compose:translate.apply')}
          </button>
        </form>
      ) : (
        <button type="button" onClick={() => setShowInput(true)} disabled={buttonDisabled} className={buttonClass}>
          {t('compose:translate.open')}
        </button>
      )}
      {undoHtml !== null && !isTranslating && (
        <button
          type="button"
          onClick={undo}
          className="text-xs font-medium text-gray-500 hover:text-gray-700 whitespace-nowrap dark:text-gray-400 dark:hover:text-gray-300"
        >
          {t('compose:translate.undo')}
        </button>
      )}
      {error && (
        <span className="text-xs text-red-600 truncate dark:text-red-400" title={error}>
          {t('compose:translate.failed', { error })}
        </span>
      )}
    </div>
  );
}
