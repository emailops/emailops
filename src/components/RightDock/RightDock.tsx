import { type ReactNode, useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { usePersistedPref } from '@/hooks/usePersistedPref';
import {
  CHAT_PANEL_DEFAULT_WIDTH,
  CHAT_PANEL_MAX_WIDTH,
  CHAT_PANEL_MIN_WIDTH,
  clampChatPanelWidth,
  parseChatPanelWidth,
} from '@/lib/chatPanelLayout';

interface RightDockProps {
  children: ReactNode;
}

/** Keyboard resize step, matching the arrow-key convention of a native splitter. */
const KEYBOARD_STEP = 16;

/**
 * Docks its content (the chat panel or the calendar agenda) against the right
 * edge of the window and owns the width: persisted to SQLite under the
 * historical `chat_panel_width` pref so an existing layout carries over,
 * drag-resizable from the left border, keyboard-resizable for anyone not
 * using a pointer.
 */
export function RightDock({ children }: RightDockProps) {
  const { t } = useTranslation('chat');
  const [width, setWidth] = usePersistedPref<number>('chat_panel_width', CHAT_PANEL_DEFAULT_WIDTH, {
    parse: parseChatPanelWidth,
    serialize: (v) => String(v),
  });
  const [dragging, setDragging] = useState(false);

  // Publish the dock's width so a non-blocking dialog can keep clear of it
  // (see `Modal`'s `nonBlocking`). A CSS variable rather than a prop: the
  // dialogs that need it are mounted in unrelated subtrees, and threading the
  // width to each of them would couple every form to the dock.
  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty('--chat-dock-width', `${width}px`);
    return () => {
      root.style.removeProperty('--chat-dock-width');
    };
  }, [width]);

  // Drag from the left border: the panel is right-anchored, so its width is
  // the distance from the pointer to the window's right edge.
  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragging(true);
  }, []);

  useEffect(() => {
    if (!dragging) return;
    const onMove = (e: PointerEvent) => {
      e.preventDefault();
      setWidth(clampChatPanelWidth(window.innerWidth - e.clientX));
    };
    const stop = () => setDragging(false);
    // Listen on window, not the handle: the pointer routinely outruns a 4px
    // target mid-drag, and losing the move events would freeze the resize.
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
  }, [dragging, setWidth]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    // ArrowLeft grows the panel (its left border moves left).
    setWidth((w) => clampChatPanelWidth(w + (e.key === 'ArrowLeft' ? KEYBOARD_STEP : -KEYBOARD_STEP)));
  };

  return (
    <div className="flex flex-shrink-0 overflow-hidden border-l border-gray-200" style={{ width }}>
      {/* A focusable separator carrying aria-valuenow is the ARIA-correct
          splitter; there is no native element for it. */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t('panel.resize')}
        aria-valuenow={width}
        aria-valuemin={CHAT_PANEL_MIN_WIDTH}
        aria-valuemax={CHAT_PANEL_MAX_WIDTH}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
        className="w-1 flex-shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-primary-300 focus:bg-primary-400 focus:outline-none"
      />
      <div className="min-w-0 flex-1">{children}</div>
      {/* While dragging, a layer over the whole window keeps the pointer in
          this document: over an iframe (a PDF, an email body) the window gets
          no pointermove events and the resize froze mid-drag. */}
      {dragging &&
        createPortal(
          <div data-testid="right-dock-drag-capture" className="fixed inset-0 z-[9999] cursor-col-resize" />,
          document.body,
        )}
    </div>
  );
}
