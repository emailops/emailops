import { useTranslation } from 'react-i18next';
import type { EmailTab } from '@/stores/emailStore';

interface EmailTabBarProps {
  /** The inbox-selected email shown in the main (replaceable) tab. */
  mainEmail: { subject: string } | null;
  isMainTabActive: boolean;
  tabs: EmailTab[];
  activeTabId: string | null;
  onSelectMainTab: () => void;
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
}

export function EmailTabBar({
  mainEmail,
  isMainTabActive,
  tabs,
  activeTabId,
  onSelectMainTab,
  onSelectTab,
  onCloseTab,
}: EmailTabBarProps) {
  const { t } = useTranslation(['inbox']);
  return (
    <div className="flex items-center overflow-x-auto border-b border-gray-200 bg-gray-50 flex-shrink-0 min-h-0">
      {/* Main tab — always first, no close button, content replaced by inbox clicks */}
      {mainEmail && (
        <button
          onClick={onSelectMainTab}
          className={`flex items-center gap-1.5 px-3 py-2 text-xs whitespace-nowrap border-r border-gray-200 flex-shrink-0 max-w-48 transition-colors ${
            isMainTabActive
              ? 'bg-white text-gray-900 border-b-2 border-b-primary-600 -mb-px'
              : 'text-gray-500 hover:bg-gray-100 hover:text-gray-700'
          }`}
        >
          <span className="truncate">{mainEmail.subject || '(no subject)'}</span>
        </button>
      )}

      {/* Persistent tabs — immutable content, closeable */}
      {tabs.map((tab) => {
        const isActive = tab.id === activeTabId;
        const title =
          tab.type === 'thread'
            ? tab.subject || '(no subject)'
            : tab.type === 'attachment'
              ? tab.filename
              : tab.subject || 'New Email';
        return (
          <div
            key={tab.id}
            role="tab"
            tabIndex={0}
            aria-selected={isActive}
            onClick={() => onSelectTab(tab.id)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') onSelectTab(tab.id);
            }}
            className={`group flex items-center gap-1.5 px-3 py-2 text-xs whitespace-nowrap border-r border-gray-200 flex-shrink-0 max-w-48 transition-colors cursor-pointer ${
              isActive
                ? 'bg-white text-gray-900 border-b-2 border-b-primary-600 -mb-px'
                : 'text-gray-500 hover:bg-gray-100 hover:text-gray-700'
            }`}
          >
            {tab.type === 'attachment' && <TabAttachmentIcon mimeType={tab.mimeType} />}
            {tab.type === 'compose' && <TabComposeIcon />}
            <span className="truncate">{title}</span>
            <button
              type="button"
              aria-label={t('inbox:emailView.closeTab')}
              onClick={(e) => {
                e.stopPropagation();
                onCloseTab(tab.id);
              }}
              className="flex-shrink-0 rounded p-0.5 text-gray-400 hover:bg-gray-200 hover:text-gray-600 opacity-0 group-hover:opacity-100 transition-opacity"
            >
              <svg className="h-3 w-3" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth={2}>
                <path d="M2 2l8 8M10 2l-8 8" />
              </svg>
            </button>
          </div>
        );
      })}
    </div>
  );
}

function TabComposeIcon() {
  return (
    <svg className="w-3 h-3 flex-shrink-0 text-primary-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
      />
    </svg>
  );
}

function TabAttachmentIcon({ mimeType }: { mimeType: string }) {
  if (mimeType === 'application/pdf') {
    return (
      <svg className="w-3 h-3 flex-shrink-0 text-red-400" fill="currentColor" viewBox="0 0 20 20">
        <path
          fillRule="evenodd"
          d="M4 4a2 2 0 012-2h4.586A2 2 0 0112 2.586L15.414 6A2 2 0 0116 7.414V16a2 2 0 01-2 2H6a2 2 0 01-2-2V4zm2 6a1 1 0 011-1h6a1 1 0 110 2H7a1 1 0 01-1-1zm1 3a1 1 0 100 2h6a1 1 0 100-2H7z"
          clipRule="evenodd"
        />
      </svg>
    );
  }
  if (mimeType === 'text/html') {
    return (
      <svg className="w-3 h-3 flex-shrink-0 text-orange-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
      </svg>
    );
  }
  return (
    <svg className="w-3 h-3 flex-shrink-0 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
      />
    </svg>
  );
}
