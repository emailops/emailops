// Pure state machine behind the Tag Board view.
//
// The board renders one column per value of a single classified tag type
// (company / priority / intent / topic). Columns come from `get_tag_stats`
// (live thread counts); each column's emails come from `get_filtered_emails`
// with that (tagType, tagValue) pair, paged independently.
//
// Everything here is pure and React-free — `TagBoardView` owns the effects and
// dispatches into `tagBoardReducer`.
import type { Email, EmailWindow, TagStat } from '@/types';

/** Tag types the classifier emits and the board can group by. Mirrors the Rust
 *  `services::filters::CLASSIFIED_TAG_TYPES`. */
export const TAG_BOARD_TYPES = ['company', 'priority', 'intent', 'topic'] as const;

export type TagBoardType = (typeof TAG_BOARD_TYPES)[number];

/** Emails fetched per column page. Small on purpose: a first paint costs one
 *  query per column, so the page has to stay cheap. */
export const TAG_BOARD_PAGE_SIZE = 8;

/** Columns to request from `get_tag_stats`. Matches the per-type cap the
 *  sidebar's smart filters already use. */
export const TAG_BOARD_MAX_COLUMNS = 15;

/** Column pages fetched at once. The tag branch of `get_filtered_emails` is
 *  index-driven and fast, but 15 simultaneous IPC round-trips on view entry
 *  still stutters the webview. */
export const TAG_BOARD_FETCH_CONCURRENCY = 4;

export function isTagBoardType(value: unknown): value is TagBoardType {
  return typeof value === 'string' && (TAG_BOARD_TYPES as readonly string[]).includes(value);
}

/** Separator for column keys. A literal newline can't appear in an account id
 *  or a classifier tag value, so no pair of parts can collide by concatenation. */
const KEY_SEP = '\n';

/** Identity of one board block: a tag value within one account. */
export function columnKey(accountId: string, tagValue: string): string {
  return `${accountId}${KEY_SEP}${tagValue}`;
}

/** Time slices offered above the board. */
export const TAG_BOARD_RANGES = ['all', 'today', 'yesterday', 'last7', 'custom'] as const;

export type TagBoardRange = (typeof TAG_BOARD_RANGES)[number];

export function isTagBoardRange(value: unknown): value is TagBoardRange {
  return typeof value === 'string' && (TAG_BOARD_RANGES as readonly string[]).includes(value);
}

export interface CustomRange {
  /** `YYYY-MM-DD`, inclusive. */
  from: string;
  /** `YYYY-MM-DD`, inclusive to the user; converted to an exclusive bound. */
  to: string;
}

function parseLocalDate(value: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value.trim());
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return Number.isNaN(d.getTime()) ? null : d;
}

/**
 * Put a custom range's two dates in order.
 *
 * `rangeToWindow` already normalises internally, but that left the two date
 * boxes still showing the reversed pair — so the board looked like it was
 * ignoring the dates on screen. Correcting the stored values makes the applied
 * range and the displayed one the same thing.
 *
 * A half-filled or unparseable pair is left untouched: that is someone
 * mid-keystroke, not a reversed range.
 */
export function normaliseCustomRange(custom: CustomRange): CustomRange {
  const from = parseLocalDate(custom.from);
  const to = parseLocalDate(custom.to);
  if (!from || !to || to >= from) return custom;
  return { from: custom.to, to: custom.from };
}

function formatLocalDate(d: Date): string {
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${mm}-${dd}`;
}

const DEFAULT_CUSTOM_DAYS = 30;

/**
 * The custom range to show when the user picks the Custom preset.
 *
 * Two empty date boxes are drawn by macOS WebKit as today's date, while an
 * empty pair filters nothing — so the board looked like "today → today" and
 * listed years of mail. Seeding a real range (the last 30 days, today
 * included) on selection keeps what is shown and what is applied identical.
 * Anything the user already typed, even one box, is kept.
 */
export function customRangeOnSelect(custom: CustomRange, now: Date): CustomRange {
  if (custom.from || custom.to) return custom;
  const from = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (DEFAULT_CUSTOM_DAYS - 1));
  return { from: formatLocalDate(from), to: formatLocalDate(now) };
}

/**
 * One date box edited. The value is stored exactly as typed — a reversed pair
 * is put in order by `normaliseCustomRange` when the box is left, not while
 * the user is still typing the year digit by digit (0002 → 0020 → 0202 → 2026
 * would otherwise flip the boxes under their fingers). A half-typed date
 * reports an empty value; that keeps the last complete one so the field does
 * not blank.
 */
export function customRangeOnEdit(custom: CustomRange, field: keyof CustomRange, value: string): CustomRange {
  if (value === '') return custom;
  return { ...custom, [field]: value };
}

function startOfLocalDay(d: Date): number {
  return Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 1000);
}

const DAY_SECONDS = 86_400;

/**
 * Turn a range preset into the backend's half-open `[since, until)` window.
 *
 * Boundaries are local midnights, and `until` is exclusive, so Today and
 * Yesterday can never both claim a thread that landed at 00:00:00.
 */
export function rangeToWindow(range: TagBoardRange, now: Date, custom?: CustomRange): EmailWindow {
  const todayStart = startOfLocalDay(now);
  switch (range) {
    case 'today':
      return { since: todayStart };
    case 'yesterday':
      return { since: todayStart - DAY_SECONDS, until: todayStart };
    case 'last7':
      // Includes today, so six whole days back plus today = seven.
      return { since: todayStart - 6 * DAY_SECONDS };
    case 'custom': {
      const a = custom?.from ? parseLocalDate(custom.from) : null;
      const b = custom?.to ? parseLocalDate(custom.to) : null;
      const window: EmailWindow = {};

      // Two date boxes are easy to fill in the wrong order. Dropping the bound
      // that looked out of place meant the board silently honoured half the
      // range and showed mail from outside the dates on screen — so normalise
      // instead, the way a date picker does. A single filled box is one bound,
      // not a reversed range, and stays as it is.
      const [from, to] = a && b && b < a ? [b, a] : [a, b];
      if (from) window.since = startOfLocalDay(from);
      if (to) window.until = startOfLocalDay(to) + DAY_SECONDS;
      return window;
    }
    default:
      return {};
  }
}

export interface TagBoardColumn {
  /** Account this block's threads belong to — the block title names it. */
  accountId: string;
  /** The tag value this column lists — e.g. "globex", "high", "billing". */
  value: string;
  /** Stable identity, `columnKey(accountId, value)`. */
  key: string;
  /** Threads carrying the tag, from `get_tag_stats`. Independent of how many
   *  emails have actually been paged in. */
  threadCount: number;
  emails: Email[];
  isLoading: boolean;
  /** A page has come back for this block, so `emails.length === 0` means
   *  genuinely empty rather than not-yet-fetched. */
  hasLoaded: boolean;
  /** False once a short page (or an error) proves there is nothing more. */
  hasMore: boolean;
  error: string | null;
}

export interface TagBoardState {
  tagType: TagBoardType;
  /** Identity of the query the current columns' emails were fetched under —
   *  dimension + category + time window. Rows only survive a refresh whose
   *  query matches; otherwise they belong to a slice the user has left. */
  queryKey: string;
  columns: TagBoardColumn[];
  isLoadingColumns: boolean;
  /** Board-level failure (the stats query itself) — column failures live on
   *  the column so one bad tag doesn't blank the board. */
  error: string | null;
}

export const initialTagBoardState: TagBoardState = {
  tagType: 'company',
  queryKey: '',
  columns: [],
  isLoadingColumns: false,
  error: null,
};

export type TagBoardAction =
  | { type: 'SET_TAG_TYPE'; tagType: TagBoardType }
  | { type: 'COLUMNS_LOADING' }
  | { type: 'COLUMNS_LOADED'; stats: TagStat[]; queryKey: string }
  | { type: 'COLUMNS_ERROR'; error: string }
  | { type: 'PAGE_LOADING'; key: string }
  | { type: 'PAGE_LOADED'; key: string; emails: Email[]; pageSize: number }
  | { type: 'PAGE_ERROR'; key: string; error: string };

function emptyColumn(stat: TagStat): TagBoardColumn {
  const accountId = stat.accountId ?? '';
  return {
    accountId,
    key: columnKey(accountId, stat.tagValue),
    value: stat.tagValue,
    threadCount: stat.count,
    emails: [],
    isLoading: false,
    hasLoaded: false,
    hasMore: true,
    error: null,
  };
}

/** Apply `patch` to the named column, leaving every other column untouched.
 *  A page for a column that is no longer on the board (the stats refreshed
 *  mid-flight) is dropped rather than resurrecting it. */
function patchColumn(
  state: TagBoardState,
  key: string,
  patch: (column: TagBoardColumn) => TagBoardColumn,
): TagBoardState {
  if (!state.columns.some((c) => c.key === key)) return state;
  return {
    ...state,
    columns: state.columns.map((c) => (c.key === key ? patch(c) : c)),
  };
}

/** Row identity for a card. Threads dedup per `(account, id)` because a
 *  provider id is only unique within one account. */
export function emailKey(email: Email): string {
  return `${email.accountId}:${email.id}`;
}

export function tagBoardReducer(state: TagBoardState, action: TagBoardAction): TagBoardState {
  switch (action.type) {
    case 'SET_TAG_TYPE':
      if (action.tagType === state.tagType) return state;
      // A different dimension means a different set of columns entirely —
      // keeping the old ones on screen would show company emails under topic
      // headers until the new stats land.
      return { ...state, tagType: action.tagType, columns: [], isLoadingColumns: true, error: null };

    case 'COLUMNS_LOADING':
      return { ...state, isLoadingColumns: true, error: null };

    case 'COLUMNS_LOADED': {
      // Carry already-paged emails across a refresh so a periodic stats reload
      // doesn't blank every column — but ONLY when the query is unchanged.
      // Those rows were fetched under the previous category/time window, and
      // keeping them across a window change left yesterday's mail on screen
      // under a "Today" filter.
      const sameQuery = state.queryKey === action.queryKey;
      const previous = new Map(state.columns.map((c) => [c.key, c]));
      return {
        ...state,
        queryKey: action.queryKey,
        isLoadingColumns: false,
        error: null,
        columns: action.stats.map((stat) => {
          const existing = sameQuery ? previous.get(columnKey(stat.accountId ?? '', stat.tagValue)) : undefined;
          return existing ? { ...existing, threadCount: stat.count } : emptyColumn(stat);
        }),
      };
    }

    case 'COLUMNS_ERROR':
      return { ...state, isLoadingColumns: false, error: action.error };

    case 'PAGE_LOADING':
      return patchColumn(state, action.key, (c) => ({ ...c, isLoading: true, error: null }));

    case 'PAGE_LOADED':
      return patchColumn(state, action.key, (c) => {
        // Append only what isn't already here. The same page can legitimately
        // arrive twice — React StrictMode double-invokes the loading effect in
        // dev, and a refresh can race a page already in flight — and appending
        // it produced duplicate React keys, which render as phantom blank
        // cards between the real ones.
        const seen = new Set(c.emails.map(emailKey));
        const added = action.emails.filter((e) => !seen.has(emailKey(e)));
        return {
          ...c,
          emails: [...c.emails, ...added],
          isLoading: false,
          hasLoaded: true,
          error: null,
          // `get_filtered_emails` returns total_count = -1 for tag filters, so a
          // page shorter than requested is the only end-of-list signal. This
          // reads the delivered page, not the deduped remainder — a fully
          // duplicate page still means the server had a full page to give.
          hasMore: action.emails.length === action.pageSize,
        };
      });

    case 'PAGE_ERROR':
      // hasMore goes false so the failed column stops auto-requesting; the
      // rendered error carries a retry affordance instead.
      return patchColumn(state, action.key, (c) => ({
        ...c,
        isLoading: false,
        hasLoaded: true,
        error: action.error,
        hasMore: false,
      }));

    default:
      return state;
  }
}

// ── Selectors ─────────────────────────────────────────────────────────────

export function selectColumn(state: TagBoardState, key: string): TagBoardColumn | undefined {
  return state.columns.find((c) => c.key === key);
}

/** Offset for the column's next page — everything already loaded. */
export function selectNextPageOffset(state: TagBoardState, key: string): number {
  return selectColumn(state, key)?.emails.length ?? 0;
}

/**
 * Blocks worth rendering: everything except one that has finished loading and
 * turned out to have no threads in the current slice. A block still loading,
 * or one that errored (its retry has to stay reachable), is kept.
 */
export function selectRenderableColumns(columns: TagBoardColumn[]): TagBoardColumn[] {
  return columns.filter((c) => !(c.hasLoaded && c.error === null && c.emails.length === 0));
}

/**
 * How many blocks to ask the backend for. Hidden tags still rank, so each one
 * the user hid must be fetched and dropped for the next tag to move up — with
 * a fixed limit, hiding three tags left three empty slots.
 */
export function tagBoardStatsLimit(hiddenForType: number): number {
  return TAG_BOARD_MAX_COLUMNS + Math.max(0, hiddenForType);
}

/** The blocks that fit on the board, once hidden ones are out of the way. */
export function capColumns<T>(columns: T[], max: number): T[] {
  return columns.length > max ? columns.slice(0, max) : columns;
}

/** True while the board has nothing to show and is not still fetching. */
export function selectIsEmpty(state: TagBoardState): boolean {
  return !state.isLoadingColumns && state.error === null && state.columns.length === 0;
}

// ── Async helper ──────────────────────────────────────────────────────────

/**
 * `Promise.all`-shaped map that keeps at most `limit` tasks in flight.
 *
 * A rejected task resolves to its `Error` in the result slot instead of
 * failing the whole batch — one column that cannot load must not strand the
 * other fourteen. Results stay in input order.
 */
export async function mapWithConcurrency<T, R>(
  items: readonly T[],
  limit: number,
  task: (item: T, index: number) => Promise<R>,
): Promise<(R | Error)[]> {
  const results = new Array<R | Error>(items.length);
  let next = 0;

  const worker = async (): Promise<void> => {
    while (next < items.length) {
      const index = next;
      next += 1;
      try {
        results[index] = await task(items[index], index);
      } catch (e) {
        results[index] = e instanceof Error ? e : new Error(String(e));
      }
    }
  };

  const workers = Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, worker);
  await Promise.all(workers);
  return results;
}

/** How wide each block is. `granular` fits the most blocks across the pane;
 *  `extended` doubles their width, halving the columns, for blocks whose
 *  subjects and snippets need the room. */
export const TAG_BOARD_DENSITIES = ['granular', 'extended'] as const;

export type TagBoardDensity = (typeof TAG_BOARD_DENSITIES)[number];

export function isTagBoardDensity(value: unknown): value is TagBoardDensity {
  return typeof value === 'string' && (TAG_BOARD_DENSITIES as readonly string[]).includes(value);
}

/** Minimum block width per density, in px. `extended` is exactly double so the
 *  column count halves at any pane width. */
export const DENSITY_MIN_COLUMN_PX: Record<TagBoardDensity, number> = {
  granular: 272, // 17rem
  extended: 544, // 34rem
};

/** Minimum block width and grid gap, in px — kept in step with the grid's
 *  Tailwind classes so the row calculation matches what the browser lays out. */
export const TAG_BOARD_MIN_COLUMN_PX = 272; // 17rem
export const TAG_BOARD_GAP_PX = 12; // gap-3

/**
 * Would every block sit on a single row at this pane width?
 *
 * Only a single row is stretched to fill the pane — once blocks wrap, filling
 * would make each row as tall as the viewport and hide the rows below.
 * Mirrors `repeat(auto-fill, minmax(TAG_BOARD_MIN_COLUMN_PX, 1fr))`.
 */
export function fitsInOneRow(
  columnCount: number,
  containerWidth: number,
  minColumnPx: number = TAG_BOARD_MIN_COLUMN_PX,
  gapPx: number = TAG_BOARD_GAP_PX,
): boolean {
  if (columnCount <= 1) return true;
  // Not measured yet — assume one row so the first paint isn't a squashed grid.
  if (containerWidth <= 0) return true;
  const perRow = Math.max(1, Math.floor((containerWidth + gapPx) / (minColumnPx + gapPx)));
  return columnCount <= perRow;
}

/**
 * Order blocks by the user's saved arrangement.
 *
 * Saved keys come first, in saved order. Blocks the saved order has never seen
 * — a tag classified since the user last dragged anything — keep their ranked
 * position at the end rather than disappearing. Saved keys whose block is gone
 * are skipped. Every input block appears exactly once.
 */
export function applySavedOrder(columns: TagBoardColumn[], savedKeys: string[]): TagBoardColumn[] {
  const byKey = new Map(columns.map((c) => [c.key, c]));
  const ordered: TagBoardColumn[] = [];
  const placed = new Set<string>();

  for (const key of savedKeys) {
    const column = byKey.get(key);
    if (column && !placed.has(key)) {
      ordered.push(column);
      placed.add(key);
    }
  }
  for (const column of columns) {
    if (!placed.has(column.key)) ordered.push(column);
  }
  return ordered;
}

/** Which gap beside a block a drop lands in. */
export type DropSide = 'before' | 'after';

/**
 * Move `fromKey` into the gap on `side` of `toKey`.
 *
 * The drop indicator highlights a *gap*, not a block, so the side is the
 * caller's decision (which half of the target the pointer is over) rather than
 * something inferred from travel direction. "after b" and "before c" name the
 * same gap and produce the same order.
 *
 * Returns the order unchanged when either key is absent or they are the same
 * block.
 */
export function moveColumnKey(order: string[], fromKey: string, toKey: string, side: DropSide): string[] {
  if (fromKey === toKey) return order;
  if (!order.includes(fromKey) || !order.includes(toKey)) return order;

  const next = order.filter((k) => k !== fromKey);
  next.splice(next.indexOf(toKey) + (side === 'after' ? 1 : 0), 0, fromKey);
  return next;
}

/** Blocks already asked for, and the query they were asked for under. */
export interface RequestLedger {
  queryKey: string;
  keys: Set<string>;
}

/**
 * Which blocks to fetch a first page for, and the ledger to carry forward.
 *
 * Two rules, both learned the hard way:
 *
 * - **Nothing is queued while the block set is still being fetched.** During
 *   that window the columns on screen belong to the *previous* query. Paging
 *   them marks their keys as requested, and when the real columns arrive under
 *   a new query — clearing their rows — those keys look already-handled and are
 *   never fetched, leaving the blocks on a skeleton forever.
 * - **The ledger is scoped to the query.** A new category, range or search
 *   means the rows on screen were fetched under the old window, so every block
 *   is fair game again.
 */
export function planColumnLoads(
  columns: TagBoardColumn[],
  ledger: RequestLedger,
  queryKey: string,
  isLoadingColumns: boolean,
): { toLoad: TagBoardColumn[]; ledger: RequestLedger } {
  if (isLoadingColumns) return { toLoad: [], ledger };

  const current = ledger.queryKey === queryKey ? ledger : { queryKey, keys: new Set<string>() };
  const toLoad = columns.filter((c) => !current.keys.has(c.key));
  if (toLoad.length === 0) return { toLoad, ledger: current };

  const keys = new Set(current.keys);
  for (const c of toLoad) keys.add(c.key);
  return { toLoad, ledger: { queryKey, keys } };
}

/**
 * What to show as a card's sender.
 *
 * The user's own address on their own message is noise — every mail client
 * writes "Me" there instead. Matches on the provider's sent flag as well as the
 * address, so aliases and send-as identities are still recognised.
 */
export function senderLabel(email: Email, ownerEmail: string, meLabel: string): string {
  const owner = ownerEmail.trim().toLowerCase();
  const from = email.senderEmail.trim().toLowerCase();
  if (email.isSent || (owner !== '' && from === owner)) return meLabel;
  return email.sender || email.senderEmail;
}

/** Dimensions whose values are a scale rather than independent labels. Mirrors
 *  the Rust `tag_ordinal`. */
const ORDINAL_TAG_TYPES = new Set<string>(['priority']);

/**
 * On an ordinal dimension, show a thread only in the highest-ranked block that
 * holds it.
 *
 * A thread matches a tag when *any* of its messages carries it, which is right
 * for topics and companies — one conversation really can be about billing and a
 * project. It is wrong for priority: a thread whose messages were classified
 * `normal` and `low` appeared under both, reading as one conversation with two
 * priorities. Blocks arrive ordered urgent → normal → low, so the first block
 * to claim a thread keeps it.
 *
 * Done here rather than in SQL deliberately: the "highest priority per thread"
 * query measured **396 seconds** on a 6 GB mailbox, for something affecting
 * ~3% of threads.
 */
export function dedupeOrdinalThreads(columns: TagBoardColumn[], tagType: string): TagBoardColumn[] {
  if (!ORDINAL_TAG_TYPES.has(tagType)) return columns;

  const claimed = new Set<string>();
  return columns.map((column) => {
    const kept = column.emails.filter((e) => {
      // Per account: two mailboxes can carry the same provider thread id.
      const thread = `${e.accountId}\n${e.threadId}`;
      if (claimed.has(thread)) return false;
      claimed.add(thread);
      return true;
    });
    return kept.length === column.emails.length ? column : { ...column, emails: kept };
  });
}
