---
name: inbox-triage
description: Triage what is waiting in the inbox and say what to answer first. Use when the user asks what needs attention, what is pending, what to do first, or for a morning or end-of-day inbox review.
---

# Inbox triage

Goal: a short, ranked list of what the user should act on, not a summary of every email.

## Steps

1. Call `list_open_threads` with `awaiting` set to `user` to find the threads where the
   user owes the next reply.
2. Call `search_emails` for mail from the last 2 days (or the period the user named) to
   catch new messages that are not threads yet.
3. Call `list_pending_tasks` to see what the user already planned to do.
4. Read a thread with `get_thread` only when its subject and snippet do not make the ask
   clear. Read at most 3 threads.

## How to rank

- **Act today**: someone is waiting on a decision, an approval, a payment, a document or
  a date, or the message names a deadline within 2 days.
- **This week**: a reply is expected but nothing breaks if it waits a day or two.
- **Read later**: newsletters, notifications, receipts, FYI messages with no question.

Within a group, older waits come first. Skip anything already covered by an open task,
and say so in one line at the end.

## Answer format

Three headings, **Act today**, **This week** and **Read later**, in that order. Under each,
one bullet per thread: who, what they need in a few words, and how long they have been
waiting, for example:

- **Ana (Example Corp)**: approve the Q3 quote, waiting 2 days.

Leave out an empty heading. Keep **Read later** to a count unless the user asks for the
list. End with one line offering to draft the first reply. Never send anything.
