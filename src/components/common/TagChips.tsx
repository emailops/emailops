import type { EmailTag } from '@/types';

interface TagChipsProps {
  tags: EmailTag[];
  compact?: boolean;
  /**
   * When true, chips never wrap to a second line. Used inside virtualized
   * email rows where a wrapped chip line silently grows the row past its
   * measured height and causes the virtualizer to lay subsequent rows on
   * top of each other (visual overlap) until ResizeObserver catches up.
   */
  nowrap?: boolean;
}

const TAG_COLORS: Record<string, Record<string, string>> = {
  priority: {
    urgent: 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-300',
    normal: 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-300',
    low: 'bg-gray-100 text-gray-500 dark:bg-surface-hover dark:text-gray-400',
  },
  intent: {
    request: 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-300',
    approval: 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-300',
    scheduling: 'bg-cyan-100 text-cyan-800',
    delivery: 'bg-teal-100 text-teal-800',
    question: 'bg-indigo-100 text-indigo-800 dark:bg-indigo-900/30 dark:text-indigo-300',
    introduction: 'bg-pink-100 text-pink-800',
    feedback: 'bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-300',
    notification: 'bg-slate-100 text-slate-700',
    complaint: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-300',
    promotion: 'bg-orange-100 text-orange-700 dark:bg-orange-900/30 dark:text-orange-300',
    conversation: 'bg-sky-100 text-sky-800 dark:bg-sky-900/30 dark:text-sky-300',
  },
  topic: {
    _default: 'bg-amber-50 text-amber-700 dark:bg-amber-900/20 dark:text-amber-300',
  },
  // Severity-ordered on purpose: an impersonation attempt and an unwanted
  // newsletter must not look alike at a glance.
  junk: {
    phishing: 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-300',
    spam: 'bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-300',
    graymail: 'bg-slate-100 text-slate-600',
    _default: 'bg-slate-100 text-slate-600',
  },
};

function getChipColor(tagType: string, tagValue: string): string {
  const typeColors = TAG_COLORS[tagType];
  if (!typeColors) return 'bg-gray-100 text-gray-600 dark:bg-surface-hover dark:text-gray-400';
  return (
    typeColors[tagValue] || typeColors._default || 'bg-gray-100 text-gray-600 dark:bg-surface-hover dark:text-gray-400'
  );
}

export function TagChips({ tags, compact = false, nowrap = false }: TagChipsProps) {
  if (tags.length === 0) return null;

  const sizeClass = compact ? 'text-[10px] px-1.5 py-0' : 'text-xs px-2 py-0.5';

  // Sort: junk first (it changes how the whole row should be read), then
  // priority, intent, topic.
  const order = ['junk', 'priority', 'intent', 'topic'];
  const sorted = [...tags].sort((a, b) => order.indexOf(a.tagType) - order.indexOf(b.tagType));

  return (
    <div className={`flex gap-1 ${nowrap ? 'flex-nowrap' : 'flex-wrap'}`}>
      {sorted.map((tag) => (
        <span
          key={tag.tagType}
          className={`inline-block rounded-full font-medium ${sizeClass} ${getChipColor(tag.tagType, tag.tagValue)}`}
          title={`${tag.tagType}: ${tag.tagValue}${tag.confidence != null ? ` (${Math.round(tag.confidence * 100)}%)` : ''}`}
        >
          {tag.tagValue}
        </span>
      ))}
    </div>
  );
}
