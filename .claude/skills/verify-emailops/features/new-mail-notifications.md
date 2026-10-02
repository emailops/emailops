# New-mail notifications

A desktop notification when new mail lands in the inbox (and when a snoozed conversation comes
back), decided in the backend after junk scoring and blocked-sender filing: never for an
account's first sync, older mail, mail already read, junk or blocked senders. More than three at
once become one summary. Settings → **Notifications**: on/off, per account, content (sender and
subject, or hidden) and "only when EmailOps is not focused".

## Sub-features

- `notifications.decide` the pure planner over a synced batch (`services::mail_notifications`).
- `notifications.deliver` the notifier seam (`services::notifier`).
- `notifications.settings` the four preferences under `notifications.new_mail.*` in `user_preferences`.

## How to get to it (user POV)

- Gear → **Notifications**.

## Driving it with verify.sh

Preconditions: baseline. Confirmed live on 02/10/2026.

- Open Settings → **Notifications**; click the switch `aria/Only when EmailOps is not focused`, `aria/Notify for ulises@fastmail.com`, and the *Hide content* radio → `select key, value from user_preferences where key like 'notifications.new_mail.%'` shows the new values; close and reopen Settings: the controls keep them. Flip them back.

## Gotchas

- The demo never syncs (no credentials), so no notification is ever shown by the verification instance; delivery is proven by the integration test against the fake provider.
- macOS asks for notification permission the first time; the verifier never triggers it.

| Case | Test kind |
|---|---|
| what notifies (first sync, read, junk, blocked, summaries, content) | unit (`services::mail_notifications`, `services::notifier`) |
| settings round-trip | vitest (`NotificationsSettings`) |
| a sync notifies only genuinely new mail | integration (`sync_notifies_only_genuinely_new_mail`) |
| contract | n/a: no command of its own; the settings use the generic preference commands (`ajustes.api.test.ts`) |
| settings persist across reopen | e2e (`Notificaciones/*`) |
| eval | n/a: no model involved |
