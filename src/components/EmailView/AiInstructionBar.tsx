import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineError } from '@/components/common/InlineError';

interface AiInstructionBarProps {
  /** Called with the trimmed instruction ('' when the box is empty). */
  onGenerate: (instructions: string) => void;
  /** Edit the draft already written as the instruction says ('' when empty). */
  onPolish: (instructions: string) => void;
  /** A draft or polish is running: both actions and the box are locked. */
  isGenerating: boolean;
  /** A draft is already in the composer: Generate offers to regenerate and
   *  Polish has something to work on. */
  hasDraft: boolean;
  /** The composer holds what a draft needs (a new email: recipients and a
   *  subject). Default true. */
  canGenerate?: boolean;
  /** The draft holds links or images, which Polish (plain text in, plain
   *  text out) would drop: Polish is not offered. Default false. */
  draftHasLinksOrImages?: boolean;
  /** Tooltip of the Generate button: what it uses, and needs. */
  generateTitle?: string;
  /** Why the last request failed, shown under the bar. */
  error?: string | null;
}

/**
 * Instructions for the AI, kept apart from the email: the text here is never
 * sent nor saved with the draft. **Generate** writes the draft following them
 * (Enter does too); **Polish** edits the draft already written, keeping its
 * facts and meaning. The box keeps its text after a failure, for a retry.
 */
export function AiInstructionBar({
  onGenerate,
  onPolish,
  isGenerating,
  hasDraft,
  canGenerate = true,
  draftHasLinksOrImages = false,
  generateTitle,
  error,
}: AiInstructionBarProps) {
  const { t } = useTranslation(['compose']);
  const [instruction, setInstruction] = useState('');
  // Which action is running, so its button says so.
  const [running, setRunning] = useState<'generate' | 'polish'>('generate');

  const generate = () => {
    if (isGenerating || !canGenerate) return;
    setRunning('generate');
    onGenerate(instruction.trim());
  };
  const canPolish = hasDraft && !draftHasLinksOrImages;
  const polish = () => {
    if (isGenerating || !canPolish) return;
    setRunning('polish');
    onPolish(instruction.trim());
  };

  const buttonClass =
    'shrink-0 rounded px-3 py-1 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60';

  return (
    <div className="mb-2">
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={instruction}
          onChange={(e) => setInstruction(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              generate();
            }
          }}
          disabled={isGenerating}
          placeholder={t('compose:aiDraft.instructionPlaceholder')}
          aria-label={t('compose:aiDraft.instructionPlaceholder')}
          data-testid="ai-instructions"
          className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm focus:border-purple-500 focus:outline-none disabled:bg-gray-50"
        />
        <button
          type="button"
          onClick={generate}
          disabled={isGenerating || !canGenerate}
          title={generateTitle}
          data-testid="ai-generate"
          className={`${buttonClass} bg-purple-600 text-white hover:bg-purple-700`}
        >
          {isGenerating && running === 'generate'
            ? t('compose:aiDraft.generating')
            : hasDraft
              ? t('compose:aiDraft.regenerate')
              : t('compose:aiDraft.generate')}
        </button>
        <button
          type="button"
          onClick={polish}
          disabled={isGenerating || !canPolish}
          title={
            !hasDraft
              ? t('compose:aiDraft.polishNeedsDraft')
              : draftHasLinksOrImages
                ? t('compose:aiDraft.polishDropsLinks')
                : t('compose:aiDraft.polishTitle')
          }
          data-testid="ai-polish"
          className={`${buttonClass} border border-purple-300 bg-white text-purple-700 hover:bg-purple-50`}
        >
          {isGenerating && running === 'polish' ? t('compose:aiDraft.polishing') : t('compose:aiDraft.polish')}
        </button>
      </div>
      <InlineError message={error} className="mt-2" />
    </div>
  );
}
