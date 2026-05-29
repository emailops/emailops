import { useFormatters } from '@/hooks/useFormatters';

interface ProgressBarProps {
  label: string;
  numerator: number;
  denominator: number;
  /** Tailwind background class for the filled portion. */
  color?: string;
}

export function ProgressBar({ label, numerator, denominator, color = 'bg-blue-500' }: ProgressBarProps) {
  const fmt = useFormatters();
  const pct = denominator > 0 ? Math.min(100, Math.round((numerator / denominator) * 100)) : 0;
  return (
    <div>
      <div className="flex justify-between text-xs text-gray-300 mb-1">
        <span>{label}</span>
        <span className="font-mono text-gray-400">
          {pct}% — {fmt.number(numerator)} / {fmt.number(denominator)}
        </span>
      </div>
      <div className="w-full bg-gray-800 rounded h-2 overflow-hidden">
        <div className={`h-2 ${color} transition-all`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}
