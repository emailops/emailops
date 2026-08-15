# Google OAuth verification — demo video script (take 2)

Shooting script for the demo video required by Google's restricted-scope OAuth
verification. Target length **7–8 minutes**, English narration, unlisted
YouTube.



**Intro**

> This is EmailOps, a desktop email client for macOS, Windows and Linux. It is not a web service — there is no EmailOps server. Every message, contact and calendar event is stored in a local database on the user's own machine, and the AI features run locally on the same machine by default. In this video I'll connect a Google account and show every requested scope in use, including each change appearing in the Google account itself.



**Adding an account**

To add an account you click on the plus sign near accounts, this will open a selector, I choose Gmail, 

> Here is the consent screen, with every permission expanded. EmailOps requests six scopes: read and manage my Gmail messages and drafts; send mail on my behalf; view and edit calendar events; view the list of calendars in my account; and my email address and basic profile. I'll show each one in use. I approve.



**Show added account**

> The two profile scopes are used once, at sign-in. EmailOps reads the account's email address and display name so it can label the account in the sidebar,



**(Put gmail side by side with EmailOps)**  
Now we'll put gmail page side by side with EmailOps to check how inbox is synchronized  


**Reading an email** 

This is the list of emails, last one is unread, if we check in gmail it's unread also. Now we'll read it from EmailOps and check that the read status is updated in Gmail.

If we delete an email in EmailOps that's deleted on Gmail too



**Creating a draft**

If we click compose and start creating an email, then we close. It appears as draft on gmail

Then we edit the draft and save, and when we check in Gmail this draft was updated



**Sending an email**

Now we finally send the email, we go to gmail and we can see it in Sent



**Calendar**

Here we can see the different calendars, by clicking on one we show/hide it

We see the events, we can delete this one. And check in gmail how it's gone

We create an event



**Data Handling policies**

Finally to check data handling we can go to Settings and we have the priacy policy link

## Why take 1 was rejected (2026-08-15)

The reviewer asked for a new video and named three scopes —
`gmail.modify`, `gmail.send`, `calendar.events` — with four criteria. Take 1
missed three of them:


| Criterion                                                                | Take 1                   | Fix in take 2                                                 |
| ------------------------------------------------------------------------ | ------------------------ | ------------------------------------------------------------- |
| **In-app functionality** — the *maximum extent* of every requested scope | Read + compose/send only | Every write each scope grants is exercised on camera          |
| **Source account impact** — writes visible in the user's Google account  | Never left the app       | Every write cuts to `mail.google.com` / `calendar.google.com` |
| **Consent screen fully expanded**                                        | Scopes likely collapsed  | Click **Show all services**, hold, scroll the full list       |
| **Scope matching** — app manifest = Cloud Console                        | Believed OK              | Re-verify on the recording day (checklist below)              |


The reviewer's sentence to answer is *"why narrower permissions cannot be
used"*. For `gmail.modify` that could not be answered in take 1: the app only
read mail and wrote drafts, which `gmail.readonly` + `gmail.compose` would have
covered. So the app changed rather than the video — **read state and delete now
write back to Gmail** (`messages.modify` / `messages.trash`; see
`docs/DECISIONS.md`, 2026-08-15). Scene 5 exists to show that, and it is the
single most important scene in this take.

**The scope set did not change**, so the consent screen, the privacy policy and
existing users' grants all stay as they are.

Scopes under review (`GMAIL_SCOPES`, `src-tauri/src/sync/oauth.rs:24`):


| Scope                            | Class      | Demonstrated in                                       |
| -------------------------------- | ---------- | ----------------------------------------------------- |
| `gmail.modify`                   | restricted | Scenes 4 (read) + 5 (read state, delete) + 6 (drafts) |
| `gmail.send`                     | restricted | Scene 7                                               |
| `calendar.events`                | sensitive  | Scene 8                                               |
| `calendar.calendarlist.readonly` | sensitive  | Scene 9                                               |
| `userinfo.email`                 | sensitive  | Scene 3                                               |
| `userinfo.profile`               | sensitive  | Scene 3                                               |


---

## 0. Before you press record

**Blocking — the video is worthless without these:**

- [ ] **Build from a version that contains the mailbox write-back.** A release
  ```
  build predating the 2026-08-15 change deletes and marks read *locally
  only*, so Scene 5 would show nothing happening in Gmail — the exact
  failure that caused this rejection. Verify before filming: mark a message
  read in the app, refresh Gmail, confirm it flipped there.
  ```
- [ ] **Cloud Console scope list must match `GMAIL_SCOPES` exactly** — all six,
  ```
  no extras. A mismatch is a named rejection criterion. Confirm the calendar
  list row still reads **"See the list of Google calendars you're subscribed
  to"** (the `.readonly` variant); a read-write wording there contradicts the
  Scene 9 narration.
  ```
- [ ] **Publishing status is "In Production."** Google's mail says to keep it
  ```
  there; do not flip back to Testing to record.
  ```
- [ ] **Revoke EmailOps' grant** at `myaccount.google.com/permissions` for the
  ```
  filming account. The auth URL sets no `prompt` param, so an
  already-granted account skips the consent screen entirely and you lose
  Scene 2.
  ```
- [ ] Privacy policy live at `https://getemailops.com/en/privacy/`, support
  ```
  email `hello@getemailops.com`, both on the consent screen.
  ```

**Recording hygiene:**

- [ ] Film with a **dedicated test Google account**, not the personal mailbox.
  ```
  The address, every subject line and every contact name ends up on YouTube
  permanently. Seed it with synthetic mail and calendar events.
  ```
- [ ] Seed enough to work with: ~10 messages of which **several unread**, one
  ```
  with an attachment, one carrying a calendar invite, and 2–3 calendar
  events across two calendars.
  ```
- [ ] Keep a browser window open on Gmail and Google Calendar, already logged in
  ```
  as the same account, so cutting to them is one click. Both should be on
  screen next to the app if the resolution allows — a side-by-side shot of
  the app and Gmail is the single most convincing frame in the video.
  ```
- [ ] App UI language **English** (Settings → language). Narration in English.
- [ ] Use a **signed release build**, not `make dev`.
- [ ] macOS: Do Not Disturb on, other apps quit, menu bar and Dock clean.
- [ ] Record at **1920×1080**, cursor visible, no background music.
- [ ] The unverified-app interstitial will still appear. Leave it in.

---

## Scene 1 — Identify the app (0:00 – 0:25)

**On screen:** EmailOps on the Inbox, then Settings → About with name and
version.

**Narration:**

> This is EmailOps, a desktop email client for macOS, Windows and Linux. It is
> not a web service — there is no EmailOps server. Every message, contact and
> calendar event is stored in a local database on the user's own machine, and
> the AI features run locally on the same machine by default. In this video I'll
> connect a Google account and show every requested scope in use, including each
> change appearing in the Google account itself.

---

## Scene 2 — The OAuth flow (0:25 – 1:50)

The scene the reviewer scrubs to first. Move slowly; do not cut inside it.

**Actions:**

1. Sidebar → **Add account** → **Gmail** ("Sign in with Google OAuth").
2. The system browser opens. **Hold 3–4 seconds** on the `accounts.google.com`
  URL, zoomed enough that `client_id=` is readable. Mandatory.
3. Account chooser → the test account, address visible.
4. Unverified-app interstitial → **Advanced** → **Go to EmailOps (unsafe)**.
5. The consent screen. **Click "Show all services" so nothing is collapsed.**
  Hold 5 seconds, then scroll slowly top to bottom so all six scopes are
   legible in one continuous shot. Read them aloud as they pass.
6. **Continue** / **Allow** → the success page → back to EmailOps, account added
  and syncing.

**Narration:**

> Adding an account starts here: Add account, then Gmail. EmailOps opens the
> system browser — it never renders Google's sign-in inside its own window. The
> address bar shows accounts.google.com and the client ID of the OAuth client
> under review.
>
> I sign in with a Google account. Because this app is not verified yet, Google
> shows its unverified-app warning — that is what this request is for.
>
> Here is the consent screen, with every permission expanded. EmailOps requests
> six scopes: read and manage my Gmail messages and drafts; send mail on my
> behalf; view and edit calendar events; view the list of calendars in my
> account; and my email address and basic profile. I'll show each one in use. I
> approve, and control returns to the app.

---

## Scene 3 — `userinfo.email` + `userinfo.profile` (1:50 – 2:10)

**On screen:** The new account in the sidebar with address and display name;
then Settings → Accounts showing the same.

**Narration:**

> The two profile scopes are used once, at sign-in. EmailOps reads the account's
> email address and display name so it can label the account in the sidebar,
> address outgoing mail from the right identity, and tell this account apart
> from other connected accounts. Both are stored locally, next to the token in
> the operating system's keychain. There is no server to send them to.

---

## Scene 4 — `gmail.modify`, reading (2:10 – 2:50)

**On screen:** Sync completes, the inbox fills. Open a thread, scroll the body,
open an attachment. Show the search box returning results.

**Narration:**

> With the account connected, EmailOps downloads mail through the Gmail API and
> stores it locally, so the inbox works offline and searches instantly. Opening
> a thread shows the full message — headers, body, inline images and attachments
> — read through the Gmail API.
>
> Reading is only half of what this scope is for. The rest of it is changing the
> state of the mailbox, and that is next.

---

## Scene 5 — `gmail.modify`, changing the mailbox (2:50 – 4:20)

**The scene take 1 did not have.** Both halves must show Gmail's own UI
changing. Do not narrate over a stale Gmail tab — refresh it on camera.

**Actions:**

1. Put EmailOps and Gmail web side by side, both on the inbox. Point out the
  **same message showing as unread in both**.
2. In EmailOps, open that message. It becomes read.
3. Refresh Gmail on camera → the message is **no longer bold**. Hold on it.
4. Back in EmailOps, delete a different message (trash icon on the row or in the
  thread view).
5. Refresh Gmail → the message is **gone from the inbox**. Open Gmail's **Trash**
  → it is there. Hold on it.

**Narration:**

> This is why EmailOps needs gmail.modify rather than a read-only scope. The
> mailbox state the user changes here is changed in their Google account.
>
> This message is unread — in EmailOps, and in Gmail on the right. I open it in
> EmailOps. I refresh Gmail: it is read there too. EmailOps removed the UNREAD
> label through the Gmail API, so the message does not come back unread on the
> user's phone or on gmail.com.
>
> The same for deleting. I delete this message in EmailOps. Refreshing Gmail, it
> has left the inbox — and here it is in the user's Trash. EmailOps uses Gmail's
> trash operation deliberately, not permanent deletion, so the user can still
> recover the message from Gmail for thirty days. EmailOps never permanently
> deletes a message.

---

## Scene 6 — `gmail.modify`, drafts (4:20 – 5:05)

**Actions:**

1. **Compose** → fill To / Subject / a short body → close and save (don't send).
2. Gmail → **Drafts** → refresh → the draft is there. Point at it.
3. Edit the draft body in EmailOps, save. Refresh Gmail → the change is there.
4. Delete the draft in EmailOps. Refresh Gmail → it is gone.

**Narration:**

> The same scope covers drafts. When the user saves a draft here, it is created
> in their own Gmail account — the same draft is waiting on gmail.com and on
> their phone. Here it is in Gmail's Drafts folder. Editing it in EmailOps
> updates it in Gmail; deleting it here removes it there.
>
> That is the full extent of what EmailOps does with gmail.modify: read
> messages, set read state, trash messages, and create, update and delete
> drafts. A read-only scope covers none of the writes, and the app requests no
> additional read scope because gmail.modify already includes read access.

---

## Scene 7 — `gmail.send` (5:05 – 5:45)

**Actions:**

1. **Compose** a message to a second address you control → **Send**.
2. Show the success state in EmailOps.
3. Gmail → **Sent** → refresh → the message is there. Open it.
4. Optionally show it arriving in the recipient's inbox.
5. Open a received thread in EmailOps, **Reply**, send — show that in Sent too.

**Narration:**

> Sending is the other half of an email client. I compose a message and send it.
> EmailOps hands it to the Gmail API using the gmail.send scope, so it goes out
> from the user's own address — and here it is, filed in their Sent folder in
> Gmail. Replying inside a thread uses the same path, and lands in the same
> place.

---

## Scene 8 — `calendar.events` (5:45 – 6:50)

**Actions:**

1. Sidebar → **Calendar**. Show the week with the seeded events.
2. Click an event → detail dialog with time, location, attendees.
3. Create a new event (title, time) → save.
4. Cut to `calendar.google.com` → refresh → **the new event is there**. Open it.
5. Back in EmailOps, delete that event → refresh Google Calendar → **it is
  gone**.
6. Open an email carrying an invite and **RSVP** from the invite card → show the
  response reflected in Google Calendar.

**Narration:**

> EmailOps shows the account's calendar next to their mail, because most
> meetings are arranged over email. The calendar view reads events through the
> Google Calendar API.
>
> Creating an event here writes it to the user's Google Calendar — here it is on
> calendar.google.com, with the same title and time. Deleting it in EmailOps
> cancels it there too; refreshing Google Calendar, it is gone.
>
> The same scope handles invitations that arrive by email: EmailOps shows the
> invite inline and lets the user accept or decline without leaving their inbox,
> which sends the RSVP through the Calendar API — and the response shows on the
> event in Google Calendar. Reading, creating, deleting and responding is
> everything EmailOps does with calendar.events.

---

## Scene 9 — `calendar.calendarlist.readonly` (6:50 – 7:15)

**Actions:** In the Calendar view, open the calendar list showing the account's
calendars with their colours. Toggle one off — its events disappear from the
grid. Toggle it back on.

**Narration:**

> Most people have more than one calendar — a personal one, a shared team
> calendar, a subscribed one. To show them and let the user turn each on and
> off, EmailOps needs to know which calendars the account has. That is the only
> thing this scope does: the list of calendars, their names and their colours.
> It does not read what is inside them — the events come from calendar.events.
> EmailOps deliberately does not request calendar.readonly, which would also
> grant reading every calendar's contents.

---

## Scene 10 — Data handling and close (7:15 – 7:40)

**On screen:** Settings → Privacy, then `https://getemailops.com/en/privacy/`
open in the browser.

**Narration:**

> To close, on how the data is handled. Everything EmailOps receives from Google
> APIs stays in a local database on the user's device. There is no EmailOps
> server, no telemetry and no cloud sync. AI features run locally by default;
> using a remote AI provider is opt-in, off by default, and clearly labelled.
> Google user data is never used to train any model and is never sold or
> transferred to third parties.
>
> EmailOps' use and transfer of information received from Google APIs to any
> other app will adhere to the Google API Services User Data Policy, including
> the Limited Use requirements.

---

## Post-production

- No music. No cut inside Scene 2, and no cut between an action in EmailOps and
the Gmail/Calendar refresh that proves it — the reviewer must see cause and
effect in one continuous shot.
- Burn in English subtitles or upload an English caption track.
- Upload as **Unlisted** (not Private — reviewers must open it without a grant).
- Title: `EmailOps — Google OAuth scope demonstration`
- Description: the block below.
- Reply **directly to the reviewer's email** with the new URL; do not open a
fresh submission.

**Description block to paste:**

```
EmailOps (desktop email client) — OAuth scope demonstration for Google verification.
Privacy policy: https://getemailops.com/en/privacy/

0:25  OAuth consent flow, all scopes expanded (client ID visible in the address bar)
1:50  userinfo.email + userinfo.profile — labelling the connected account
2:10  gmail.modify — reading messages and attachments
2:50  gmail.modify — read state and delete, shown landing in the Gmail account
4:20  gmail.modify — creating, updating and deleting Gmail drafts
5:05  gmail.send — sending and replying, shown in the Gmail Sent folder
5:45  calendar.events — creating and deleting events, and RSVPs, shown in Google Calendar
6:50  calendar.calendarlist.readonly — listing the account's calendars
7:15  Local-only storage and Limited Use statement
```

---

## Reply to the reviewer

Keep it short and point at timestamps. Suggested body:

> Thank you for the review. A new demonstration video is at .
>
> It shows each requested scope exercised to its full extent, with every write
> reflected in the Google account on camera: read state and message deletion
> appearing in Gmail (2:50), draft create/update/delete in Gmail's Drafts
> folder (4:20), sent mail in the Sent folder (5:05), and event creation,
> deletion and RSVP in Google Calendar (5:45). The consent screen is shown fully
> expanded via "Show all services" at 0:25.
>
> EmailOps is a desktop application; the scopes requested in the app match the
> six configured in the Cloud Console exactly. On gmail.modify specifically:
> the app sets read state via users.messages.modify and deletes via
> users.messages.trash, in addition to reading messages and managing drafts, so
> gmail.readonly does not cover its use.

---

## Justification crib sheet (submission form)

Keep consistent with the privacy policy — reviewers diff them.


| Scope                            | Justification                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gmail.modify`                   | Reads full message content into the local mailbox for offline reading and search; sets read/unread state (`users.messages.modify`) and deletes messages to Trash (`users.messages.trash`) so the user's mailbox state matches everywhere; creates, updates and deletes drafts. `gmail.readonly` covers none of these writes; `gmail.readonly` is not requested in addition because `gmail.modify` already includes read access. (2:10, 2:50, 4:20) |
| `gmail.send`                     | Sends messages and replies composed in EmailOps from the user's own address, filed in their Sent folder. (5:05)                                                                                                                                                                                                                                                                                                                                    |
| `calendar.events`                | Displays the user's events next to their mail, creates and deletes events, and sends RSVPs for invitations that arrive by email. (5:45)                                                                                                                                                                                                                                                                                                            |
| `calendar.calendarlist.readonly` | Enumerates the account's calendars, with names and colours, so the user can show or hide each one. Contents are not read through this scope; `calendar.readonly` is deliberately not requested. (6:50)                                                                                                                                                                                                                                             |
| `userinfo.email`                 | Labels the connected account, addresses outgoing mail from the correct identity, distinguishes between multiple connected accounts. (1:50)                                                                                                                                                                                                                                                                                                         |
| `userinfo.profile`               | Display name for the account in the UI and in outgoing mail. (1:50)                                                                                                                                                                                                                                                                                                                                                                                |


**If the reviewer still pushes on `gmail.modify`:** the fallback is
`gmail.readonly` + `gmail.compose`, which would cover reading, drafts and
sending but *not* read state or delete — the app would lose those features. It
requires a code, consent-screen and privacy-policy change in lockstep plus
re-consent from every existing user, so it is a last resort, not an offer to
make unprompted.

**Not to claim on camera:** archiving and moving messages between folders are
still local-only for Gmail accounts (`move_message` is unimplemented for the
Gmail provider), and IMAP/Outlook accounts do not push read state or deletes at
all (`provider_supports_mailbox_writes` is Gmail-only). Reviewers test claims —
demonstrate read state, trash, drafts and send, and nothing else.