// Modal that shows the active Lens's extraction prompt and allows editing it.
// Saving bumps `prompt_version` on the backend — existing rows then count as
// stale and the user is offered a re-extraction via the standard Reextract
// button on the row (or a future "Re-extract stale" sweep).

import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import { errorText } from '@/lib/errors';
import { useLensStore } from '@/stores/lensStore';
import type { Lens } from '@/types';

interface LensPromptEditorProps {
  lens: Lens | null;
  open: boolean;
  onClose: () => void;
}

export function LensPromptEditor({ lens, open, onClose }: LensPromptEditorProps) {
  const { t } = useTranslation(['lenses']);
  const updateLens = useLensStore((s) => s.updateLens);
  const [value, setValue] = useState('');
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-seed the textarea whenever the modal opens or the active Lens changes.
  useEffect(() => {
    if (open && lens) {
      setValue(lens.promptText);
      setError(null);
    }
  }, [open, lens]);

  if (!lens) return null;

  const dirty = value.trim() !== lens.promptText.trim();

  const handleSave = async () => {
    if (!dirty) {
      onClose();
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await updateLens(lens.id, { promptText: value });
      onClose();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Edit prompt — ${lens.name}`}
      subtitle="This prompt is sent to the model alongside each email's content. Saving will mark all existing rows as stale (prompt_version bump) so they can be re-extracted."
      size="lg"
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-gray-600 px-3 py-1 text-xs text-gray-200 hover:bg-gray-700"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void handleSave()}
            disabled={!dirty || isSaving}
            className="rounded bg-blue-600 px-3 py-1 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-50"
          >
            {isSaving ? 'Saving…' : 'Save'}
          </button>
        </div>
      }
    >
      <div className="space-y-3">
        <div className="text-[11px] text-gray-500">
          Prompt version: <span className="text-gray-300">{lens.promptVersion}</span>
        </div>
        <textarea
          value={value}
          onChange={(e) => setValue(e.currentTarget.value)}
          spellCheck={false}
          rows={14}
          className="w-full rounded border border-gray-600 bg-[#1e1e1e] p-3 font-mono text-xs leading-relaxed text-gray-100 focus:border-blue-500 focus:outline-none"
          placeholder={t('lenses:prompt.placeholder')}
        />
        {error && <div className="text-xs text-red-400">{error}</div>}
      </div>
    </Modal>
  );
}
