# Calendar

Gmail and Outlook accounts show their calendars, create and delete events, and answer
invitations. IMAP accounts get a read-only calendar by subscribing to a published ICS feed
(Proton Calendar's "share via link" URL, or any `https://…/calendar.ics`). The calendar opens
by default as a day agenda in the right dock, beside the chat; the full month / week / day grid
is one click further. An email that carries an invitation (`.ics` part) shows an invite card;
the chat can list events.

## Sub-features

- `calendar.panel` right-docked day agenda for the account the user is in, drawn as the hour-by-hour `TimeGrid` of a single day (click opens the event, double-click on a free slot creates one); the collapsed dock's rail shows a calendar button directly below the chat button; "Open full calendar" switches to the week/month grid.
- `calendar.meet` meeting links are detected for Google Meet, Teams, Zoom, Webex, GoToMeeting, Jitsi and Proton Meet (`meet.proton.me`, also from Proton's `X-PM-CONFERENCE-URL` feed property): the event dialog and the reminder show a Join button.
- `calendar.view` month / week grid, several calendars per account.
- `calendar.feeds` Settings → Calendar → Calendar feeds: an IMAP account subscribes to an ICS URL (kept in the keychain, never shown back); the feed becomes a read-only calendar with its recurrences expanded locally. An IMAP account without a feed gets a setup prompt in the panel. On a Proton feed, double-clicking an empty slot opens Proton Calendar's create-event page for that day in the browser.
- `calendar.invite` an email with a calendar part shows the invite card; detection is structural (the `.ics` part), not the presence of a meeting link.
- `calendar.rsvp` accept / decline / tentative notifies the organizer.
- `calendar.chat` "what do I have tomorrow?" is answered from the stored events.

## How to get to it (user POV)

- Sidebar → **Calendar** (any account) → the agenda panel docks on the right; its header's expand button opens the full calendar view.
- Right-edge rail (dock collapsed) → calendar button, below the chat button.
- Settings → Calendar → **Calendar feeds (IMAP accounts)** to subscribe / remove a feed.
- An invitation email → the card above the body.

## Driving it with verify.sh

Preconditions: baseline; the credential-less Gmail account `ulises.emailopslabs@gmail.com` owns the demo calendar (six events around today); the two IMAP demo accounts have no feed.

- Panel → `$V wd click 'button*=Calendar'` → `[data-testid="calendar-panel"]` renders; on the IMAP demo account it shows the setup prompt (`calendar:panel.setupTitle`) with a button to Settings.
- Grid → `$V wd click 'button[aria-label="Open full calendar"]'` → the grid renders with the demo events.
- Feeds → Settings → Calendar → the `calendar-feeds-<account id>` card lists feeds and has the add form.

## Gotchas

- Creating, deleting and answering events need provider credentials: not drivable on the demo instance.
- Subscribing a feed needs network (the backend fetches the URL once): not drivable on the demo instance either — assert the setup prompt and the Settings form, and test a real feed manually.
- Demo events are re-anchored to "now" on every run (`ensure_demo_db.sh` → `--refresh-calendar`); assert on titles, never on dates.
- The all-day step writes its event with SQL as the providers store it (UTC midnights, end exclusive) and removes it: the chat evals read the demo calendar.
- The agenda and the full view are never mounted together (the dock hides its calendar while the full view is open), but switching between them runs `sync_calendar_now` twice in a row.

| Case | Test kind |
|---|---|
| grid, recurrence, time zones, colours | unit + vitest (`services::calendar::*`, `src/lib/calendar*`, `Calendar/*`) |
| provider clients | unit (`sync::gmail_calendar`, `sync::outlook_calendar`) |
| ICS feeds: parser, RRULE/EXDATE/RECURRENCE-ID expansion, read-only writes, vault-backed URLs, account-aware capability | unit (`services::calendar::ics`, `sync::ics_feed_calendar`, `services::calendar::feeds`, `services::calendar::capability`, `db::calendar_feeds`) |
| invite parsing, stored invite read from the data dir | unit (`services::calendar::invite`) |
| invite card built from the part kept at ingest; a link-only email has none | integration (`calendar_invite_card_comes_from_the_ics_part_kept_at_ingest`) |
| calendar command arguments, `Calendar` / `CalendarFeed` / `CalendarInvite` shapes | contract (`src/lib/apiContract/calendario.api.test.ts`, `db::calendar*` round trips) |
| right dock mode, rail order, agenda panel states, feeds settings form, read-only dialog, Proton create link | vitest (`src/lib/rightDockLayout`, `src/components/RightDock`, `Calendar/CalendarPanel`, `Settings/CalendarSettings.feeds`, `Calendar/EventDetailDialog.readOnly`, `src/lib/calendarFeeds`) |
| the panel and the view | e2e (`Vistas/Calendar`) |
| an all-day event sits on its own date in several zones; Escape closes its detail | vitest (`calendarGrid.allDay`, `EventDetailDialog.escape`) + e2e in the real grid (`Calendario/evento de día completo en su fecha`) |
| chat answers about events | eval (`calendar_*` in `src-tauri/evals/chat/cases/calendar_routing.yaml`) |
