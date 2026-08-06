interface ProviderTabProps {
  active: boolean;
  label: string;
  description: string;
  onClick: () => void;
  /** Renders the tab greyed out and inert. Used for the embedded runtime on
   *  hosts that cannot execute it (a build compiled without llama.cpp, or an
   *  Intel Mac, whose GPU cannot run the Metal kernels). Selecting it there
   *  produced an opaque `Decode Error -3` on every AI turn. */
  disabled?: boolean;
  /** Replaces `description` when disabled, so the tab explains why. */
  disabledReason?: string;
}

export function ProviderTab({ active, label, description, onClick, disabled, disabledReason }: ProviderTabProps) {
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      title={disabled ? disabledReason : undefined}
      className={`flex-1 text-left px-4 py-3 rounded border transition-colors ${
        disabled
          ? 'bg-[#242425] border-gray-800 text-gray-600 cursor-not-allowed'
          : active
            ? 'bg-primary-900/40 border-primary-600 text-primary-300'
            : 'bg-[#2a2a2b] border-gray-700 text-gray-400 hover:border-gray-500 hover:text-gray-300'
      }`}
    >
      <div className="text-sm font-medium">{label}</div>
      <div className="text-xs mt-0.5 opacity-70">{disabled ? (disabledReason ?? description) : description}</div>
    </button>
  );
}
