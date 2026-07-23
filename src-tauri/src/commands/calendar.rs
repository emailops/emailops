use crate::models::CalendarEvent;
use crate::{AppError, AppState};
use tauri::State;

/// Events overlapping `[range_start, range_end)` for one account. The calendar
/// surface is per-account only (docs/DECISIONS.md) — there is deliberately no
/// unified variant of this command.
#[tauri::command]
pub async fn get_calendar_events(
    state: State<'_, AppState>,
    account_id: String,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<CalendarEvent>, AppError> {
    state.db.list_calendar_events(&account_id, range_start, range_end)
}

/// Create an event on the account's primary calendar (double-click in the
/// calendar view). Gmail events get a generated Google Meet link; Graph
/// events are created plain. Invitees receive invitations; `recurrence` is a
/// preset ("none" | "daily" | "weekly" | "weekdays" | "monthly" | "yearly")
/// and `time_zone` is the user's IANA zone (recurrence anchors to it).
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command boundary — mirrors the dialog's fields 1:1
pub async fn create_calendar_event(
    state: State<'_, AppState>,
    account_id: String,
    title: String,
    description: String,
    attendees: Vec<String>,
    start_time: i64,
    end_time: i64,
    recurrence: String,
    time_zone: String,
) -> Result<CalendarEvent, AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    let recurrence = crate::sync::calendar_provider::EventRecurrence::from_wire(&recurrence)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown recurrence '{recurrence}'")))?;
    let provider = crate::services::calendar::sync::build_calendar_provider(&account.id, &account.provider)?;
    let input = crate::sync::calendar_provider::NewCalendarEvent {
        title,
        description,
        attendees,
        start_time,
        end_time,
        time_zone,
        recurrence,
        request_meet_link: account.provider == "gmail",
    };
    crate::services::calendar::create::create_calendar_event(
        &state.db,
        &account.id,
        provider.as_ref(),
        input,
        chrono::Utc::now().timestamp(),
    )
    .await
}

/// Delete (cancel) an event. When `notify_attendees` is true the provider
/// sends a cancellation; `message` is included where supported (Outlook —
/// Google's API only sends its standard cancellation email).
#[tauri::command]
pub async fn delete_calendar_event(
    state: State<'_, AppState>,
    account_id: String,
    provider_event_id: String,
    notify_attendees: bool,
    // Option so a frontend `null` / omitted field can never fail arg
    // deserialization (the regression behind "invalid args `message`").
    message: Option<String>,
    // "instance" (default) | "following" | "all" — recurring-series scope.
    scope: Option<String>,
) -> Result<(), AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    let scope_wire = scope.unwrap_or_else(|| "instance".to_string());
    let scope = crate::services::calendar::delete::DeleteScope::from_wire(&scope_wire)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown delete scope '{scope_wire}'")))?;
    let provider = crate::services::calendar::sync::build_calendar_provider(&account.id, &account.provider)?;
    crate::services::calendar::delete::delete_calendar_event(
        &state.db,
        &account.id,
        provider.as_ref(),
        &provider_event_id,
        scope,
        notify_attendees,
        message.as_deref().unwrap_or_default(),
    )
    .await
}

/// Parse the calendar invite (.ics / text/calendar attachment) on an email,
/// if any. Returns `None` for emails without an invite part.
#[tauri::command]
pub async fn get_calendar_invite(
    state: State<'_, AppState>,
    email_id: String,
) -> Result<Option<crate::services::calendar::invite::CalendarInvite>, AppError> {
    crate::services::calendar::invite::get_calendar_invite(&state.db, &email_id).await
}

/// RSVP to an invitation ("accepted" | "declined" | "tentative"). The event is
/// located in the account's calendar by the invite's iCalendar UID; the
/// organizer is notified of the response.
#[tauri::command]
pub async fn rsvp_calendar_invite(
    state: State<'_, AppState>,
    account_id: String,
    ical_uid: String,
    response: String,
) -> Result<(), AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    let rsvp = crate::sync::calendar_provider::RsvpResponse::from_wire(&response)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown RSVP response '{response}'")))?;
    let provider = crate::services::calendar::sync::build_calendar_provider(&account.id, &account.provider)?;
    provider.rsvp_by_ical_uid(&ical_uid, rsvp, &account.email).await
}

/// Run one calendar sync cycle for an account right now (view open / manual
/// refresh). The background scheduler covers steady-state; this exists so the
/// user never stares at a stale week after opening the calendar.
#[tauri::command]
pub async fn sync_calendar_now(state: State<'_, AppState>, account_id: String) -> Result<u32, AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    let provider = crate::services::calendar::sync::build_calendar_provider(&account.id, &account.provider)?;
    crate::services::calendar::sync::sync_account_calendar(
        &state.db,
        &account.id,
        provider.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .await
}
