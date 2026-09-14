import { AttachmentTabView } from '@/components/EmailView/AttachmentTabView';
import { ComposeTabView } from '@/components/EmailView/ComposeTabView';
import { EmailTabBar } from '@/components/EmailView/EmailTabBar';
import { EmailView } from '@/components/EmailView/EmailView';
import type { EmailTab } from '@/stores/emailStore';
import type { Account, Email } from '@/types';

interface ReadingPaneProps {
  /** Thread shown when no tab is active (the "main" selection). */
  threadEmails: Email[];
  isLoading: boolean;
  /** Drives the tab bar's main-tab entry. */
  selectedEmail: Email | null;
  tabs: EmailTab[];
  activeTabId: string | null;
  activeTab: EmailTab | null;
  accounts: Account[];
  /** null in unified mode — EmailView falls back to the thread's own account. */
  activeAccountId: string | null;
  onSelectMainTab: () => void;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  /** Close the main (non-tab) email. */
  onCloseMain: () => void;
  /** Full-width layout: closing anything returns to the list. */
  fullWidth?: boolean;
  /** Only meaningful for the main email; omitted where tabs don't apply. */
  onOpenInTab?: () => void;
  /** Header chat icon: open a chat seeded with the shown thread. */
  onChatAboutThread?: (email: Email) => void;
  className?: string;
}

/**
 * The email-reading half of a list view: tab bar, plus whichever of the
 * thread, an attachment or a compose draft the active tab holds.
 *
 * Shared because the tag board originally rendered a bare `EmailView` and so
 * had no way to display an attachment tab — opening a PDF pushed a tab that
 * nothing rendered, and the file silently vanished. Any surface that shows an
 * email needs the whole set, not just the thread view.
 */
export function ReadingPane({
  threadEmails,
  isLoading,
  selectedEmail,
  tabs,
  activeTabId,
  activeTab,
  accounts,
  activeAccountId,
  onSelectMainTab,
  onSelectTab,
  onCloseTab,
  onCloseMain,
  fullWidth = false,
  onOpenInTab,
  onChatAboutThread,
  className = 'flex flex-col flex-1 overflow-hidden',
}: ReadingPaneProps) {
  // A tab shows its own content; otherwise the main selection does.
  const shownThread = activeTab?.type === 'thread' ? activeTab.threadEmails : threadEmails;
  const shownIsLoading = activeTab?.type === 'thread' ? activeTab.isLoading : isLoading;

  // In full-width layout "close" always returns to the list, so it clears the
  // tab AND the selection; in split layout it closes just the active thing.
  const closeActive = () => {
    if (fullWidth) {
      if (activeTab) onCloseTab(activeTab.id);
      onCloseMain();
      return;
    }
    if (activeTab) onCloseTab(activeTab.id);
    else onCloseMain();
  };

  return (
    <div className={className}>
      {tabs.length > 0 && (
        <EmailTabBar
          mainEmail={selectedEmail}
          isMainTabActive={activeTabId === null}
          tabs={tabs}
          activeTabId={activeTabId}
          onSelectMainTab={onSelectMainTab}
          onSelectTab={onSelectTab}
          onCloseTab={onCloseTab}
        />
      )}

      {activeTab?.type === 'attachment' ? (
        <AttachmentTabView tab={activeTab} onClose={closeActive} />
      ) : activeTab?.type === 'compose' ? (
        <ComposeTabView tab={activeTab} accounts={accounts} onClose={() => onCloseTab(activeTab.id)} />
      ) : (
        <EmailView
          threadEmails={shownThread}
          isLoading={shownIsLoading}
          onClose={closeActive}
          accounts={accounts}
          activeAccountId={activeAccountId}
          fullWidth={fullWidth}
          onOpenInTab={onOpenInTab}
          onChatAboutThread={onChatAboutThread}
        />
      )}
    </div>
  );
}
