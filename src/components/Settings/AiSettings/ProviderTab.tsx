interface ProviderTabProps {
  active: boolean;
  label: string;
  description: string;
  onClick: () => void;
}

export function ProviderTab({ active, label, description, onClick }: ProviderTabProps) {
  return (
    <button
      onClick={onClick}
      className={`flex-1 text-left px-4 py-3 rounded border transition-colors ${
        active
          ? 'bg-primary-900/40 border-primary-600 text-primary-300'
          : 'bg-[#2a2a2b] border-gray-700 text-gray-400 hover:border-gray-500 hover:text-gray-300'
      }`}
    >
      <div className="text-sm font-medium">{label}</div>
      <div className="text-xs mt-0.5 opacity-70">{description}</div>
    </button>
  );
}
