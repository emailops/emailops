type StatusFilter = 'all' | 'promoted' | 'candidate' | 'retired';

interface StatusToggleProps {
  value: StatusFilter;
  onChange: (v: StatusFilter) => void;
}

/** Segmented control for the status filter (All / Consolidated / Candidate / Retired). */
export function StatusToggle({ value, onChange }: StatusToggleProps) {
  const options: Array<{ key: StatusFilter; label: string }> = [
    { key: 'all', label: 'All' },
    { key: 'promoted', label: 'Consolidated' },
    { key: 'candidate', label: 'Candidate' },
    { key: 'retired', label: 'Retired' },
  ];
  return (
    <div className="flex rounded-md border border-gray-300 overflow-hidden">
      {options.map((opt) => (
        <button
          key={opt.key}
          type="button"
          onClick={() => onChange(opt.key)}
          className={`px-2.5 py-1 text-xs transition-colors ${
            value === opt.key ? 'bg-primary-600 text-white' : 'text-gray-600 hover:bg-gray-50'
          }`}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

interface CompanyChipProps {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}

/** Pill-shaped chip used in the company filter row. */
export function CompanyChip({ label, count, active, onClick }: CompanyChipProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs transition-colors ${
        active ? 'bg-primary-600 text-white' : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
      }`}
    >
      <span className="font-medium">{label}</span>
      <span className={active ? 'text-primary-100' : 'text-gray-500'}>{count}</span>
    </button>
  );
}
