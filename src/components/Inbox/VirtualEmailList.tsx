import { useVirtualizer } from '@tanstack/react-virtual';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { measuredRowHeight } from '@/lib/rowMeasure';
import {
  INITIAL_SCROLL_RESTORE,
  planOffsetResync,
  planScrollRestore,
  type ScrollRestoreState,
} from '@/lib/scrollRestore';
import { useSelectionStore } from '@/stores/selectionStore';
import type { Email } from '@/types';
import type { RulePrefill } from './EmailRow';
import { EmailRow } from './EmailRow';

const ESTIMATED_ROW_HEIGHT = 130; // Accounts for tag chips + unread bold text
const ESTIMATED_COMPACT_ROW_HEIGHT = 48; // Single-line Gmail-style row (incl. chip slot reserved height)

interface VirtualEmailListProps {
  emails: Email[];
  selectedEmailId: string | null;
  focusEmailId: string | null;
  /** The keyboard cursor row (j/k): kept in view, and marked when `showCursor`. */
  cursorEmailId?: string | null;
  /** Mark the cursor row — the full-width list, where no row is "open". */
  showCursor?: boolean;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  isLoadingMore: boolean;
  hasMore: boolean;
  isSyncing: boolean;
  emptyStateMessage?: string;
  onSelectEmail: (email: Email) => void;
  onLoadMore: () => void;
  onAddSenderFilter?: (senderEmail: string) => void;
  onHideSenderFromFilters?: (senderEmail: string) => void;
  onCreateAttachmentRule?: (prefill: RulePrefill) => void;
  onCreateClassificationRule?: (prefill: RulePrefill) => void;
  onOpenInTab?: (email: Email) => void;
  onChatAboutThread?: (email: Email) => void;
  compact?: boolean;
  /** Unified ("All accounts") mode: per-account color bar. Returns undefined
   *  outside unified mode so rows render exactly as before. */
  getAccountBadge?: (email: Email) => { colorClass: string; label: string; chip?: boolean } | undefined;
}

/**
 * Windowed email list. The parent owns the scroll container ref so it can also
 * drive its own auto-load-more / scroll heuristics — this component reads the
 * same ref to wire up the virtualizer and infinite-scroll trigger.
 */
export function VirtualEmailList({
  emails,
  selectedEmailId,
  focusEmailId,
  cursorEmailId = null,
  showCursor = false,
  scrollContainerRef,
  isLoadingMore,
  hasMore,
  isSyncing,
  emptyStateMessage,
  onSelectEmail,
  onLoadMore,
  onAddSenderFilter,
  onHideSenderFromFilters,
  onCreateAttachmentRule,
  onCreateClassificationRule,
  onOpenInTab,
  onChatAboutThread,
  compact = false,
  getAccountBadge,
}: VirtualEmailListProps) {
  const { t } = useTranslation(['inbox']);

  // Multi-select. The selection lives in a store (the bulk toolbar and
  // keyboard shortcuts act on it too); rows that leave the list leave it.
  const selectedIds = useSelectionStore((s) => s.ids);
  const toggleSelected = useSelectionStore((s) => s.toggle);
  const selectRange = useSelectionStore((s) => s.selectRange);
  const pruneSelection = useSelectionStore((s) => s.prune);
  const clearSelection = useSelectionStore((s) => s.clear);
  const order = useMemo(() => emails.map((e) => e.id), [emails]);
  const selectionActive = selectedIds.size > 0;
  useEffect(() => pruneSelection(order), [order, pruneSelection]);
  useEffect(() => {
    if (!selectionActive) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      clearSelection();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [selectionActive, clearSelection]);

  const virtualizer = useVirtualizer({
    count: emails.length,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: () => (compact ? ESTIMATED_COMPACT_ROW_HEIGHT : ESTIMATED_ROW_HEIGHT),
    // Key by email ID so the measurement cache survives list updates
    getItemKey: (index) => emails[index].id,
    overscan: 5,
    // A row inside a `display: none` subtree reports a zero-height box, and the
    // library would cache that as the row's size — see src/lib/rowMeasure.ts.
    measureElement: (element, entry, instance) => {
      const box = entry?.borderBoxSize?.[0];
      const index = instance.indexFromElement(element);
      return measuredRowHeight({
        measured: box ? Math.round(box.blockSize) : element.getBoundingClientRect().height,
        cached: instance.itemSizeCache.get(instance.options.getItemKey(index)),
        estimate: instance.options.estimateSize(index),
      });
    },
  });

  // Survive the `display: none` hide that full-width layout applies while an
  // email is open (App.tsx keeps the inbox mounted rather than unmounting it).
  //
  // `display: none` resets the container's scrollTop to 0 WITHOUT firing a
  // scroll event, and the virtualizer only learns its offset from scroll events
  // — so on the way back it kept rendering the window for the pre-hide offset
  // while the container sat at 0, leaving a blank band above the rows until the
  // user scrolled. Writing the saved scrollTop back on re-show returns the user
  // to where they were, and usually resyncs the virtualizer for free through the
  // scroll event it fires — `resyncVirtualizer` below covers the cases where it
  // fires none.
  //
  // Visibility is read from layout (`clientHeight === 0`) rather than a prop,
  // because what matters is what the browser actually did to scrollTop.
  const restoreRef = useRef<ScrollRestoreState>(INITIAL_SCROLL_RESTORE);

  // Put the virtualizer back in step with the container whenever the two have
  // drifted apart. Restoring scrollTop is not enough on its own: the write only
  // resyncs the virtualizer through the scroll event it happens to fire, and it
  // fires none when the container is already at that value or when the browser
  // clamps the write against content that is momentarily shorter.
  const resyncVirtualizer = useCallback(
    (el: HTMLDivElement) => {
      const shouldResync = planOffsetResync({
        hidden: el.clientHeight === 0,
        scrollTop: el.scrollTop,
        virtualOffset: virtualizer.scrollOffset ?? 0,
      });
      if (!shouldResync) return;
      // A scroll event is the only channel virtual-core reads the container
      // through. Writing scrollTop or calling scrollToOffset cannot stand in for
      // it: both are no-ops once the DOM already holds the value, which is
      // exactly the situation that strands the offset.
      el.dispatchEvent(new Event('scroll'));
    },
    [virtualizer],
  );

  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    let rafId: number | null = null;

    const observe = () => {
      const plan = planScrollRestore(restoreRef.current, {
        hidden: el.clientHeight === 0,
        scrollTop: el.scrollTop,
      });
      restoreRef.current = plan.state;
      if (plan.restoreTo !== null) {
        el.scrollTop = plan.restoreTo;
      }
      resyncVirtualizer(el);
      // ...and again once the frame has settled: rows re-measure on the way back
      // from hidden, so both the spacer height that bounds the restore write and
      // the virtualizer's own offset are still moving at this point.
      if (rafId === null) {
        rafId = requestAnimationFrame(() => {
          rafId = null;
          resyncVirtualizer(el);
        });
      }
    };

    // ResizeObserver is the reliable signal for both directions: hiding collapses
    // the box to 0x0 and showing restores it, and it fires for ancestor
    // display changes too.
    const resizeObserver = new ResizeObserver(observe);
    resizeObserver.observe(el);
    el.addEventListener('scroll', observe, { passive: true });

    return () => {
      if (rafId !== null) cancelAnimationFrame(rafId);
      resizeObserver.disconnect();
      el.removeEventListener('scroll', observe);
    };
  }, [scrollContainerRef, resyncVirtualizer]);

  // Scroll focused email into view using the virtualizer (avoids inline ref callbacks)
  useEffect(() => {
    if (!focusEmailId) return;
    const index = emails.findIndex((e) => e.id === focusEmailId);
    if (index === -1) return;
    const raf = requestAnimationFrame(() => {
      virtualizer.scrollToIndex(index, { align: 'center', behavior: 'smooth' });
    });
    return () => cancelAnimationFrame(raf);
  }, [focusEmailId, emails, virtualizer]);

  // Keep the keyboard cursor row in view as j/k move it. `auto` scrolls only
  // when the row is off screen, so walking a visible page does not jump.
  useEffect(() => {
    if (!cursorEmailId) return;
    const index = emails.findIndex((e) => e.id === cursorEmailId);
    if (index === -1) return;
    virtualizer.scrollToIndex(index, { align: 'auto' });
  }, [cursorEmailId, emails, virtualizer]);

  // Infinite scroll: load more when near bottom
  const virtualItems = virtualizer.getVirtualItems();
  const lastVirtualItem = virtualItems[virtualItems.length - 1];
  useEffect(() => {
    if (!lastVirtualItem) return;
    if (lastVirtualItem.index >= emails.length - 5 && hasMore && !isLoadingMore) {
      onLoadMore();
    }
  }, [lastVirtualItem?.index, emails.length, hasMore, isLoadingMore, onLoadMore, lastVirtualItem]);

  if (emails.length === 0) {
    const message = emptyStateMessage ?? (isSyncing ? 'Syncing emails...' : 'No emails match the selected filters');
    return (
      <div ref={scrollContainerRef as React.LegacyRef<HTMLDivElement>} className="flex-1 overflow-y-auto">
        <div className="p-8 text-center">
          <svg className="mx-auto h-10 w-10 text-gray-300" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1}
              d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
            />
          </svg>
          <p className="mt-3 text-sm text-gray-500">{message}</p>
        </div>
      </div>
    );
  }

  return (
    <div ref={scrollContainerRef as React.LegacyRef<HTMLDivElement>} className="flex-1 overflow-y-auto">
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: '100%',
          position: 'relative',
        }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const email = emails[virtualRow.index];
          return (
            <div
              key={virtualRow.key}
              ref={virtualizer.measureElement}
              data-index={virtualRow.index}
              // Opaque background + clipping protects against visual overlap:
              // if a row's content ever grows past its measured height (e.g.
              // during the frame ResizeObserver hasn't caught up), the alpha
              // background on unread rows below would otherwise expose the
              // overflow as "double text". Clipping prevents that bleed and
              // keeps rows visually independent of each other.
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                width: '100%',
                backgroundColor: 'white',
                overflow: 'hidden',
                transform: `translateY(${virtualRow.start}px)`,
              }}
              data-cursor={showCursor && email.id === cursorEmailId ? 'true' : undefined}
            >
              {showCursor && email.id === cursorEmailId && (
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 z-10 ring-2 ring-inset ring-primary-400"
                />
              )}
              <EmailRow
                email={email}
                isSelected={email.id === selectedEmailId}
                onClick={() => onSelectEmail(email)}
                onAddSenderFilter={onAddSenderFilter}
                onHideSenderFromFilters={onHideSenderFromFilters}
                onCreateAttachmentRule={onCreateAttachmentRule}
                onCreateClassificationRule={onCreateClassificationRule}
                onOpenInTab={onOpenInTab}
                onChatAboutThread={onChatAboutThread}
                compact={compact}
                accountBadge={getAccountBadge?.(email)}
                isChecked={selectedIds.has(email.id)}
                selectionActive={selectionActive}
                onCheck={({ range }) => (range ? selectRange(order, email.id) : toggleSelected(email.id))}
              />
            </div>
          );
        })}
      </div>
      {isLoadingMore && (
        <div className="p-4 text-center">
          <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary-600 mx-auto" />
          <p className="mt-2 text-xs text-gray-500">{t('inbox:loadingMoreEmails')}</p>
        </div>
      )}
      {!hasMore && emails.length > 0 && (
        <div className="p-4 text-center text-xs text-gray-400">{t('inbox:noMoreEmails')}</div>
      )}
    </div>
  );
}
