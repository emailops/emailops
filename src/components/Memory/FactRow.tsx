import { format } from 'date-fns';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMemoryStore } from '@/stores/memoryStore';
import type { MemoryFact } from '@/types';

interface FactRowProps {
  fact: MemoryFact;
  selected: boolean;
  onSelect: (factId: string) => void;
}

/**
 * One memory fact rendered as a clickable row with inline-editable text and
 * promote/retire/delete actions. Selection state is owned by the parent so the
 * email-preview pane can react to it.
 */
export function FactRow({ fact, selected, onSelect }: FactRowProps) {
  const { t } = useTranslation(['memory']);
  const { promoteFact, retireFact, updateFact, deleteFact } = useMemoryStore();
  const [isEditing, setIsEditing] = useState(false);
  const [draft, setDraft] = useState(fact.fact);
  const [busy, setBusy] = useState(false);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch {
      // Error surfaced via store.error.
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    const trimmed = draft.trim();
    if (!trimmed || trimmed === fact.fact) {
      setIsEditing(false);
      setDraft(fact.fact);
      return;
    }
    await run(() => updateFact(fact.id, trimmed));
    setIsEditing(false);
  };

  return (
    <li
      onClick={() => onSelect(fact.id)}
      className={`p-3 border rounded-md cursor-pointer transition-colors ${
        selected ? 'border-primary-400 bg-primary-50' : 'border-gray-200 hover:bg-gray-50'
      }`}
    >
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          {isEditing ? (
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              rows={2}
              className="w-full px-2 py-1 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-1 focus:ring-primary-500"
              // biome-ignore lint/a11y/noAutofocus: edit-mode input expected to receive focus
              autoFocus
            />
          ) : (
            <div className="text-sm text-gray-900 whitespace-pre-wrap break-words">{fact.fact}</div>
          )}
          <div className="flex flex-wrap items-center gap-2 mt-1.5 text-xs text-gray-500">
            <span className="font-mono text-[11px] text-gray-600">{fact.subjectKey}</span>
            <StatusPill status={fact.status} />
            {fact.company && (
              <span
                className="px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase bg-indigo-100 text-indigo-800"
                title={t('memory:fact.companyTitle')}
              >
                {fact.company}
              </span>
            )}
            {fact.domain && <ClassificationPill kind="domain" value={fact.domain} />}
            {fact.vigency && <ClassificationPill kind="vigency" value={fact.vigency} />}
            <span>
              score {fact.score.toFixed(2)}
              {' · '}
              confidence {fact.confidence.toFixed(2)}
            </span>
            <span>· {format(new Date(fact.createdAt * 1000), 'MMM d, yyyy')}</span>
            {fact.source !== 'extraction' && (
              <span className="px-1.5 py-0.5 rounded bg-gray-100 text-gray-600 uppercase text-[10px]">
                {fact.source}
              </span>
            )}
          </div>
        </div>
        <div className="flex gap-1 flex-shrink-0">
          {isEditing ? (
            <>
              <button
                type="button"
                onClick={() => void save()}
                disabled={busy}
                className="text-xs text-primary-600 hover:text-primary-800 px-2 py-1 disabled:opacity-50"
              >
                Save
              </button>
              <button
                type="button"
                onClick={() => {
                  setIsEditing(false);
                  setDraft(fact.fact);
                }}
                disabled={busy}
                className="text-xs text-gray-500 hover:text-gray-700 px-2 py-1 disabled:opacity-50"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              {fact.status === 'candidate' && (
                <button
                  type="button"
                  onClick={() => void run(() => promoteFact(fact.id))}
                  disabled={busy}
                  className="text-xs text-primary-600 hover:text-primary-800 px-2 py-1 disabled:opacity-50"
                >
                  Promote
                </button>
              )}
              <button
                type="button"
                onClick={() => setIsEditing(true)}
                disabled={busy}
                className="text-xs text-gray-500 hover:text-gray-700 px-2 py-1 disabled:opacity-50"
              >
                Edit
              </button>
              {fact.status !== 'retired' && (
                <button
                  type="button"
                  onClick={() => void run(() => retireFact(fact.id))}
                  disabled={busy}
                  className="text-xs text-gray-500 hover:text-gray-700 px-2 py-1 disabled:opacity-50"
                >
                  Retire
                </button>
              )}
              <button
                type="button"
                onClick={() => void run(() => deleteFact(fact.id))}
                disabled={busy}
                className="text-xs text-red-600 hover:text-red-800 px-2 py-1 disabled:opacity-50"
              >
                Delete
              </button>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

function StatusPill({ status }: { status: string }) {
  const classes =
    status === 'promoted'
      ? 'bg-green-100 text-green-800'
      : status === 'retired'
        ? 'bg-gray-100 text-gray-500'
        : 'bg-yellow-100 text-yellow-800';
  return (
    <span className={`px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase ${classes}`}>
      {status === 'promoted' ? 'consolidated' : status}
    </span>
  );
}

// Small badge for the extractor-assigned classification of a fact. `domain`
// captures life context (personal vs. professional); `vigency` captures how
// long the fact is expected to remain useful (atemporal vs. deciduous).
// Unknown values still render with a neutral palette so the row remains
// informative instead of silently dropping the value.
function ClassificationPill({ kind, value }: { kind: 'domain' | 'vigency'; value: string }) {
  const palette: Record<string, string> = {
    personal: 'bg-pink-100 text-pink-800',
    professional: 'bg-blue-100 text-blue-800',
    atemporal: 'bg-emerald-100 text-emerald-800',
    deciduous: 'bg-amber-100 text-amber-800',
  };
  const classes = palette[value] ?? 'bg-gray-100 text-gray-600';
  return (
    <span className={`px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase ${classes}`} title={kind}>
      {value}
    </span>
  );
}
