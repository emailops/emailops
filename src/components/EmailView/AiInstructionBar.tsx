import { useState } from 'react';
import { useTranslation } from 'react-i18next';

interface AiInstructionBarProps {
  /** Called with the trimmed instruction ('' when the box is empty). */
  onGenerate: (instructions: string) => void;
  isGenerating: boolean;
  /** A draft is already in the composer: the button offers to regenerate. */
  hasDraft: boolean;
}

/**
 * Free-text steer for the AI reply draft ("accept, but propose Thursday").
 * Sits above the reply editor; Enter or the button (re)generates the draft
 * with the instruction passed to `generate_draft`.
 */
export function AiInstructionBar({ onGenerate, isGenerating, hasDraft }: AiInstructionBarProps) {
  const { t } = useTranslation(['compose']);
  const [instruction, setInstruction] = useState('');

  const submit = () => {
    if (isGenerating) return;
    onGenerate(instruction.trim());
  };

  return (
    <div className="mb-2 flex items-center gap-2">
      <input
        type="text"
        value={instruction}
        onChange={(e) => setInstruction(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            submit();
          }
        }}
        disabled={isGenerating}
        placeholder={t('compose:aiDraft.instructionPlaceholder')}
        aria-label={t('compose:aiDraft.instructionPlaceholder')}
        className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm focus:border-purple-500 focus:outline-none disabled:bg-gray-50"
      />
      <button
        type="button"
        onClick={submit}
        disabled={isGenerating}
        className="shrink-0 rounded bg-purple-600 px-3 py-1 text-sm font-medium text-white transition-colors hover:bg-purple-700 disabled:cursor-not-allowed disabled:opacity-60"
      >
        {isGenerating
          ? t('compose:aiDraft.generating')
          : hasDraft
            ? t('compose:aiDraft.regenerate')
            : t('compose:aiDraft.generate')}
      </button>
    </div>
  );
}
