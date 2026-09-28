/** The research estimate's filter (the `search_emails` arguments the gather
 *  ran) as labelled parts the confirmation card can show, so the user can
 *  judge whether the search makes sense before starting it. Pure. */

export type ResearchFilterField =
  | 'from'
  | 'to'
  | 'with'
  | 'subject'
  | 'query'
  | 'intent'
  | 'topic'
  | 'since'
  | 'until'
  | 'unread';

export interface ResearchFilterPart {
  field: ResearchFilterField;
  /** Empty for a flag (`unread`) whose label says it all. */
  value: string;
}

const TEXT_FIELDS: ResearchFilterField[] = ['from', 'to', 'with', 'subject', 'query', 'intent', 'topic'];

/** `YYYY-MM-DD` → `DD/MM/YYYY`, moved back `daysBack` days; anything else is
 *  shown as written. Calendar arithmetic in UTC so no zone shifts the day. */
function formatPlanDate(value: string, daysBack = 0): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return value;
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]) - daysBack));
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getUTCDate())}/${p(d.getUTCMonth() + 1)}/${d.getUTCFullYear()}`;
}

export function researchFilterParts(filter: Record<string, unknown>): ResearchFilterPart[] {
  const text = (key: string): string | null => {
    const v = filter[key];
    return typeof v === 'string' && v.trim() ? v.trim() : null;
  };
  const parts: ResearchFilterPart[] = [];
  for (const field of TEXT_FIELDS) {
    const v = text(field);
    if (v) parts.push({ field, value: v });
  }
  const since = text('since');
  if (since) parts.push({ field: 'since', value: formatPlanDate(since) });
  // `until` is exclusive (the search keeps mail before it): show the last day
  // the search includes, or "until 28/09" would promise mail from the 28th.
  const until = text('until');
  if (until) parts.push({ field: 'until', value: formatPlanDate(until, 1) });
  if (filter.unread === true) parts.push({ field: 'unread', value: '' });
  return parts;
}
