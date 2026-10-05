//! Contacts service: thin business-logic layer between Tauri commands and the
//! database. The DB layer (`db/emails.rs`) owns the SQL; this module exists so
//! commands stay thin wrappers per the project's command/service/db layering
//! convention.
//!
//! Today the work here is just delegation, but keeping a service module lets us
//! grow features (caching, enrichment, account-scoped authorization) without
//! changing the command surface.

use crate::db::Database;
use crate::models::error::{AppError, Result};
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

/// Pure: the organization an address belongs to — its domain, unless that is
/// a free personal provider (gmail.com, outlook.com…), which says nothing
/// about who someone works with.
pub fn organization_domain(address: &str) -> Option<String> {
    let domain = crate::util::email_addr::extract_domain(&address.trim().to_lowercase())?;
    (!crate::util::email_addr::is_personal_email_domain(&domain)).then_some(domain)
}

/// The organization of one of the user's accounts ("Mi organización"), if any.
pub fn account_organization_domain(db: &Database, account_id: &str) -> Result<Option<String>> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))?;
    Ok(organization_domain(&account.email))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_company_address_has_an_organization_and_a_free_provider_does_not() {
        assert_eq!(organization_domain("Ana@Acme.example").as_deref(), Some("acme.example"));
        assert_eq!(organization_domain("me@gmail.com"), None);
        assert_eq!(organization_domain("me@outlook.com"), None);
        assert_eq!(organization_domain("not-an-address"), None);
    }

    #[test]
    fn the_organization_of_an_account_comes_from_its_address() {
        let db = Database::new_for_testing().unwrap();
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at) VALUES
                   ('work', 'imap', 'ana@acme.example', 'Ana', 0),
                   ('home', 'gmail', 'ana@gmail.com', 'Ana', 0)",
                [],
            )
            .unwrap();
        assert_eq!(
            account_organization_domain(&db, "work").unwrap().as_deref(),
            Some("acme.example")
        );
        assert_eq!(account_organization_domain(&db, "home").unwrap(), None);
        assert!(account_organization_domain(&db, "missing").is_err());
    }
}
