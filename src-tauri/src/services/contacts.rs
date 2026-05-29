//! Contacts service: thin business-logic layer between Tauri commands and the
//! database. The DB layer (`db/emails.rs`) owns the SQL; this module exists so
//! commands stay thin wrappers per the project's command/service/db layering
//! convention.
//!
//! Today the work here is just delegation, but keeping a service module lets us
//! grow features (caching, enrichment, account-scoped authorization) without
//! changing the command surface.

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{CompanyContactsGroup, Contact, ContactDetail, ContactsPage, ContactsQuery};

pub fn get_contacts(db: &Database, account_id: &str) -> Result<Vec<Contact>> {
    db.get_contacts(account_id)
}

pub fn list_contacts(db: &Database, account_id: &str, query: &ContactsQuery) -> Result<ContactsPage> {
    db.list_contacts(account_id, query)
}

pub fn get_contact_detail(db: &Database, account_id: &str, address: &str) -> Result<Option<ContactDetail>> {
    db.get_contact_detail(account_id, address)
}

pub fn list_contacts_by_company(db: &Database, account_id: &str) -> Result<Vec<CompanyContactsGroup>> {
    db.list_contacts_by_company(account_id)
}

/// Resolve an informal contact hint ("alice emailops", "smith@") to actual
/// contacts. Backs the chat `search_contacts` tool and any future
/// autocomplete command — keeps the SQL in `db::emails::contacts`.
pub fn search_contacts(db: &Database, account_id: &str, query: &str, limit: i32) -> Result<Vec<Contact>> {
    db.search_contacts(account_id, query, limit)
}
