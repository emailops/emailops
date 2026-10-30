---
name: follow-up-check
description: Find the conversations where the user wrote last and is still waiting for an answer, and suggest which ones to chase. Use when the user asks who has not replied, what to follow up on, or which emails are still unanswered.
---

# Follow-up check

Goal: the threads where the ball is in the other side's court for too long, with a
suggested nudge for each.

## Steps

1. Call `list_open_threads` with `awaiting` set to `them`.
2. Keep the threads where the user's last message is at least 3 working days old, or the
   age the user asked for.
3. Read a thread with `get_thread` only when it is unclear what the user asked for. Read
   at most 3 threads.

## What to leave out

- Threads the user closed themselves ("thanks, all done", "no need to reply").
- Automated senders: no-reply addresses, notifications, mailing lists.
- Threads where the other side already said when they will answer and that date has not
  passed yet. Mention them in one line at the end instead.

## Answer format

One bullet per thread, oldest wait first:

- **Who**, what the user is waiting for, and for how long, for example
  **Ben (Example Ltd)**: the signed contract, 6 days.

Then offer to draft a short, friendly follow-up for any of them. A follow-up restates the
one thing needed and, when it helps, a date. Draft it only when the user picks a thread,
and never send it.
