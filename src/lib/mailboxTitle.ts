/**
 * Title of the message-list pane: the mailbox the user opened, then the
 * account. `t` is the i18n function so the label follows the UI language.
 */
export function mailboxTitle(viewMode: string, accountName: string | undefined, t: (key: string) => string): string {
  let label: string;
  if (viewMode === 'sent' || viewMode === 'spam' || viewMode === 'deleted') label = t(`sidebar:${viewMode}`);
  else if (viewMode.startsWith('folder:')) label = viewMode.slice('folder:'.length);
  else label = t('sidebar:inbox');
  return accountName ? `${label} — ${accountName}` : label;
}
