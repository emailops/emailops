---
title: 'Standard Features'
description: 'The email client itself: accounts, unified inbox, calendar, attachments, search and junk filtering.'
weight: 30
---

Everything on this page works with AI switched off. The AI layer is covered separately in
[AI features](../ai-features/).

## Accounts and sync

Connect as many mailboxes as you like — Gmail, Outlook / Microsoft 365 (Graph API), and any
IMAP/SMTP server (iCloud, Yahoo, Fastmail, ProtonMail Bridge, self-hosted). Mail is synced
into a local SQLite database, so reading and searching stay fast and work offline.

## Unified inbox

An **All accounts** view merges every enabled mailbox into one list, alongside the
per-account views. Custom IMAP folders are synced too, and you can create, rename, delete
and drag messages between folders from inside the app.

## Smart filters

Narrow the list by domain, sender, or any classification tag — useful for triaging one
client, one project or one newsletter flood at a time.

## Calendar

Per-account month, week and day views for Google Calendar and Outlook. You get meeting
reminders ahead of each event with a one-click **Join** button for Meet, Teams, Webex and
Zoom links. Calendar sync is on by default for Gmail and Outlook accounts and can be
switched off per account, along with the notification lead time, in **Settings → Calendar**.

Every calendar on an account is synced, not just the primary one — so a calendar a
colleague shared with you shows up here the same way it does in Google or Outlook. Each
one is tinted with the colour its provider gives it, and the legend above the grid hides
or shows individual calendars; the same switches live in **Settings → Calendar**.

## Attachments view

One place listing every attachment across your mail — invoices, contracts, images — with
preview and export, instead of digging back through threads.

## Search

Full-text search over subjects, bodies, senders and attachments. With AI enabled this is
joined by semantic search, which matches on meaning rather than exact words.

## Junk and bulk mail

EmailOps scores every incoming message locally for spam and unwanted bulk mail. No model
and no network call is involved, and your corrections ("junk" / "not junk") train the filter
over time. You decide what happens to flagged mail:

- **Fade it in the list** — still there, just easy for the eye to skip.
- **Keep it out of the inbox** — removed from the list, still reachable via search and your
  provider's own folders.

Neither option moves or deletes anything on the server; only an explicit **Confirm junk**
does. An optional impersonation/phishing warning is available and off by default.

## Privacy and security controls

A main password locks the app on startup, remote images and tracking pixels are blocked
until you allow them, and credentials live in the system keyring. All of it is covered in
[Privacy & security](../privacy-security/).

## Interface

Split or full-width inbox layout, and a UI available in English, Spanish, French and German.
The AI's output language is set separately, so you can read the interface in one language and
have replies drafted in another.
