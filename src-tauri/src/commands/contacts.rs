use tauri::State;

use crate::models::error::AppError;
use crate::models::{CompanyContactsGroup, Contact, ContactDetail, ContactsPage, ContactsQuery};
use crate::services::contacts;
use crate::AppState;

/// Legacy command kept for backward compatibility. The frontend uses
/// `list_contacts` now; this still returns a flat list (no enrichment) for
/// any external caller that may still depend on it.
#[tauri::command]
pub async fn get_contacts(state: State<'_, AppState>, account_id: String) -> Result<Vec<Contact>, AppError> {
    contacts::get_contacts(&state.db, &account_id)
}

/// Paginated, sortable, filterable contacts list (Phase 1+2).
#[tauri::command]
pub async fn list_contacts(
    state: State<'_, AppState>,
    account_id: String,
    query: Option<ContactsQuery>,
) -> Result<ContactsPage, AppError> {
    contacts::list_contacts(&state.db, &account_id, &query.unwrap_or_default())
}

/// Detail payload for the contact drawer (Phase 3.1).
#[tauri::command]
pub async fn get_contact_detail(
    state: State<'_, AppState>,
    account_id: String,
    address: String,
) -> Result<Option<ContactDetail>, AppError> {
    contacts::get_contact_detail(&state.db, &account_id, &address)
}

/// Group contacts by their derived company tag (Phase 4.5).
#[tauri::command]
pub async fn list_contacts_by_company(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<CompanyContactsGroup>, AppError> {
    contacts::list_contacts_by_company(&state.db, &account_id)
}
