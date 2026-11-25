use crate::models::{Calendar, CalendarEvent, CalendarFeed};
use crate::services;
use crate::{AppError, AppState};
use tauri::State;

/// Every calendar the account can see (its own, shared-with-it, subscribed),
/// in the provider's list order with the primary first. The frontend uses this
/// for per-calendar colours and the show/hide filter.
#[tauri::command]
pub async fn get_calendars(state: State<'_, AppState>, account_id: String) -> Result<Vec<Calendar>, AppError> {
    state.db.list_calendars(&account_id)
}

/// Show or hide one calendar in the calendar view. Hidden calendars keep
/// syncing, so toggling one back on is instant.
#[tauri::command]
pub async fn set_calendar_visible(
    state: State<'_, AppState>,
    account_id: String,
    calendar_id: String,
    visible: bool,
) -> Result<(), AppError> {
    state.db.set_calendar_visible(&account_id, &calendar_id, visible)
}

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
    let provider = crate::services::calendar::sync::build_calendar_provider(&state.db, &account)?;
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
#[allow(clippy::too_many_arguments)] // Tauri command boundary — mirrors the dialog's fields 1:1
pub async fn delete_calendar_event(
    state: State<'_, AppState>,
    account_id: String,
    // Option so pre-multi-calendar callers keep working; the event carries its
    // own calendar id, and everything synced before V022 lived in "primary".
    calendar_id: Option<String>,
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
    let provider = crate::services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    crate::services::calendar::delete::delete_calendar_event(
        &state.db,
        &account.id,
        provider.as_ref(),
        calendar_id.as_deref().unwrap_or("primary"),
        &provider_event_id,
        scope,
        notify_attendees,
        message.as_deref().unwrap_or_default(),
    )
    .await
}

/// Change my own answer on an event in my calendar. `scope` is "instance"
/// (default) or "series".
#[tauri::command]
pub async fn respond_to_calendar_event(
    state: State<'_, AppState>,
    account_id: String,
    calendar_id: String,
    provider_event_id: String,
    response: String,
    scope: Option<String>,
) -> Result<(), AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    let response = crate::sync::calendar_provider::RsvpResponse::from_wire(&response)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown response '{response}'")))?;
    let scope_wire = scope.unwrap_or_else(|| "instance".to_string());
    let scope = crate::services::calendar::respond::ResponseScope::from_wire(&scope_wire)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown response scope '{scope_wire}'")))?;
    let provider = crate::services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    crate::services::logger::log("info", "sync", "Calendar response change started");
    let result = crate::services::calendar::respond::respond_to_calendar_event(
        &state.db,
        &account,
        provider.as_ref(),
        &calendar_id,
        &provider_event_id,
        response,
        scope,
    )
    .await;
    match &result {
        Ok(()) => crate::services::logger::log("success", "sync", "Calendar response changed"),
        Err(e) => crate::services::logger::log(
            "error",
            "sync",
            format!("Calendar response change failed: {}", e.code()),
        ),
    }
    result
}

/// Parse the calendar invite (.ics / text/calendar attachment) on an email,
/// if any. Returns `None` for emails without an invite part.
#[tauri::command]
pub async fn get_calendar_invite(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Option<crate::services::calendar::invite::CalendarInvite>, AppError> {
    services::ownership::email_in_account(&state.db, &account_id, &email_id)?;
    crate::services::calendar::invite::get_calendar_invite(&state.db, &state.app_data_dir, &email_id).await
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
    let provider = crate::services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    crate::services::calendar::sync::sync_account_calendar(
        &state.db,
        &account.id,
        provider.as_ref(),
        chrono::Utc::now().timestamp(),
    )
    .await
}

/// The account, its calendar kind and a provider, for the `.ics` import
/// commands. Only Google and Outlook calendars accept imported events.
fn ics_import_target(
    state: &AppState,
    account_id: &str,
) -> Result<(crate::models::Account, crate::sync::calendar_provider::CalendarKind), AppError> {
    use crate::services::calendar::capability::{calendar_available, calendar_capability};
    use crate::sync::calendar_provider::CalendarKind;
    let account = state
        .db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    match calendar_capability(&state.db, &account)? {
        Some(kind @ (CalendarKind::Google | CalendarKind::Outlook)) if calendar_available(&state.db, &account) => {
            Ok((account, kind))
        }
        _ => Err(AppError::InvalidInput(
            "this account has no calendar EmailOps can write to".into(),
        )),
    }
}

/// Read an `.ics` file and report what importing it would do.
#[tauri::command]
pub async fn preview_ics_import(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    time_zone: String,
) -> Result<services::calendar::import::IcsImportPreview, AppError> {
    let (account, kind) = ics_import_target(&state, &account_id)?;
    let provider = services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    services::calendar::import::preview_ics_file(provider.as_ref(), kind, std::path::Path::new(&path), &time_zone).await
}

/// Import an `.ics` file into the account's primary calendar, then mirror the
/// calendar like `sync_calendar_now`.
#[tauri::command]
pub async fn import_ics(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    time_zone: String,
) -> Result<services::calendar::import::IcsImportSummary, AppError> {
    let (account, kind) = ics_import_target(&state, &account_id)?;
    let ics = services::calendar::import::read_ics_file(std::path::Path::new(&path))?;
    let provider = services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    let summary = services::calendar::import::import_ics(provider.as_ref(), kind, &ics, &time_zone).await?;
    if summary.imported > 0 {
        if let Err(e) = services::calendar::sync::sync_account_calendar(
            &state.db,
            &account.id,
            provider.as_ref(),
            chrono::Utc::now().timestamp(),
        )
        .await
        {
            services::logger::log(
                "error",
                "sync",
                format!("Calendar sync after import failed: {}", e.code()),
            );
        }
    }
    Ok(summary)
}

/// Mirror the calendar after an invite changed it; a failed sync is logged
/// and never fails the answer.
async fn sync_after_invite(
    state: &AppState,
    account_id: &str,
    provider: &dyn crate::sync::calendar_provider::CalendarProvider,
    outcome: &services::calendar::respond::InviteResponseOutcome,
) {
    use services::calendar::respond::InviteResponseOutcome::{Imported, Removed};
    if !matches!(outcome, Imported { .. } | Removed { .. }) {
        return;
    }
    if let Err(e) =
        services::calendar::sync::sync_account_calendar(&state.db, account_id, provider, chrono::Utc::now().timestamp())
            .await
    {
        services::logger::log(
            "error",
            "sync",
            format!("Calendar sync after invite failed: {}", e.code()),
        );
    }
}

/// Answer an invitation received by email ("accepted" | "declined" |
/// "tentative"): the event is looked up in the account's calendar, imported
/// when it never arrived, and the organizer gets the iTIP reply.
#[tauri::command]
pub async fn rsvp_calendar_invite(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
    response: String,
    time_zone: String,
) -> Result<services::calendar::respond::InviteResponseOutcome, AppError> {
    let rsvp = crate::sync::calendar_provider::RsvpResponse::from_wire(&response)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown RSVP response '{response}'")))?;
    let (account, kind) = ics_import_target(&state, &account_id)?;
    let email = services::ownership::email_in_account(&state.db, &account_id, &email_id)?;
    let ics = services::calendar::invite::fetch_invite_ics(&state.db, &state.app_data_dir, &email_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("email '{email_id}' has no calendar invite")))?;
    let calendar = services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    let mail = services::emails::build_provider(&account, None).await?;
    let primary_calendar_id = services::calendar::create::primary_calendar_id(&state.db, &account.id)?;
    let request = services::calendar::respond::InviteRequest {
        account: &account,
        kind,
        ics: &ics,
        email_received_at: email.timestamp,
        response: rsvp,
        default_tz: &time_zone,
        primary_calendar_id: &primary_calendar_id,
        language: services::i18n::resolve_ui_language(&state.db)?.unwrap_or_default(),
        now: chrono::Utc::now().timestamp(),
    };
    let outcome = services::calendar::respond::respond_to_invite(calendar.as_ref(), mail.as_ref(), &request).await?;
    sync_after_invite(&state, &account.id, calendar.as_ref(), &outcome).await;
    Ok(outcome)
}

/// Add the invitation (or published event) in an email to the account's
/// primary calendar without answering anyone.
#[tauri::command]
pub async fn add_invite_to_calendar(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
    time_zone: String,
) -> Result<services::calendar::respond::InviteResponseOutcome, AppError> {
    let (account, kind) = ics_import_target(&state, &account_id)?;
    services::ownership::email_in_account(&state.db, &account_id, &email_id)?;
    let ics = services::calendar::invite::fetch_invite_ics(&state.db, &state.app_data_dir, &email_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("email '{email_id}' has no calendar invite")))?;
    let calendar = services::calendar::sync::build_calendar_provider(&state.db, &account)?;
    let outcome =
        services::calendar::respond::add_invite_to_calendar(calendar.as_ref(), kind, &ics, &time_zone).await?;
    sync_after_invite(&state, &account.id, calendar.as_ref(), &outcome).await;
    Ok(outcome)
}

/// Subscribed ICS feeds of an IMAP account (Settings → Calendar). Only the
/// label and host are returned; the URL stays in the vault.
#[tauri::command]
pub async fn list_calendar_feeds(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<CalendarFeed>, AppError> {
    services::calendar::feeds::list_feeds(&state.db, &account_id)
}

/// Subscribe an IMAP account to a read-only ICS feed (e.g. a Proton Calendar
/// "share via link" URL). Validates and fetches the URL once, stores it in the
/// secrets vault and mirrors the calendar right away. A blank `label` takes
/// the feed's own name.
#[tauri::command]
pub async fn add_calendar_feed(
    state: State<'_, AppState>,
    account_id: String,
    label: String,
    url: String,
) -> Result<CalendarFeed, AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{account_id}' not found")))?;
    services::calendar::feeds::add_feed(
        &state.db,
        &account,
        &label,
        &url,
        std::sync::Arc::new(crate::sync::ics_feed_calendar::ReqwestIcsFetcher),
        chrono::Utc::now().timestamp(),
    )
    .await
}

/// Unsubscribe from a feed: its calendar, events and stored URL go with it.
#[tauri::command]
pub async fn remove_calendar_feed(
    state: State<'_, AppState>,
    account_id: String,
    feed_id: String,
) -> Result<(), AppError> {
    services::calendar::feeds::remove_feed(&state.db, &account_id, &feed_id)
}
