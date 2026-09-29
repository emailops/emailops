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

/** `YYYY-MM-DD` → `DD/MM/YYYY`; anything else is shown as written. */
function formatPlanDate(value: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  return m ? `${m[3]}/${m[2]}/${m[1]}` : value;
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
  // `until` is the last day the search includes.
  for (const field of ['since', 'until'] as const) {
    const v = text(field);
    if (v) parts.push({ field, value: formatPlanDate(v) });
  }
  if (filter.unread === true) parts.push({ field: 'unread', value: '' });
  return parts;
}
