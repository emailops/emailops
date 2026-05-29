import { useFormatters } from '@/hooks/useFormatters';
import type { Attachment } from '@/types';

interface AttachmentRowProps {
  attachment: Attachment;
  ruleName?: string;
  ruleColor?: string;
  isSelected: boolean;
  isChecked: boolean;
  onToggleChecked: () => void;
  onClick: () => void;
}

const RULE_COLORS = [
  'text-violet-600',
  'text-teal-600',
  'text-rose-600',
  'text-amber-600',
  'text-cyan-600',
  'text-fuchsia-600',
  'text-lime-600',
  'text-sky-600',
  'text-orange-600',
  'text-emerald-600',
];

export function ruleColor(ruleId: string): string {
  let hash = 0;
  for (let i = 0; i < ruleId.length; i++) {
    hash = ((hash << 5) - hash + ruleId.charCodeAt(i)) | 0;
  }
  return RULE_COLORS[Math.abs(hash) % RULE_COLORS.length];
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function AttachmentRow({
  attachment,
  ruleName,
  ruleColor: color,
  isSelected,
  isChecked,
  onToggleChecked,
  onClick,
}: AttachmentRowProps) {
  const fmt = useFormatters();
  const date = new Date(attachment.emailTimestamp * 1000);
  const isThisYear = date.getFullYear() === new Date().getFullYear();
  const formattedDate = fmt.date(
    attachment.emailTimestamp,
    isThisYear ? { month: 'short', day: 'numeric' } : { year: 'numeric', month: 'short', day: 'numeric' },
  );
  return (
    <div
      onClick={onClick}
      className={`flex items-center gap-2.5 px-3 py-2 cursor-pointer border-b border-gray-100 transition-colors ${
        isSelected ? 'bg-primary-50 border-l-2 border-l-primary-500' : 'hover:bg-gray-50'
      }`}
    >
      <input
        type="checkbox"
        checked={isChecked}
        onChange={(e) => {
          e.stopPropagation();
          onToggleChecked();
        }}
        onClick={(e) => e.stopPropagation()}
        className="w-3.5 h-3.5 rounded border-gray-300 text-primary-600 focus:ring-primary-500 flex-shrink-0"
      />
      <div className="flex-1 min-w-0">
        {/* Line 1: RULE NAME / filename   Date */}
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-baseline gap-1 min-w-0">
            {ruleName && (
              <>
                <span className={`text-xs font-semibold flex-shrink-0 ${color ?? 'text-primary-600'}`}>{ruleName}</span>
                <span className="text-gray-300 text-xs flex-shrink-0">/</span>
              </>
            )}
            <span className="text-sm text-gray-900 truncate">{attachment.filename}</span>
          </div>
          <span className="text-[11px] text-gray-400 flex-shrink-0">{formattedDate}</span>
        </div>
        {/* Line 2: file size  tags */}
        <div className="flex items-center gap-1.5 mt-0.5">
          <span className="text-[11px] text-gray-400">{formatFileSize(attachment.fileSize)}</span>
          {attachment.tags.map((tag) => (
            <span
              key={tag}
              className="inline-flex items-center px-1.5 py-0 rounded text-[10px] font-medium bg-gray-100 text-gray-500"
            >
              {tag}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}
