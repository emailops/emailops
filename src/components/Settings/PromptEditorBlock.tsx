import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PromptInfo } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';

// ── Reset confirmation modal ───────────────────────────────────────────────

function ResetConfirmModal({
  promptLabel,
  onCancel,
  onConfirm,
}: {
  promptLabel: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation(['common', 'settings']);
  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div className="bg-[#2d2d2e] border border-gray-600 rounded-lg p-5 shadow-xl max-w-sm w-full mx-4">
        <h3 className="text-sm font-semibold text-gray-100 mb-2">{t('settings:promptEditor.resetTitle')}</h3>
        <p className="text-xs text-gray-400 mb-4">{t('settings:promptEditor.resetBody', { label: promptLabel })}</p>
        <div className="flex gap-2 justify-end">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded transition-colors"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            onClick={onConfirm}
            className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-500 transition-colors"
          >
            {t('common:actions.reset')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Variables sidebar ───────────────────────────────────────────────────────

function VariablesPanel({ prompt, onInsert }: { prompt: PromptInfo; onInsert: (placeholder: string) => void }) {
  const { t } = useTranslation(['common', 'settings']);
  if (prompt.variables.length === 0) {
    return <p className="text-xs text-gray-500">{t('settings:promptEditor.noVariables')}</p>;
  }
  return (
    <ul className="space-y-2">
      {prompt.variables.map((v) => (
        <li key={v.name}>
          <button
            type="button"
            onClick={() => onInsert(`{{${v.name}}}`)}
            title={t('settings:promptEditor.insertAtCursor')}
            className="w-full text-left bg-[#1f1f20] border border-gray-700 hover:border-primary-500 rounded px-2 py-1.5 transition-colors group"
          >
            <code className="text-[11px] text-primary-300 group-hover:text-primary-200">{`{{${v.name}}}`}</code>
            <div className="text-[11px] text-gray-500 mt-0.5 leading-snug">{v.description}</div>
          </button>
        </li>
      ))}
    </ul>
  );
}

// ── Block ───────────────────────────────────────────────────────────────────

interface PromptEditorBlockProps {
  promptId: string;
  /** Heading shown above the block (defaults to the prompt's registry label). */
  title?: string;
  /** Optional one-liner shown under the title (defaults to the prompt's registry description). */
  description?: string;
  /** Body height in px — the textarea and variables list both scroll within this. */
  bodyHeightPx?: number;
}

/**
 * Self-contained editor for a single prompt id, designed to be embedded inside
 * a topical Settings panel (Classification, Memory, AI). Owns its own load /
 * draft / save / reset state so callers just drop it in.
 */
export function PromptEditorBlock({ promptId, title, description, bodyHeightPx = 380 }: PromptEditorBlockProps) {
  const { t } = useTranslation(['common', 'settings']);
  // i18next key union truncated in JSX context at ~1,248 total keys; hoisting avoids TS2345
  const variablesHelpSuffix = t('settings:promptEditor.variablesHelpSuffix');
  const [prompt, setPrompt] = useState<PromptInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [resetOpen, setResetOpen] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await api.listPrompts();
      const found = list.find((p) => p.id === promptId) ?? null;
      if (!found) {
        setLoadError(`Prompt "${promptId}" not found`);
        return;
      }
      setPrompt(found);
      setDraft(found.currentTemplate);
      setSaveError(null);
    } catch (e) {
      setLoadError(errorText(e));
    }
  }, [promptId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const isDirty = prompt !== null && draft !== prompt.currentTemplate;

  const handleInsert = useCallback((placeholder: string) => {
    const ta = textareaRef.current;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    setDraft((prev) => prev.slice(0, start) + placeholder + prev.slice(end));
    requestAnimationFrame(() => {
      const next = textareaRef.current;
      if (!next) return;
      next.focus();
      const caret = start + placeholder.length;
      next.setSelectionRange(caret, caret);
    });
  }, []);

  const handleSave = async () => {
    if (!prompt || !isDirty || saving) return;
    setSaving(true);
    setSaveError(null);
    try {
      await api.setPrompt(prompt.id, draft);
      await refresh();
    } catch (e) {
      setSaveError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const handleResetConfirm = async () => {
    if (!prompt) return;
    try {
      await api.resetPrompt(prompt.id);
      await refresh();
    } catch (e) {
      setSaveError(errorText(e));
    } finally {
      setResetOpen(false);
    }
  };

  if (loadError) {
    return (
      <div className="border border-red-800 bg-red-900/20 rounded p-3">
        <p className="text-xs text-red-300">{t('settings:promptEditor.loadFailed', { error: loadError })}</p>
      </div>
    );
  }

  if (!prompt) {
    return (
      <div className="border border-gray-700 rounded p-3">
        <p className="text-xs text-gray-500">{t('settings:promptEditor.loading')}</p>
      </div>
    );
  }

  const heading = title ?? prompt.label;
  const subheading = description ?? prompt.description;

  return (
    <div className="border border-gray-700 rounded-lg bg-[#252526] overflow-hidden">
      {/* Header */}
      <div className="flex items-start justify-between gap-4 px-4 py-3 border-b border-gray-700 bg-[#2a2a2b]">
        <div className="min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h4 className="text-sm font-semibold text-gray-100">{heading}</h4>
            {prompt.isOverridden && (
              <span className="text-[10px] uppercase tracking-wider px-1.5 py-0.5 bg-amber-900/40 border border-amber-700/60 text-amber-300 rounded">
                {t('settings:promptEditor.modified')}
              </span>
            )}
            {prompt.advanced && (
              <span className="text-[10px] uppercase tracking-wider px-1.5 py-0.5 bg-gray-700/60 border border-gray-600 text-gray-300 rounded">
                {t('settings:promptEditor.advanced')}
              </span>
            )}
          </div>
          {subheading && <p className="text-xs text-gray-500 mt-1">{subheading}</p>}
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          {prompt.isOverridden && (
            <button
              type="button"
              onClick={() => setResetOpen(true)}
              className="px-3 py-1.5 text-xs text-gray-300 border border-gray-600 hover:bg-gray-700 hover:text-white rounded transition-colors"
            >
              {t('settings:promptEditor.resetToDefault')}
            </button>
          )}
          <button
            type="button"
            onClick={handleSave}
            disabled={!isDirty || saving}
            className="px-3 py-1.5 text-xs bg-primary-600 text-white rounded hover:bg-primary-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {saving ? t('common:state.saving') : t('common:actions.save')}
          </button>
        </div>
      </div>

      {saveError && (
        <div className="px-4 py-2 border-b border-gray-700 bg-red-900/20">
          <p className="text-xs text-red-300">{t('settings:promptEditor.saveFailed', { error: saveError })}</p>
        </div>
      )}

      {/* Body — explicit height so both panes have a bounded box to scroll within. */}
      <div className="grid grid-cols-[1fr_220px]" style={{ height: `${bodyHeightPx}px` }}>
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          spellCheck={false}
          className="w-full h-full px-4 py-3 bg-[#1e1e1e] text-gray-100 text-[12.5px] leading-relaxed font-mono resize-none focus:outline-none border-r border-gray-700"
        />
        <aside className="h-full overflow-y-auto px-3 py-3 bg-[#1f1f20]">
          <h5 className="text-[11px] uppercase tracking-wider text-gray-500 mb-2">
            {t('settings:promptEditor.variables')}
          </h5>
          <p className="text-[11px] text-gray-500 mb-3 leading-snug">
            {t('settings:promptEditor.variablesHelp')} <code className="text-primary-300">{`{{name}}`}</code>{' '}
            {variablesHelpSuffix}
          </p>
          <VariablesPanel prompt={prompt} onInsert={handleInsert} />
        </aside>
      </div>

      {resetOpen && (
        <ResetConfirmModal
          promptLabel={prompt.label}
          onCancel={() => setResetOpen(false)}
          onConfirm={handleResetConfirm}
        />
      )}
    </div>
  );
}
