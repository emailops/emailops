import type { ActiveFilter, FilterType } from '@/types';

/**
 * A smart filter and the search-box query that lists the same mail.
 *
 * Clicking a filter writes its token into the search box; the search text is
 * the one source of truth, and the sidebar highlights the filter whose token
 * is the whole query. A filter and its text are one query, so they list the
 * same threads.
 */

const TAG_TYPES: readonly FilterType[] = ['priority', 'intent', 'topic', 'company'];

/** Wrap an operator argument in quotes when a space would split it. */
function quoteIfNeeded(operator: string, argument: string): string {
  return /\s/.test(argument) ? `${operator}"${argument}"` : `${operator}${argument}`;
}

export function filterToken(filter: ActiveFilter): string {
  switch (filter.type) {
    case 'sender':
      return quoteIfNeeded('from:', filter.value);
    case 'domain':
      return quoteIfNeeded('domain:', filter.value);
    case 'attachment_ext':
      return quoteIfNeeded('ext:', filter.value);
    default:
      return quoteIfNeeded('tag:', `${filter.type}=${filter.value}`);
  }
}

/** The smart filter a query names, when the query is exactly one filter token. */
export function filterFromQuery(query: string | null): ActiveFilter | null {
  const trimmed = (query ?? '').trim();
  const match = /^(tag|from|domain|ext):(?:"([^"]+)"|(\S+))$/i.exec(trimmed);
  if (!match) return null;
  const operator = match[1].toLowerCase();
  const argument = match[2] ?? match[3];
  switch (operator) {
    case 'from':
      // The sender filter is a whole address; `from:ana` is a name search.
      return argument.includes('@') ? { type: 'sender', value: argument } : null;
    case 'domain':
      return { type: 'domain', value: argument };
    case 'ext':
      return { type: 'attachment_ext', value: argument };
    default: {
      const eq = argument.indexOf('=');
      if (eq <= 0) return null;
      const type = argument.slice(0, eq).toLowerCase() as FilterType;
      const value = argument.slice(eq + 1);
      return TAG_TYPES.includes(type) && value ? { type, value } : null;
    }
  }
}

/** The current query with the filter's token added (once). */
export function appendFilterToken(query: string | null, filter: ActiveFilter): string {
  const token = filterToken(filter);
  const current = (query ?? '').trim();
  if (!current) return token;
  if (current.split(/\s+/).includes(token)) return current;
  return `${current} ${token}`;
}

/** Where the email list comes from. */
export type ListSource = { kind: 'query'; query: string } | { kind: 'mailbox' };

/** Any query, written or set by a smart filter, takes the one list with a query. */
export function planListSource(searchQuery: string | null): ListSource {
  const query = (searchQuery ?? '').trim();
  return query ? { kind: 'query', query } : { kind: 'mailbox' };
}
