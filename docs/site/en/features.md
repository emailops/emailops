---
title: 'Standard Features'
description: 'The email client itself: accounts, unified inbox, archive, snooze, scheduled send, signatures, calendar, attachments, search, junk filtering, notifications and keyboard shortcuts.'
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
  undo-send-and-scheduled-send: settings/appearance
  unsubscribe-and-block-sender: settings/junk
  keyboard-shortcuts: settings/appearance
---

<!-- claim:feat-intro-1 -->
Everything on this page works with AI switched off. The AI layer is covered separately in
[AI features](../ai-features/).

## Accounts and sync

<!-- claim:feat-accounts-sync-1 -->
Connect as many mailboxes as you like — Gmail, Outlook / Microsoft 365 (Graph API), and any
IMAP/SMTP server (iCloud, Yahoo, Fastmail, ProtonMail Bridge, self-hosted). Mail is synced
into a local SQLite database, so reading and searching stay fast and work offline.

<!-- claim:feat-accounts-sync-2 -->
The name on each account is the sender name recipients see on the mail you send from it.
Set it in the **Sender name** field of the account's settings, or leave it empty to send with
the address only. Gmail accounts start with the name from Gmail's send-as setting, and IMAP
accounts with the display name you gave when connecting them. Mail sent through Outlook
carries the name Microsoft has for the mailbox.

## Unified inbox {#unified-inbox}

<!-- claim:feat-unified-inbox-1 -->
An **All accounts** view merges every enabled mailbox into one list, alongside the
per-account views. Custom IMAP folders are synced too, and you can create, rename, delete
and drag messages between folders from inside the app.

## Forwarding

<!-- claim:reading-pane-forward -->
**Forward** sits next to **Reply** and **Reply all** in the reading pane. The draft opens with
no recipients and carries the original message under a *Forwarded message* header with its
sender, date and recipients, together with the original attachments (up to 20 MB in total).
It goes out as a new message, so it does not join the recipient's existing conversations.

## Organizing conversations {#organizing-conversations}

<!-- claim:feat-organize-1 -->
**Archive** takes a conversation out of the inbox without deleting it, and **Move to Inbox**
brings it back. Both, along with **Mark as unread** and **Star**, are in the reading pane and in
each conversation's **More actions** (⋮) menu; the star also sits on every row of the list. The
change is made at your mail provider too, so Gmail or Outlook show the same thing.

<!-- claim:feat-organize-2 -->
**Starred** in the sidebar lists your starred conversations. **Archive** lists archived mail
for Gmail and Outlook accounts and in **All accounts**; an IMAP account archives into its own
Archive folder, which appears with its other folders. Archived mail is only out of the inbox:
search, the smart filters and the AI features still reach it.

<!-- claim:feat-organize-3 -->
Tick the box at the start of a row to select it. With one or more selected, a toolbar above the
list acts on all of them at once — archive, snooze, delete, mark as read or unread, star — and
**Clear selection** ends it. Accounts with folders of their own also get **Move to folder**.

<!-- claim:feat-organize-4 -->
Archiving or deleting removes the conversations from the list at once and shows a notice with
**Undo** for 6 seconds. Your mail provider is only told when those seconds are up (or sooner, if
you archive or delete something else or open another view), so Undo simply puts them back.

<!-- claim:feat-organize-5 -->
When the conversation you are reading leaves the list — archived, deleted, snoozed, or moved to
Spam — the next one opens. **Settings → Appearance → After archiving or deleting** chooses
between the next conversation, the previous one, or going back to the list. Marking a
conversation as unread always goes back to the list.

## Snooze {#snooze}

<!-- claim:feat-snooze-1 -->
**Snooze** hides a conversation from the inbox until a time you choose: later today, tomorrow,
this weekend, next week, or a date and time you pick. It is offered in the reading pane, the
row's ⋮ menu and the selection toolbar.

<!-- claim:feat-snooze-2 -->
Snoozed conversations are listed under **Snoozed** in the sidebar, soonest first, where
**Unsnooze** brings one back early. When the time comes, the conversation returns to the top of
the inbox marked unread. A new message in a snoozed conversation brings it back straight away.

<!-- claim:feat-snooze-3 -->
Snoozing is kept on this computer only: other mail apps keep showing the conversation in the
inbox. Conversations wake while EmailOps is running; one whose time passed while the app was
closed comes back the next time you open it.

## Undo send and scheduled send {#undo-send-and-scheduled-send}

<!-- claim:feat-send-1 -->
After you press **Send**, the message waits for a few seconds with an **Undo** notice; Undo
takes it back and reopens it for editing. The wait is set in **Settings → Appearance → Undo
send**: off, 5, 10, 20 or 30 seconds, 10 by default. EmailOps has to stay open until the message
has gone.

<!-- claim:feat-send-2 -->
The arrow next to **Send** opens **Schedule send**: tomorrow morning, tomorrow afternoon, Monday
morning, or a date and time you pick.

<!-- claim:feat-send-3 -->
Messages waiting to go out are listed under **Scheduled** in the sidebar, where you can **Send
now**, **Edit** or **Delete** each one. A scheduled message only goes out while EmailOps is open;
one whose time passed while the app was closed is sent the next time you open it. A message
that could not be sent stays there marked **Not sent**, with **Retry**: EmailOps never resends
on its own.
## Signatures {#signatures}

<!-- claim:feat-signatures-1 -->
Each account has its own signature, set in **Settings → Signatures**. Two switches decide where
it goes: **Insert in new messages** and **Insert in replies and forwards**. In a new message or a
reply it sits below your text; in a forward, above the forwarded message. It is part of the
message body, so you can change or delete it in any message before sending.

<!-- claim:feat-signatures-2 -->
**Add image** puts a logo or a picture of your handwritten signature into it. PNG, JPEG, GIF and
WebP are accepted; SVG and other files are refused with the reason. A wide image is scaled down
to 600 px, each image may be 200 KB at most and the whole signature 512 KB. Gmail accounts also
offer **Import from Gmail**, which copies the signature Gmail has for that address into the
editor.

<!-- claim:feat-signatures-3 -->
In the plain-text version of a message, a signature that ends it is preceded by the standard
`-- ` line, so other mail apps can recognise it. When the account has a signature for that kind
of message, AI drafts leave your name and contact details out and let the signature sign.

## Smart filters

<!-- claim:feat-smart-filters-1 -->
Narrow the list by domain, sender, or any classification tag — useful for triaging one
client, one project or one newsletter flood at a time. With AI on, the same tags also
feed the [Tag Board](../ai-features/#tag-board), which lays them out as a grid of blocks.

## Calendar {#calendar}

<!-- claim:feat-calendar-1 -->
Per-account month, week and day views for Google Calendar and Outlook. You get meeting
reminders ahead of each event with a one-click **Join** button for Meet, Teams, Webex and
Zoom links. Calendar sync is on by default for Gmail and Outlook accounts and can be
switched off per account, along with the notification lead time, in **Settings → Calendar**.

<!-- claim:feat-calendar-2 -->
Every calendar on an account is synced, not just the primary one — so a calendar a
colleague shared with you shows up here the same way it does in Google or Outlook. Each
one is tinted with the colour its provider gives it, and the legend above the grid hides
or shows individual calendars; the same switches live in **Settings → Calendar**.

## Attachments view {#attachments-view}

<!-- claim:feat-attachments-view-1 -->
One place for the attachments you care about — invoices, contracts, receipts — with preview and
download, instead of digging back through threads. Open it from **Attachments** in the sidebar.

<!-- claim:feat-attachments-view-2 -->
The view collects attachments through **rules**, so it starts empty. Click **Manage Rules** (or
**Create a Rule** on the empty view) and fill in:

- **Rule Name** — how the rule is listed. <!-- claim:feat-attachments-view-3 -->
- **Sender Email Pattern** — comma-separated; an exact match unless it contains `*`
  (`*apple.com*` matches any sender containing "apple.com"). Leave it empty to match any sender. <!-- claim:feat-attachments-view-4 -->
- **Subject Pattern** and **Filename Pattern** — `*` is a wildcard; only matching filenames are
  collected. <!-- claim:feat-attachments-view-5 -->
- **Tags** — pick tags you already use or type to create a new one; they appear as filter buttons at the top of the view. <!-- claim:feat-attachments-view-6 -->

<!-- claim:feat-attachments-view-7 -->
Every pattern you fill in must match. Rules run on new mail as it syncs; tick **Apply to existing
emails after creating** to collect from the mail you already have. Rules reach the inbox, Sent, the
archive and your own folders, never Spam or Trash. Select attachments to download them together to your
Downloads folder.

<!-- claim:feat-attachments-view-8 -->
EmailOps also proposes rules on its own. When the same sender keeps mailing you documents
(PDFs, Office files or e-invoices) — at least two emails in two different months, in your inbox
or a folder you filed them in — a
**Suggested rules** section appears in **Manage Rules**, and a badge next to **Attachments** in
the sidebar counts them. **Review** opens the rule form already filled in (sender and filename
pattern); the rule is only created when you save it. **Dismiss** hides the suggestion for
good, even when the sender later mails from another address; **Undo**, or **Restore** under
**Dismissed suggestions**, brings it back. Mail from your own address, or
from colleagues at your own company, is never suggested.

## EO Docs {#eo-docs}

<!-- claim:feat-eo-docs-1 -->
**EO Docs** lets you write documents and spreadsheets together with other EmailOps users,
with no cloud in between: every change travels as an ordinary email between your accounts,
and each copy merges what arrives without conflicts. It is experimental and on by default;
**Settings → EO Docs** turns it off, and while it is off nothing is received or sent.

<!-- claim:feat-eo-docs-2 -->
Open **EO Docs** in the sidebar and click **New** to create a document or a sheet. Documents
have headings, bold, italic, underline, lists, links, tables and images. Sheets grow with
rows and columns, take a block pasted from Excel, and a column is resized by dragging the
edge of its header. Undo and redo only take back your own changes.

<!-- claim:feat-eo-docs-3 -->
A sheet cell that starts with `=` is a formula: `SUM`, `AVERAGE`, `MIN`, `MAX` and `COUNT`
(or `SUMA`, `PROMEDIO` and `CONTAR`) over ranges such as `=SUM(B2:B10)`. Inserting or deleting
rows keeps the ranges pointing at the same cells. The filter button on a column header hides
the rows you untick; filters only change your own view.

<!-- claim:feat-eo-docs-4 -->
**Share** asks for the email addresses, suggesting colleagues from your company first, and for
your consent: from then on EmailOps emails your changes to them on its own, about two minutes
after you stop typing, or straight away with **Send changes now**. Other EmailOps users get an
invitation to **Accept**; anyone else gets a read-only copy in the invitation. Changes arrive
with the next sync, are merged, and their emails are marked read and archived.

<!-- claim:feat-eo-docs-5 -->
A change is only applied when it comes from someone the document is shared with. On Gmail and
Outlook accounts it is also refused when the sender fails your provider's authentication check
(DMARC, or SPF without a valid DKIM signature). The emails are not end-to-end encrypted: they
are as private as the rest of your mail.

<!-- claim:feat-eo-docs-6 -->
Folders of your own (never shared) keep documents in order; drag a document onto a folder, or
use **Move to**. Search finds documents by title and content, and **History** shows earlier
versions. **Export PDF** opens the print dialog, where you save the document as a PDF.
**Delete** asks first; deleting a shared document removes only your copy, and the others keep
theirs.

<!-- claim:feat-eo-docs-7 -->
**Import** turns a Word document (`.docx`) or a spreadsheet (`.xlsx`, `.xls`, `.ods`) into EO Docs,
one sheet per tab and formulas as their values; **Open in EO Docs** does the same for an email
attachment. In the composer, **From EO Docs** attaches a document, which shares it with the
email's recipients.

## Search

<!-- claim:feat-search-1 -->
Full-text search over subjects, bodies, senders and attachments. With AI enabled this is
joined by semantic search, which matches on meaning rather than exact words.

<!-- claim:feat-search-2 -->
Searches can be narrowed with operators, on their own or next to free text:

| Operator | Matches |
|---|---|
| `from:ana` | sender address or name |
| `to:ana` | recipient |
| `subject:invoice` | subject line |
| `before:2026-09-01` / `after:2026-09-01` | received date |
| `id:<email id>` | one specific email |
| `tag:newsletter` / `tag:intent=request` | a classifier tag, optionally within one facet |

## Junk and bulk mail {#junk-and-bulk-mail}

<!-- claim:feat-junk-bulk-1 -->
EmailOps scores every incoming message locally for spam and unwanted bulk mail. No model
and no network call is involved, and your corrections ("junk" / "not junk") train the filter
over time. You decide what happens to flagged mail:

- **Fade it in the list** — still there, just easy for the eye to skip. <!-- claim:feat-junk-bulk-2 -->
- **Keep it out of the inbox** — removed from the list, still reachable via search and your
  provider's own folders. <!-- claim:feat-junk-bulk-3 -->

<!-- claim:feat-junk-bulk-4 -->
Neither option moves or deletes anything on the server; only an explicit **Confirm junk**
or **Block sender** does. An optional impersonation/phishing warning is available and off by default.

## Unsubscribe and block sender {#unsubscribe-and-block-sender}

<!-- claim:feat-unsubscribe-1 -->
A newsletter or mailing-list message that says how to leave the list shows **Unsubscribe** next
to its sender. Before anything is sent, a confirmation says exactly what will happen: a request
sent straight to the sender's server (not through your mail provider), an unsubscribe email sent
from your account, or the sender's own page opened in your browser.

<!-- claim:feat-block-sender-1 -->
**Block sender**, in a conversation's ⋮ menu, sends that sender's new mail to Spam in this
account and reports it to your mail provider as spam. **Also move their existing messages to
Spam** files what is already there, and the conversation shows that the sender is blocked, with
**Unblock** at hand.

<!-- claim:feat-block-sender-2 -->
**Settings → Junk → Blocked senders** lists everyone you blocked, with **Unblock**, which can
also bring their messages back from Spam to the inbox. The ⋮ menu's **Hide from smart filters**
is a different thing: it only removes the sender from the sidebar's smart filters.

## New-mail notifications {#new-mail-notifications}

<!-- claim:feat-notifications-1 -->
EmailOps shows a desktop notification when new mail arrives in your inbox, and when a snoozed
conversation comes back — never for an account's first sync, older mail, mail you already
read, junk or blocked senders. More than three new messages at once become a single summary.
Clicking a notification brings EmailOps to the front; it does not open the message.

<!-- claim:feat-notifications-2 -->
**Settings → Notifications** has the main switch, one switch per account, the **Notification
content** (sender and subject, or **Hide content**, which shows only the account) and **Only
when EmailOps is not focused**; all are on by default, with sender and subject shown. The message
text is never shown, and while the app is locked with the main password neither are the sender
and subject.

## Privacy and security controls {#privacy-and-security-controls}

<!-- claim:feat-privacy-security-1 -->
A main password locks the app on startup, remote images and tracking pixels are blocked
until you allow them, and credentials live in the system keyring. All of it is covered in
[Privacy & security](../privacy-security/).

## Interface {#interface}

<!-- claim:feat-interface-1 -->
Split or full-width inbox layout, and a UI available in English, Spanish, French and German.
The AI's output language is set separately, so you can read the interface in one language and
have replies drafted in another.

## Keyboard shortcuts {#keyboard-shortcuts}

<!-- claim:feat-shortcuts-1 -->
Press `?` anywhere outside a text field to see every shortcut. They follow Gmail's:

| Keys | Action |
|---|---|
| `j` / `k` | next / previous conversation |
| `Enter` or `o`, `u` | open the conversation, back to the list |
| `x` | select or deselect the conversation |
| `e`, `#`, `s`, `b` | archive, delete, star, snooze |
| `Shift+U` / `Shift+I` | mark as unread / read |
| `c`, `r`, `a`, `f` | new message, reply, reply all, forward |
| `g` then `i`, `s`, `b`, `a`, `l` | go to Inbox, Starred, Snoozed, Archive, Scheduled |
| `/` | search |

<!-- claim:feat-shortcuts-2 -->
Shortcuts pause while you type and while a dialog or menu is open. The buttons they stand for
name their key in the tooltip, as in "Archive (E)". **Settings → Appearance → Keyboard
shortcuts** turns them off, and **Show the list** opens the same overview as `?`.
