/** The account an item belongs to, shown in "All accounts": its colour dot and address. */
export function AccountChip({ label, colorClass, testId }: { label: string; colorClass: string; testId: string }) {
  return (
    <span
      data-testid={testId}
      className="inline-flex items-center gap-1 flex-shrink min-w-0 max-w-[50%] rounded-full px-1.5 text-[10px] bg-gray-100 text-gray-700"
      title={label}
    >
      <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${colorClass}`} aria-hidden="true" />
      <span className="truncate">{label}</span>
    </span>
  );
}
