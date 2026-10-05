# Calendar

Gmail and Outlook accounts show their calendars, create and delete events, and answer
invitations. An email that carries an invitation (`.ics` part) shows an invite card; the chat
can list events.

## Sub-features

- `calendar.view` month / week grid, several calendars per account.
- `calendar.invite` an email with a calendar part shows the invite card; detection is structural (the `.ics` part), not the presence of a meeting link.
- `calendar.rsvp` accept / decline / tentative notifies the organizer.
- `calendar.chat` "what do I have tomorrow?" is answered from the stored events.
- `calendar.create` New event (button or double-click a slot) creates an event, with optional invitees, on the provider.
- `calendar.delete` an event's Delete removes it on the provider: this occurrence, following, or all, optionally notifying attendees.
- `calendar.enable` calendar features appear only for Gmail/Outlook accounts whose calendar integration is on; the view offers to turn it on for the current account.

## How to get to it (user POV)

Three entry points, the columns of `## Parity`:

- **Calendar view** — Sidebar → **Calendar** (only for Gmail / Outlook accounts with calendar integration on): month / week / day grid, event detail, New event.
- **Invite card** — an invitation email shows the card above the body, with Yes / No / Maybe.
- **Chat** — the `list_calendar_events` tool answers from stored events.

## Parity

| Capability | Calendar view | Invite card (open email) | Chat |
|---|---|---|---|
| calendar.view | e2e:Vistas/Calendar | n/a: the card shows one invitation, not a calendar | n/a: the chat answers in text; that is calendar.chat |
| calendar.invite | n/a: the view shows synced provider events; invite detection is about an email's .ics part | gap: untested — CalendarInviteCard has no component test and the demo DB has no .ics email; the integration test proves the backend only | gap: missing — no tool reads an email's invite; list_calendar_events reads stored events with no email link |
| calendar.rsvp | gap: missing — the event detail lists attendees read-only and offers only Delete | gap: untested — Yes/No/Maybe has no component test; the provider side is only unit-tested | gap: missing — no RSVP chat tool |
| calendar.chat | n/a: answering questions is a chat capability | n/a: the card answers one invitation, it is not asked questions | rust:src-tauri/src/services/chat/tools/list_calendar_events.rs::lists_upcoming_events_with_organizer_and_platform |
| calendar.create | gap: untested — NewEventDialog only has a dropdown-styling test; creation is unit-tested in the service | n/a: the card answers an existing invitation | gap: missing — no create-event chat tool |
| calendar.delete | gap: untested — delete with scope and notify has no component test; the backend is unit-tested | n/a: declining an invitation is calendar.rsvp | gap: missing — no delete-event chat tool |
| calendar.enable | vitest:src/components/Calendar/CalendarView.enableBanner.test.tsx::clicking Enable turns the integration on and selects the account | gap: untested — the card only probes for an invite when the account's calendar is on; no test | rust:src-tauri/src/services/chat/tools/list_calendar_events.rs::availability_requires_a_calendar_enabled_account |

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
