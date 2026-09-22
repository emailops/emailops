---
title: 'Standard Features'
description: 'The email client itself: accounts, unified inbox, calendar, attachments, search and junk filtering.'
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
---

Everything on this page works with AI switched off. The AI layer is covered separately in
[AI features](../ai-features/).

## Accounts and sync

Connect as many mailboxes as you like — Gmail, Outlook / Microsoft 365 (Graph API), and any
IMAP/SMTP server (iCloud, Yahoo, Fastmail, ProtonMail Bridge, self-hosted). Mail is synced
into a local SQLite database, so reading and searching stay fast and work offline.

The name on each account is the sender name recipients see on the mail you send from it.
Set it in the **Sender name** field of the account's settings, or leave it empty to send with
the address only. Gmail accounts start with the name from Gmail's send-as setting, and IMAP
accounts with the display name you gave when connecting them. Mail sent through Outlook
carries the name Microsoft has for the mailbox.

## Unified inbox {#unified-inbox}

An **All accounts** view merges every enabled mailbox into one list, alongside the
per-account views. Custom IMAP folders are synced too, and you can create, rename, delete
and drag messages between folders from inside the app.

## Forwarding

**Forward** sits next to **Reply** and **Reply all** in the reading pane. The draft opens with
no recipients and carries the original message under a *Forwarded message* header with its
sender, date and recipients, together with the original attachments (up to 20 MB in total).
It goes out as a new message, so it does not join the recipient's existing conversations.

## Smart filters

Narrow the list by domain, sender, or any classification tag — useful for triaging one
client, one project or one newsletter flood at a time. With AI on, the same tags also
feed the [Tag Board](../ai-features/#tag-board), which lays them out as a grid of blocks.

## Calendar {#calendar}

Per-account month, week and day views for Google Calendar and Outlook. You get meeting
reminders ahead of each event with a one-click **Join** button for Meet, Teams, Webex and
Zoom links. Calendar sync is on by default for Gmail and Outlook accounts and can be
switched off per account, along with the notification lead time, in **Settings → Calendar**.

Every calendar on an account is synced, not just the primary one — so a calendar a
colleague shared with you shows up here the same way it does in Google or Outlook. Each
one is tinted with the colour its provider gives it, and the legend above the grid hides
or shows individual calendars; the same switches live in **Settings → Calendar**.

## Attachments view {#attachments-view}

One place for the attachments you care about — invoices, contracts, receipts — with preview and
download, instead of digging back through threads. Open it from **Attachments** in the sidebar.

The view collects attachments through **rules**, so it starts empty. Click **Manage Rules** (or
**Create a Rule** on the empty view) and fill in:

- **Rule Name** — how the rule is listed.
- **Sender Email Pattern** — comma-separated; an exact match unless it contains `*`
  (`*apple.com*` matches any sender containing "apple.com"). Leave it empty to match any sender.
- **Subject Pattern** and **Filename Pattern** — `*` is a wildcard; only matching filenames are
  collected.
- **Tags** — comma-separated; they appear as filter buttons at the top of the view.

Every pattern you fill in must match. Rules run on new mail as it syncs; tick **Apply to existing
emails after creating** to collect from the mail you already have. Select attachments to download
them together to your Downloads folder.

## Search

Full-text search over subjects, bodies, senders and attachments. With AI enabled this is
joined by semantic search, which matches on meaning rather than exact words.

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

EmailOps scores every incoming message locally for spam and unwanted bulk mail. No model
and no network call is involved, and your corrections ("junk" / "not junk") train the filter
over time. You decide what happens to flagged mail:

- **Fade it in the list** — still there, just easy for the eye to skip.
- **Keep it out of the inbox** — removed from the list, still reachable via search and your
  provider's own folders.

Neither option moves or deletes anything on the server; only an explicit **Confirm junk**
does. An optional impersonation/phishing warning is available and off by default.

## Privacy and security controls {#privacy-and-security-controls}

A main password locks the app on startup, remote images and tracking pixels are blocked
until you allow them, and credentials live in the system keyring. All of it is covered in
[Privacy & security](../privacy-security/).

## Interface {#interface}

Split or full-width inbox layout, and a UI available in English, Spanish, French and German.
The AI's output language is set separately, so you can read the interface in one language and
have replies drafted in another.
