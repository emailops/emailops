---
title: 'Standard Features'
description: 'The email client itself: accounts, unified inbox, calendar, attachments, search and junk filtering.'
weight: 30
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

## Unified inbox

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

## Smart filters

<!-- claim:feat-smart-filters-1 -->
Narrow the list by domain, sender, or any classification tag — useful for triaging one
client, one project or one newsletter flood at a time. With AI on, the same tags also
feed the [Tag Board](../ai-features/#tag-board), which lays them out as a grid of blocks.

## Calendar

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

## Attachments view

<!-- claim:feat-attachments-view-1 -->
One place listing every attachment across your mail — invoices, contracts, images — with
preview and export, instead of digging back through threads.

## Search

<!-- claim:feat-search-1 -->
Full-text search over subjects, bodies, senders and attachments. With AI enabled this is
joined by semantic search, which matches on meaning rather than exact words.

## Junk and bulk mail

<!-- claim:feat-junk-bulk-1 -->
EmailOps scores every incoming message locally for spam and unwanted bulk mail. No model
and no network call is involved, and your corrections ("junk" / "not junk") train the filter
over time. You decide what happens to flagged mail:

- **Fade it in the list** — still there, just easy for the eye to skip. <!-- claim:feat-junk-bulk-2 -->
- **Keep it out of the inbox** — removed from the list, still reachable via search and your
  provider's own folders. <!-- claim:feat-junk-bulk-3 -->

<!-- claim:feat-junk-bulk-4 -->
Neither option moves or deletes anything on the server; only an explicit **Confirm junk**
does. An optional impersonation/phishing warning is available and off by default.

## Privacy and security controls

<!-- claim:feat-privacy-security-1 -->
A main password locks the app on startup, remote images and tracking pixels are blocked
until you allow them, and credentials live in the system keyring. All of it is covered in
[Privacy & security](../privacy-security/).

## Interface

<!-- claim:feat-interface-1 -->
Split or full-width inbox layout, and a UI available in English, Spanish, French and German.
The AI's output language is set separately, so you can read the interface in one language and
have replies drafted in another.
