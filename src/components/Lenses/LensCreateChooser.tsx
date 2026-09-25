// First step of "New Lens": build it by talking to the chat, or by hand.
// The chat path opens a new conversation with a request to complete, in the UI
// language, naming the account on screen.

import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import { isUnifiedMode, useAccountStore } from '@/stores/accountStore';

interface LensCreateChooserProps {
  open: boolean;
  onClose: () => void;
  onManual: () => void;
  /** Open the chat panel on a new conversation with `prompt` in its input. */
  onChat: (prompt: string) => void;
}

export function LensCreateChooser({ open, onClose, onManual, onChat }: LensCreateChooserProps) {
  const { t } = useTranslation(['lenses']);
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const accounts = useAccountStore((s) => s.accounts);

  const chatPrompt = () => {
    const account = isUnifiedMode(activeAccountId) ? null : accounts.find((a) => a.id === activeAccountId);
    return account
      ? t('lenses:chooser.chatPrompt', { account: account.email })
      : t('lenses:chooser.chatPromptAllAccounts');
  };

  const option = (label: string, hint: string, icon: string, onClick: () => void) => (
    <button
      type="button"
      onClick={onClick}
      className="flex flex-1 flex-col items-center gap-2 rounded-lg border border-gray-700 bg-[#1e1e1e]/60 p-5 text-center transition-colors hover:border-blue-500/60 hover:bg-blue-900/10"
    >
      <span className="text-2xl leading-none">{icon}</span>
      <span className="text-sm font-medium text-gray-100">{label}</span>
      <span className="text-[11px] leading-snug text-gray-400">{hint}</span>
    </button>
  );

  return (
    <Modal open={open} onClose={onClose} title={t('lenses:chooser.title')} size="md">
      <div className="flex flex-col gap-3 sm:flex-row">
        {option(t('lenses:chooser.chat'), t('lenses:chooser.chatHint'), '💬', () => onChat(chatPrompt()))}
        {option(t('lenses:chooser.manual'), t('lenses:chooser.manualHint'), '🛠️', onManual)}
      </div>
    </Modal>
  );
}
