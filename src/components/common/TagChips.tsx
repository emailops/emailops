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
  // Company values are an open vocabulary (one per organisation), so they get
  // one shared colour rather than a per-value map.
  company: {
    _default: 'bg-indigo-50 text-indigo-700',
  },
  priority: {
    urgent: 'bg-red-100 text-red-800',
    normal: 'bg-green-100 text-green-800',
    low: 'bg-gray-100 text-gray-500',
  },
  intent: {
    request: 'bg-blue-100 text-blue-800',
    approval: 'bg-purple-100 text-purple-800',
    scheduling: 'bg-cyan-100 text-cyan-800',
    delivery: 'bg-teal-100 text-teal-800',
    question: 'bg-indigo-100 text-indigo-800',
    introduction: 'bg-pink-100 text-pink-800',
    feedback: 'bg-amber-100 text-amber-800',
    notification: 'bg-slate-100 text-slate-700',
    complaint: 'bg-red-100 text-red-700',
    promotion: 'bg-orange-100 text-orange-700',
    conversation: 'bg-sky-100 text-sky-800',
  },
  topic: {
    _default: 'bg-amber-50 text-amber-700',
  },
  // Severity-ordered on purpose: an impersonation attempt and an unwanted
  // newsletter must not look alike at a glance.
  junk: {
    phishing: 'bg-red-100 text-red-800',
    spam: 'bg-orange-100 text-orange-800',
    graymail: 'bg-slate-100 text-slate-600',
    _default: 'bg-slate-100 text-slate-600',
  },
};

/** Chip background/foreground for a (tagType, tagValue) pair. Exported so
 *  surfaces outside the chip row — the tag board's column headers — colour a
 *  tag the same way the inline chips do. */
export function getTagChipColor(tagType: string, tagValue: string): string {
  const typeColors = TAG_COLORS[tagType];
  if (!typeColors) return 'bg-gray-100 text-gray-600';
  return typeColors[tagValue] || typeColors._default || 'bg-gray-100 text-gray-600';
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
          className={`inline-block rounded-full font-medium ${sizeClass} ${getTagChipColor(tag.tagType, tag.tagValue)}`}
          title={`${tag.tagType}: ${tag.tagValue}${tag.confidence != null ? ` (${Math.round(tag.confidence * 100)}%)` : ''}`}
        >
          {tag.tagValue}
        </span>
      ))}
    </div>
  );
}

/**
 * Just the background tint of a tag's chip colour.
 *
 * Block headers on the tag board are a full-width bar, not a small pill, so
 * they pair the tint with their own near-black text. The chip's own foreground
 * (a 700-weight of the same hue) is tuned for a tiny pill and reads as washed
 * out across a whole header — amber especially.
 */
export function tagHeaderBackground(tagType: string, tagValue: string): string {
  const background = getTagChipColor(tagType, tagValue)
    .split(' ')
    .find((c) => c.startsWith('bg-'));
  return background ?? 'bg-gray-100';
}
