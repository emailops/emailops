import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import {
  applySavedOrder,
  type CustomRange,
  capColumns,
  customRangeOnSelect,
  DENSITY_MIN_COLUMN_PX,
  type DropSide,
  dedupeOrdinalThreads,
  fitsInOneRow,
  initialTagBoardState,
  mapWithConcurrency,
  moveColumnKey,
  normaliseCustomRange,
  planColumnLoads,
  type RequestLedger,
  rangeToWindow,
  selectIsEmpty,
  selectRenderableColumns,
  TAG_BOARD_FETCH_CONCURRENCY,
  TAG_BOARD_MAX_COLUMNS,
  TAG_BOARD_PAGE_SIZE,
  type TagBoardColumn,
  type TagBoardDensity,
  type TagBoardRange,
  type TagBoardType,
  tagBoardReducer,
  tagBoardStatsLimit,
} from '@/lib/tagBoard';
import { toQueryAccountId, useAccountStore } from '@/stores/accountStore';
import { useJunkStore } from '@/stores/junkStore';
import { useLogStore } from '@/stores/logStore';
import type { Email, EmailCategory, EmailWindow, SmartFilterPref } from '@/types';
import { TagBoardToolbar } from './TagBoardToolbar';
import { TagColumn } from './TagColumn';
import type { TagEmailCardProps } from './TagEmailCard';

interface TagBoardViewProps {
  /** UI account identity — may be the unified "All accounts" sentinel, which
   *  the board supports natively (it splits into one block per account). */
  accountId: string | null;
  selectedEmailId: string | null;
  onSelectEmail: (email: Email) => void;
  /** Apply a tag as a smart filter and switch to the inbox list. */
  onOpenTagInInbox: (tagType: TagBoardType, tagValue: string) => void;
  /** Open Settings → Classification (shown when nothing is classified yet). */
  onOpenClassificationSettings: () => void;
  /** Start a new chat conversation. */
  onNewChat?: () => void;
  /** Persisted "group by" dimension and its setter. */
  tagType: TagBoardType;
  onChangeTagType: (tagType: TagBoardType) => void;
  /** Persisted block width. */
  density: TagBoardDensity;
  onChangeDensity: (density: TagBoardDensity) => void;
  /** The inbox row's ⋮ actions, offered on every card. */
  cardActions?: TagEmailCardProps['actions'];
}

/** Hidden-block key. Mirrors the backend pref row: prefs are per account, and
 *  the filter type is the tag type. */
function hiddenKey(accountId: string, tagType: string, tagValue: string): string {
  return `${accountId}\n${tagType}\n${tagValue}`;
}

export function TagBoardView({
  accountId,
  selectedEmailId,
  onSelectEmail,
  onOpenTagInInbox,
  onOpenClassificationSettings,
  onNewChat,
  tagType,
  onChangeTagType,
  density,
  onChangeDensity,
  cardActions,
}: TagBoardViewProps) {
  const { t } = useTranslation(['tagboard', 'common']);
  const addLog = useLogStore((s) => s.addLog);
  const [state, dispatch] = useReducer(tagBoardReducer, { ...initialTagBoardState, tagType });

  const [range, setRange] = useState<TagBoardRange>('all');
  const [custom, setCustom] = useState<CustomRange>({ from: '', to: '' });
  const [selectedCategories, setSelectedCategories] = useState<Set<EmailCategory>>(new Set());
  // Same preference the inbox's "Hide junk" checkbox writes, so the two
  // controls can never disagree about what is being suppressed.
  const junkFlaggedAction = useJunkStore((s) => s.flaggedAction);
  const setJunkFlaggedAction = useJunkStore((s) => s.setFlaggedAction);
  const loadJunkConfig = useJunkStore((s) => s.loadConfig);
  const hideJunk = junkFlaggedAction === 'hide';
  useEffect(() => {
    void loadJunkConfig();
  }, [loadJunkConfig]);

  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  // Each keystroke would otherwise re-run the stats query and re-page every
  // block against a 6 GB mailbox.
  useEffect(() => {
    const id = setTimeout(() => setDebouncedSearch(search.trim()), 250);
    return () => clearTimeout(id);
  }, [search]);
  const [availableCategories, setAvailableCategories] = useState<EmailCategory[]>([]);
  const [prefs, setPrefs] = useState<SmartFilterPref[]>([]);
  /** Thread participants, keyed `accountId\nthreadId`. Filled in a batch per
   *  loaded page, so a card can name the conversation it represents. */
  const [participants, setParticipants] = useState<Record<string, string[]>>({});
  /** User's drag-ordered block keys for the current dimension. */
  const [savedOrder, setSavedOrder] = useState<string[]>([]);
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ key: string; side: DropSide } | null>(null);

  const queryAccountId = toQueryAccountId(accountId);
  const allAccounts = useAccountStore((s) => s.accounts);
  const emailByAccount = useMemo(() => new Map(allAccounts.map((a) => [a.id, a.email])), [allAccounts]);

  // A window value that only changes when the slice actually changes, so it
  // can be an effect dependency without refetching on every render.
  const windowKey = useMemo(() => {
    const w = rangeToWindow(range, new Date(), custom);
    const categories = Array.from(selectedCategories).sort();
    // One block per thread: a thread's tag is that of its newest classified
    // message. Without this a six-message thread sat in four intent blocks.
    return JSON.stringify({
      ...w,
      categories,
      search: debouncedSearch || null,
      hideGraymail: hideJunk,
      latestTagOnly: true,
    });
  }, [range, custom, selectedCategories, debouncedSearch, hideJunk]);
  const window: EmailWindow = useMemo(() => JSON.parse(windowKey), [windowKey]);
  /** Query identity for the rows on screen: dimension + category + window. */
  const queryKey = `${tagType}|${queryAccountId ?? '*'}|${windowKey}`;

  // Only a single row of blocks is stretched to fill the pane; once they wrap,
  // stretching would make every row viewport-tall and hide the rows below.
  const [gridWidth, setGridWidth] = useState(0);
  const observerRef = useRef<ResizeObserver | null>(null);
  // Callback ref, not useRef + mount effect: the grid is only rendered when the
  // board has blocks, so a mount-time effect ran while the ref was still null,
  // never observed anything, and left the width at 0 — which reads as "one
  // row" and stretched every row to the full pane height.
  const gridRef = useCallback((node: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    if (!node) {
      observerRef.current = null;
      return;
    }
    setGridWidth(node.getBoundingClientRect().width);
    const ro = new ResizeObserver(([entry]) => setGridWidth(entry.contentRect.width));
    ro.observe(node);
    observerRef.current = ro;
  }, []);
  useEffect(() => () => observerRef.current?.disconnect(), []);

  const loadIdRef = useRef(0);
  // Blocks already asked for, scoped to the query they were asked for under.
  // See `planColumnLoads` for why this is query-scoped and why nothing is
  // queued while the block set itself is in flight.
  const requestedRef = useRef<RequestLedger>({ queryKey: '', keys: new Set() });

  const hiddenSet = useMemo(() => {
    const set = new Set<string>();
    for (const p of prefs) {
      if (p.status === 'removed') set.add(hiddenKey(p.accountId, p.filterType, p.filterValue));
    }
    return set;
  }, [prefs]);

  // Hidden tags of this dimension. They still rank, so the stats request asks
  // for this many extra rows and drops them here — the next tags move up.
  const hiddenForType = useMemo(
    () => prefs.filter((p) => p.status === 'removed' && p.filterType === tagType).length,
    [prefs, tagType],
  );

  // Blocks the user hid, then blocks that came back with no threads in the
  // current slice — an empty block tells you nothing and costs a grid cell.
  const unhiddenColumns = useMemo(
    () =>
      capColumns(
        state.columns.filter((c) => !hiddenSet.has(hiddenKey(c.accountId, state.tagType, c.value))),
        TAG_BOARD_MAX_COLUMNS,
      ),
    [state.columns, hiddenSet, state.tagType],
  );
  const visibleColumnsOrdered = useMemo(
    () =>
      // Dedupe before hiding empties: a block left with nothing after an
      // ordinal thread moves to a higher level should disappear, not sit blank.
      selectRenderableColumns(dedupeOrdinalThreads(applySavedOrder(unhiddenColumns, savedOrder), state.tagType)),
    [unhiddenColumns, savedOrder, state.tagType],
  );
  const visibleColumns = visibleColumnsOrdered;
  const hiddenCount = state.columns.filter((c) => hiddenSet.has(hiddenKey(c.accountId, state.tagType, c.value))).length;

  const minColumnPx = DENSITY_MIN_COLUMN_PX[density];
  const oneRow = fitsInOneRow(visibleColumns.length, gridWidth, minColumnPx);

  // Block order is per dimension: the blocks themselves differ between
  // Company and Topic, so one shared list would be meaningless.
  const orderPrefKey = `tagboard_order:${tagType}`;
  useEffect(() => {
    let cancelled = false;
    api
      .getPref(orderPrefKey)
      .then((raw) => {
        if (cancelled) return;
        if (!raw) {
          setSavedOrder([]);
          return;
        }
        try {
          const parsed = JSON.parse(raw);
          setSavedOrder(Array.isArray(parsed) ? parsed.filter((k): k is string => typeof k === 'string') : []);
        } catch {
          // A corrupt pref just means "no saved order" — the ranked default
          // is always a usable fallback.
          setSavedOrder([]);
        }
      })
      .catch(() => !cancelled && setSavedOrder([]));
    return () => {
      cancelled = true;
    };
  }, [orderPrefKey]);

  const persistOrder = useCallback(
    (keys: string[]) => {
      setSavedOrder(keys);
      api.setPref(orderPrefKey, JSON.stringify(keys)).catch((e) => {
        addLog('error', 'system', `Tag board: could not save block order — ${errorText(e)}`);
      });
    },
    [orderPrefKey, addLog],
  );

  const handleDropOnBlock = useCallback(
    (targetKey: string, side: DropSide) => {
      const from = draggingKey;
      setDraggingKey(null);
      setDropTarget(null);
      if (!from || from === targetKey) return;
      // Persist the full visible order, not just the moved pair, so the
      // arrangement survives blocks appearing and disappearing later.
      persistOrder(
        moveColumnKey(
          visibleColumnsOrdered.map((c) => c.key),
          from,
          targetKey,
          side,
        ),
      );
    },
    [draggingKey, persistOrder, visibleColumnsOrdered],
  );

  const loadPage = useCallback(
    async (column: TagBoardColumn, offset: number) => {
      const loadId = loadIdRef.current;
      dispatch({ type: 'PAGE_LOADING', key: column.key });
      try {
        const result = await api.getFilteredEmails(
          column.accountId,
          undefined,
          undefined,
          tagType,
          column.value,
          TAG_BOARD_PAGE_SIZE,
          offset,
          undefined,
          window,
        );
        if (loadIdRef.current !== loadId) return;
        dispatch({ type: 'PAGE_LOADED', key: column.key, emails: result.emails, pageSize: TAG_BOARD_PAGE_SIZE });

        // One call for the whole page. Failure is silent: the cards simply
        // don't name the other participants.
        const threadIds = [...new Set(result.emails.map((e) => e.threadId))];
        if (threadIds.length > 0) {
          api
            .getThreadParticipants(column.accountId, threadIds)
            .then((rows) => {
              if (loadIdRef.current !== loadId) return;
              setParticipants((prev) => {
                const next = { ...prev };
                for (const row of rows) {
                  next[`${column.accountId}\n${row.threadId}`] = row.names;
                }
                return next;
              });
            })
            .catch(() => {});
        }
      } catch (e) {
        if (loadIdRef.current !== loadId) return;
        const message = errorText(e);
        dispatch({ type: 'PAGE_ERROR', key: column.key, error: message });
        addLog('error', 'system', `Tag board: could not load "${column.value}" — ${message}`);
      }
    },
    [tagType, window, addLog],
  );

  /** (Re)read the block set for the current scope, dimension and window. */
  const loadColumns = useCallback(async () => {
    loadIdRef.current += 1;
    const loadId = loadIdRef.current;
    dispatch({ type: 'COLUMNS_LOADING' });
    try {
      const stats = await api.getTagBoardStats(queryAccountId, tagType, window, tagBoardStatsLimit(hiddenForType));
      if (loadIdRef.current !== loadId) return;
      dispatch({ type: 'COLUMNS_LOADED', stats, queryKey });
    } catch (e) {
      if (loadIdRef.current !== loadId) return;
      const message = errorText(e);
      dispatch({ type: 'COLUMNS_ERROR', error: message });
      addLog('error', 'system', `Tag board: could not load ${tagType} tags — ${message}`);
    }
  }, [queryAccountId, tagType, window, queryKey, hiddenForType, addLog]);

  useEffect(() => {
    dispatch({ type: 'SET_TAG_TYPE', tagType });
  }, [tagType]);

  useEffect(() => {
    if (!accountId) return;
    void loadColumns();
  }, [accountId, loadColumns]);

  // Hidden-block prefs and the category set this scope can offer.
  useEffect(() => {
    if (!accountId) return;
    let cancelled = false;
    api
      .getFilterPrefs(queryAccountId)
      .then((p) => !cancelled && setPrefs(p))
      .catch(() => !cancelled && setPrefs([]));
    // Categories are provider-specific (Gmail/Outlook only). Under the unified
    // scope the board spans every enabled account, so offer the union of what
    // they sync rather than whichever account happens to be first.
    const ids = queryAccountId ? [queryAccountId] : allAccounts.filter((a) => a.enabled).map((a) => a.id);
    if (ids.length === 0) return;
    Promise.all(ids.map((id) => api.getAvailableCategories(id).catch(() => [] as string[])))
      .then((lists) => {
        if (cancelled) return;
        const union = new Set<string>();
        for (const list of lists) for (const c of list) union.add(c);
        // Keep the inbox's tab order rather than account-discovery order.
        const ORDER = ['primary', 'social', 'updates', 'forums', 'promotions'];
        setAvailableCategories(ORDER.filter((c) => union.has(c)) as EmailCategory[]);
      })
      .catch(() => !cancelled && setAvailableCategories([]));
    return () => {
      cancelled = true;
    };
  }, [accountId, queryAccountId, allAccounts]);

  // Fill in the first page of every block not yet asked for, a few at a time.
  //
  // The already-requested check MUST happen inside the effect, not in a memo
  // feeding it: React StrictMode double-invokes effects in dev, and a memoized
  // list is identical across both invocations, so the second one re-fired every
  // block's first page — duplicate React keys, phantom blank cards.
  useEffect(() => {
    const plan = planColumnLoads(unhiddenColumns, requestedRef.current, state.queryKey, state.isLoadingColumns);
    requestedRef.current = plan.ledger;
    if (plan.toLoad.length === 0) return;
    void mapWithConcurrency(plan.toLoad, TAG_BOARD_FETCH_CONCURRENCY, (c) => loadPage(c, 0));
  }, [unhiddenColumns, loadPage, state.queryKey, state.isLoadingColumns]);

  const handleLoadMore = useCallback(
    (key: string) => {
      const column = state.columns.find((c) => c.key === key);
      if (!column || column.isLoading) return;
      void loadPage(column, column.emails.length);
    },
    [state.columns, loadPage],
  );

  const handleHide = useCallback(
    async (column: TagBoardColumn) => {
      // Reuses the sidebar's "hide this filter" pref, so a tag hidden here is
      // hidden there too — one notion of "I don't want to see this tag".
      try {
        await api.removeFilter(column.accountId, tagType, column.value);
        setPrefs(await api.getFilterPrefs(queryAccountId));
      } catch (e) {
        addLog('error', 'system', `Tag board: could not hide "${column.value}" — ${errorText(e)}`);
      }
    },
    [tagType, queryAccountId, addLog],
  );

  const handleRestoreHidden = useCallback(async () => {
    const hidden = prefs.filter((p) => p.status === 'removed' && p.filterType === tagType);
    try {
      for (const p of hidden) {
        await api.deleteFilterPref(p.accountId, p.filterType, p.filterValue);
      }
      setPrefs(await api.getFilterPrefs(queryAccountId));
    } catch (e) {
      addLog('error', 'system', `Tag board: could not restore hidden tags — ${errorText(e)}`);
    }
  }, [prefs, tagType, queryAccountId, addLog]);

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden bg-white">
      <TagBoardToolbar
        tagType={tagType}
        onChangeTagType={onChangeTagType}
        range={range}
        onChangeRange={(r) => {
          if (r === 'custom') setCustom((c) => customRangeOnSelect(c, new Date()));
          setRange(r);
        }}
        custom={custom}
        onChangeCustom={setCustom}
        onCommitCustom={() => setCustom((c) => normaliseCustomRange(c))}
        availableCategories={availableCategories}
        selectedCategories={selectedCategories}
        onSelectCategories={setSelectedCategories}
        hiddenCount={hiddenCount}
        onRestoreHidden={() => void handleRestoreHidden()}
        hideJunk={hideJunk}
        onChangeHideJunk={(hide) => void setJunkFlaggedAction(hide ? 'hide' : 'dim')}
        density={density}
        onChangeDensity={onChangeDensity}
        search={search}
        onChangeSearch={setSearch}
        isRefreshing={state.isLoadingColumns}
        onRefresh={() => void loadColumns()}
        onNewChat={onNewChat}
      />

      {state.error !== null && (
        <p className="mx-6 mt-3 rounded border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700">
          {state.error}
        </p>
      )}

      <div className="flex-1 overflow-y-auto p-4">
        {selectIsEmpty(state) && debouncedSearch !== '' ? (
          <div className="mx-auto max-w-md py-16 text-center">
            <h2 className="text-sm font-medium text-gray-900">
              {t('tagboard:noMatches.title', { query: debouncedSearch })}
            </h2>
            <p className="mt-1 text-xs text-gray-500">{t('tagboard:noMatches.body')}</p>
            <button
              type="button"
              onClick={() => setSearch('')}
              className="mt-4 rounded-lg border border-gray-200 px-3 py-1.5 text-xs text-gray-700 hover:bg-gray-50"
            >
              {t('tagboard:noMatches.clear')}
            </button>
          </div>
        ) : selectIsEmpty(state) ? (
          <div className="mx-auto max-w-md py-16 text-center">
            <h2 className="text-sm font-medium text-gray-900">{t('tagboard:empty.title')}</h2>
            <p className="mt-1 text-xs text-gray-500">{t('tagboard:empty.body')}</p>
            <button
              type="button"
              onClick={onOpenClassificationSettings}
              className="mt-4 rounded-lg bg-primary-600 px-3 py-1.5 text-xs text-white hover:bg-primary-700"
            >
              {t('tagboard:empty.openSettings')}
            </button>
          </div>
        ) : (
          <div
            ref={gridRef}
            style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${minColumnPx}px, 1fr))` }}
            className={`grid gap-3 ${
              oneRow
                ? // A single row fills the pane rather than leaving dead space.
                  'min-h-full auto-rows-[minmax(16rem,1fr)]'
                : // Several rows: cap each so more than one is visible at once.
                  'auto-rows-[minmax(16rem,24rem)]'
            }`}
          >
            {visibleColumns.map((column) => (
              <TagColumn
                key={column.key}
                column={column}
                tagType={state.tagType}
                accountEmail={emailByAccount.get(column.accountId) ?? column.accountId}
                selectedEmailId={selectedEmailId}
                onSelectEmail={onSelectEmail}
                participants={participants}
                cardActions={cardActions}
                onLoadMore={handleLoadMore}
                onOpenInInbox={(c) => onOpenTagInInbox(state.tagType, c.value)}
                onHide={(c) => void handleHide(c)}
                dropSide={
                  dropTarget?.key === column.key && draggingKey !== null && draggingKey !== column.key
                    ? dropTarget.side
                    : null
                }
                onDragStartBlock={setDraggingKey}
                onDragOverBlock={(key, side) => setDropTarget({ key, side })}
                onDropOnBlock={handleDropOnBlock}
                onDragEndBlock={() => {
                  setDraggingKey(null);
                  setDropTarget(null);
                }}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
