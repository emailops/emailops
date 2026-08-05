import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ChatConversation } from '@/types';

interface ConversationListProps {
  conversations: ChatConversation[];
  activeId: string | null;
  isLoading: boolean;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
}

export function ConversationList({
  conversations,
  activeId,
  isLoading,
  onSelect,
  onCreate,
  onRename,
  onDelete,
}: ConversationListProps) {
  const { t } = useTranslation(['chat', 'common']);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState('');
  // Two-click delete confirm: first click arms the row (shows "Confirm?"),
  // second click within the timeout actually deletes. Avoids window.confirm()
  // which blocks the Tauri webview and feels out of place on desktop.
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  // Auto-disarm the pending delete after a few seconds so the UI doesn't
  // stay in an armed state indefinitely.
  useEffect(() => {
    if (!pendingDeleteId) return;
    const timer = window.setTimeout(() => setPendingDeleteId(null), 4000);
    return () => window.clearTimeout(timer);
  }, [pendingDeleteId]);

  const beginEdit = (c: ChatConversation) => {
    setEditingId(c.id);
    setDraftTitle(c.title);
  };

  const commitEdit = () => {
    if (editingId && draftTitle.trim()) {
      onRename(editingId, draftTitle.trim());
    }
    setEditingId(null);
    setDraftTitle('');
  };

  const handleDeleteClick = (id: string) => {
    if (pendingDeleteId === id) {
      onDelete(id);
      setPendingDeleteId(null);
    } else {
      setPendingDeleteId(id);
    }
  };

  return (
    <aside
      // Full width on a phone, fixed column from `md` up. When the stacked chat
      // layout shows this list it is the *only* pane, so a fixed 16rem column
      // left a dead blank strip across the rest of the screen.
      className="w-full md:w-64 flex-shrink-0 border-r border-gray-200 bg-gray-50 flex flex-col"
    >
      <div className="p-3 border-b border-gray-200">
        <button
          onClick={onCreate}
          className="w-full flex items-center justify-center gap-2 px-3 py-2 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors"
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          {t('chat:conversations.newChat')}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <p className="p-4 text-xs text-gray-500">{t('common:state.loading')}</p>
        ) : conversations.length === 0 ? (
          <p className="p-4 text-xs text-gray-500">{t('chat:conversations.empty')}</p>
        ) : (
          <ul className="py-2">
            {conversations.map((c) => {
              const isActive = c.id === activeId;
              const isEditing = c.id === editingId;
              return (
                <li key={c.id} className="px-2">
                  <div
                    className={`group flex items-center gap-1 rounded-lg text-sm transition-colors ${
                      isActive ? 'bg-primary-100 text-primary-900' : 'text-gray-700 hover:bg-gray-200'
                    }`}
                  >
                    {isEditing ? (
                      <input
                        // biome-ignore lint/a11y/noAutofocus: rename input is only rendered on explicit user action (double-click)
                        autoFocus
                        value={draftTitle}
                        onChange={(e) => setDraftTitle(e.target.value)}
                        onBlur={commitEdit}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') commitEdit();
                          if (e.key === 'Escape') {
                            setEditingId(null);
                            setDraftTitle('');
                          }
                        }}
                        className="flex-1 px-2 py-1.5 bg-white border border-primary-400 rounded text-sm focus:outline-none"
                      />
                    ) : (
                      <button
                        onClick={() => onSelect(c.id)}
                        onDoubleClick={() => beginEdit(c)}
                        className="flex-1 text-left px-3 py-2 truncate"
                        title={c.title}
                      >
                        {c.title}
                      </button>
                    )}
                    {!isEditing && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDeleteClick(c.id);
                        }}
                        className={`mr-1 p-1 rounded transition-colors flex-shrink-0 ${
                          pendingDeleteId === c.id
                            ? 'text-white bg-red-600 hover:bg-red-700'
                            : 'text-gray-400 hover:text-red-600 hover:bg-white'
                        }`}
                        title={pendingDeleteId === c.id ? 'Click again to confirm' : 'Delete conversation'}
                      >
                        {pendingDeleteId === c.id ? (
                          <span className="text-[10px] font-semibold px-1 leading-4 whitespace-nowrap">
                            {t('chat:conversations.confirmDelete')}
                          </span>
                        ) : (
                          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6M1 7h22M9 7V4a1 1 0 011-1h4a1 1 0 011 1v3"
                            />
                          </svg>
                        )}
                      </button>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </aside>
  );
}
