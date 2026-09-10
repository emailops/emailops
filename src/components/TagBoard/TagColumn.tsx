import type React from 'react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { tagHeaderBackground } from '@/components/common/TagChips';
import { accountColorClass } from '@/lib/colors';
import type { DropSide, TagBoardColumn, TagBoardType } from '@/lib/tagBoard';
import type { Email } from '@/types';
import { TagEmailCard, type TagEmailCardProps } from './TagEmailCard';

interface TagColumnProps {
  column: TagBoardColumn;
  tagType: TagBoardType;
  /** Address of the account this block belongs to — half of its title. */
  accountEmail: string;
  selectedEmailId: string | null;
  onSelectEmail: (email: Email) => void;
  /** Thread participants keyed `accountId\nthreadId`. */
  participants: Record<string, string[]>;
  /** Per-card ⋮ actions, same set as the inbox row. */
  cardActions?: TagEmailCardProps['actions'];
  /** Load the block's next page. */
  onLoadMore: (key: string) => void;
  /** Apply this tag as a smart filter and jump to the inbox. */
  onOpenInInbox: (column: TagBoardColumn) => void;
  /** Stop showing this (account, tag) block. */
  onHide: (column: TagBoardColumn) => void;
  /** Drag-to-reorder. `dropSide` names the gap the drop would land in, or
   *  null when this block is not the current target. */
  dropSide: DropSide | null;
  onDragStartBlock: (key: string) => void;
  onDragOverBlock: (key: string, side: DropSide) => void;
  onDropOnBlock: (key: string, side: DropSide) => void;
  onDragEndBlock: () => void;
}

/** Which half of the block the pointer is over — the gap the drop lands in. */
function sideFromPointer(e: React.DragEvent<HTMLElement>): DropSide {
  const rect = e.currentTarget.getBoundingClientRect();
  return e.clientX < rect.left + rect.width / 2 ? 'before' : 'after';
}

export function TagColumn({
  column,
  tagType,
  accountEmail,
  selectedEmailId,
  onSelectEmail,
  participants,
  cardActions,
  onLoadMore,
  onOpenInInbox,
  onHide,
  dropSide,
  onDragStartBlock,
  onDragOverBlock,
  onDropOnBlock,
  onDragEndBlock,
}: TagColumnProps) {
  const { t } = useTranslation(['tagboard', 'common']);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  // A block awaiting its turn in the fetch queue has isLoading === false but
  // has never been paged. Treating that as "empty" rendered a blank block with
  // a "No threads" line — most visible right after a category change, when
  // every block is re-queued at once.
  const isPending = !column.hasLoaded && column.error === null;
  const isEmpty = column.hasLoaded && column.emails.length === 0 && column.error === null;

  // Dismiss the menu on any outside click, so it can't be left hanging over
  // a neighbouring block.
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [menuOpen]);

  return (
    <section
      onDragOver={(e) => {
        // Required for the drop to be allowed at all.
        e.preventDefault();
        onDragOverBlock(column.key, sideFromPointer(e));
      }}
      onDrop={(e) => {
        e.preventDefault();
        onDropOnBlock(column.key, sideFromPointer(e));
      }}
      className="relative flex min-h-0 min-w-0 flex-col rounded-xl border border-gray-200 bg-gray-50"
    >
      {/* The drop lands in a gap, so the gap is what lights up: a bar centred
          in the 12px gutter on the chosen side, rather than a ring around a
          block that is not itself the destination. `-inset-y-1` overshoots the
          block edges so the bar reads as filling the channel. */}
      {dropSide !== null && (
        <span
          aria-hidden="true"
          className={`pointer-events-none absolute -inset-y-1 z-10 w-1 rounded-full bg-primary-500 ${
            dropSide === 'before' ? '-left-2' : '-right-2'
          }`}
        />
      )}
      {/* Title bar carries the block's own colour so blocks are tellable apart
          at a glance when many are on screen. */}
      {/* Near-black text on the tag's tint. The chip's own 700-weight
          foreground is tuned for a small pill and reads washed out across a
          full-width header — amber especially. */}
      <header
        draggable
        onDragStart={(e) => {
          e.dataTransfer.effectAllowed = 'move';
          // Some browsers refuse to start a drag without payload.
          e.dataTransfer.setData('text/plain', column.key);
          onDragStartBlock(column.key);
        }}
        onDragEnd={onDragEndBlock}
        title={t('tagboard:dragHint')}
        className={`flex flex-shrink-0 cursor-grab items-center gap-2 rounded-t-xl px-2.5 py-2 text-gray-900 active:cursor-grabbing ${tagHeaderBackground(tagType, column.value)}`}
      >
        <span className={`h-2.5 w-2.5 flex-shrink-0 rounded-full ${accountColorClass(column.accountId)}`} />
        <button
          type="button"
          onClick={() => onOpenInInbox(column)}
          title={`${accountEmail} · ${column.value}`}
          className="flex min-w-0 flex-1 flex-col items-start text-left hover:underline"
        >
          <span className="w-full truncate text-[11px] font-medium text-gray-600">{accountEmail}</span>
          <span className="w-full truncate text-sm font-semibold">{column.value}</span>
        </button>
        <span className="flex-shrink-0 text-[11px] text-gray-600">
          {t('tagboard:threadCount', { count: column.threadCount })}
        </span>

        <div className="relative flex-shrink-0" ref={menuRef}>
          <button
            type="button"
            onClick={() => setMenuOpen((v) => !v)}
            aria-label={t('tagboard:blockMenu')}
            aria-expanded={menuOpen}
            className="rounded p-1 text-gray-600 transition-colors hover:bg-black/10 hover:text-gray-900"
          >
            <svg className="h-4 w-4" viewBox="0 0 20 20" fill="currentColor">
              <path d="M10 6a1.5 1.5 0 110-3 1.5 1.5 0 010 3zm0 5.5a1.5 1.5 0 110-3 1.5 1.5 0 010 3zm0 5.5a1.5 1.5 0 110-3 1.5 1.5 0 010 3z" />
            </svg>
          </button>
          {menuOpen && (
            <div className="absolute right-0 top-full z-20 mt-1 w-44 overflow-hidden rounded-lg border border-gray-200 bg-white py-1 text-gray-700 shadow-lg">
              <button
                type="button"
                onClick={() => {
                  setMenuOpen(false);
                  onHide(column);
                }}
                className="block w-full px-3 py-1.5 text-left text-xs hover:bg-gray-100"
              >
                {t('tagboard:hideTag')}
              </button>
              <button
                type="button"
                onClick={() => {
                  setMenuOpen(false);
                  onOpenInInbox(column);
                }}
                className="block w-full px-3 py-1.5 text-left text-xs hover:bg-gray-100"
              >
                {t('tagboard:openInInbox')}
              </button>
            </div>
          )}
        </div>
      </header>

      {/* min-h-0 lets this shrink inside the flex column so its own scrollbar
          appears instead of the block overflowing its grid row. */}
      {/* Flex column, not `space-y-*`: a <button> is inline-level, so as a block
          sibling it sits in a line box and picks up line-height leading —
          which rendered as ~50px of blank between cards. Flex children are
          block-level and the gap is exact. */}
      <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto rounded-b-xl p-1.5">
        {column.emails.map((email) => (
          <TagEmailCard
            key={`${email.accountId}:${email.id}`}
            email={email}
            isSelected={email.id === selectedEmailId}
            participants={participants[`${email.accountId}\n${email.threadId}`]}
            ownerEmail={accountEmail}
            onSelect={onSelectEmail}
            actions={cardActions}
          />
        ))}

        {isPending && column.emails.length === 0 && (
          <div className="space-y-1 p-1" aria-hidden="true">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-12 animate-pulse rounded-lg bg-gray-200/70" />
            ))}
          </div>
        )}

        {isEmpty && <p className="px-2 py-3 text-xs text-gray-400">{t('tagboard:columnEmpty')}</p>}

        {column.error !== null && (
          <div className="space-y-1 px-2 py-3">
            <p className="text-xs text-red-600" title={column.error}>
              {t('tagboard:columnError')}
            </p>
            <button
              type="button"
              onClick={() => onLoadMore(column.key)}
              className="text-xs text-primary-600 hover:text-primary-700 hover:underline"
            >
              {t('tagboard:retry')}
            </button>
          </div>
        )}

        {column.hasMore && column.emails.length > 0 && (
          <button
            type="button"
            onClick={() => onLoadMore(column.key)}
            disabled={column.isLoading}
            className="w-full px-2 py-1.5 text-xs text-primary-600 hover:text-primary-700 hover:underline disabled:opacity-50"
          >
            {column.isLoading ? t('common:state.loading') : t('tagboard:showMore')}
          </button>
        )}
      </div>
    </section>
  );
}
