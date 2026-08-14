import { useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { usePersistedPref } from '@/hooks/usePersistedPref';
import type { ChatContext } from '@/lib/chatContext';
import {
  CHAT_PANEL_DEFAULT_WIDTH,
  CHAT_PANEL_MAX_WIDTH,
  CHAT_PANEL_MIN_WIDTH,
  clampChatPanelWidth,
  parseChatPanelWidth,
} from '@/lib/chatPanelLayout';
import { ChatPanel } from './ChatPanel';

interface ChatPanelDockProps {
  accountId: string | null;
  /** Retarget which account the chat searches. */
  onAccountChange: (accountId: string) => void;
  context: ChatContext | null;
  onClose: () => void;
  onExpand: () => void;
  onNavigateToInbox?: () => void;
}

/** Keyboard resize step, matching the arrow-key convention of a native splitter. */
const KEYBOARD_STEP = 16;

/**
 * Docks {@link ChatPanel} against the right edge of the window and owns its
 * width: persisted to SQLite, drag-resizable from the left border, and
 * keyboard-resizable for anyone not using a pointer.
 */
export function ChatPanelDock({
  accountId,
  onAccountChange,
  context,
  onClose,
  onExpand,
  onNavigateToInbox,
}: ChatPanelDockProps) {
  const { t } = useTranslation('chat');
  const [width, setWidth] = usePersistedPref<number>('chat_panel_width', CHAT_PANEL_DEFAULT_WIDTH, {
    parse: parseChatPanelWidth,
    serialize: (v) => String(v),
  });
  const draggingRef = useRef(false);

  // Drag from the left border: the panel is right-anchored, so its width is
  // the distance from the pointer to the window's right edge.
  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    draggingRef.current = true;
  }, []);

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      if (!draggingRef.current) return;
      e.preventDefault();
      setWidth(clampChatPanelWidth(window.innerWidth - e.clientX));
    };
    const stop = () => {
      draggingRef.current = false;
    };
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
  }, [setWidth]);

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
      <div className="min-w-0 flex-1">
        <ChatPanel
          onAccountChange={onAccountChange}
          accountId={accountId}
          context={context}
          onClose={onClose}
          onExpand={onExpand}
          onNavigateToInbox={onNavigateToInbox}
        />
      </div>
    </div>
  );
}
