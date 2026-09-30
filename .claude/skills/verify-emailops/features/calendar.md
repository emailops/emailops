# Calendar

Gmail and Outlook accounts show their calendars, create and delete events, and answer
invitations. An email that carries an invitation (`.ics` part) shows an invite card; the chat
can list events.

## Sub-features

- `calendar.view` month / week grid, several calendars per account.
- `calendar.invite` an email with a calendar part shows the invite card; detection is structural (the `.ics` part), not the presence of a meeting link.
- `calendar.rsvp` accept / decline / tentative notifies the organizer.
- `calendar.chat` "what do I have tomorrow?" is answered from the stored events.

## How to get to it (user POV)

- Sidebar → **Calendar** (only for Gmail / Outlook accounts).
- An invitation email → the card above the body.

## Driving it with verify.sh

Preconditions: baseline; the credential-less Gmail account `ulises.emailopslabs@gmail.com` owns the demo calendar (six events around today).

- View → `$V wd click 'button*=Calendar'` → the grid renders with the demo events.

## Gotchas

- Creating, deleting and answering events need provider credentials: not drivable on the demo instance.
- Demo events are re-anchored to "now" on every run (`ensure_demo_db.sh` → `--refresh-calendar`); assert on titles, never on dates.

| Case | Test kind |
|---|---|
| grid, recurrence, time zones, colours | unit + vitest (`services::calendar::*`, `src/lib/calendar*`, `Calendar/*`) |
| provider clients | unit (`sync::gmail_calendar`, `sync::outlook_calendar`) |
| invite parsing, stored invite read from the data dir | unit (`services::calendar::invite`) |
| invite card built from the part kept at ingest; a link-only email has none | integration (`calendar_invite_card_comes_from_the_ics_part_kept_at_ingest`) |
| calendar command arguments, `CalendarInvite` shape | contract (`src/lib/apiContract/calendario.api.test.ts`, `db::calendar*` round trips) |
| the view | e2e (`Vistas/Calendar`) |
| chat answers about events | eval (`calendar_*` in `src-tauri/evals/chat/cases/calendar_routing.yaml`) |
