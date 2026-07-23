use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Folder, FolderRole};
use rusqlite::params;

/// One folder as discovered on the server, ready to persist. The sync layer
/// builds these from the IMAP `LIST` plan; the db layer stays independent of
/// sync types.
#[derive(Debug, Clone)]
pub struct FolderUpsert {
    pub server_path: String,
    pub display_name: String,
    pub role: FolderRole,
    pub delimiter: Option<String>,
}

impl Database {
    /// Replace the folder set for an account with the given snapshot in one
    /// transaction: present folders are inserted or updated in place (keeping
    /// `created_at`), and rows for folders no longer reported by the server
    /// are deleted. Emails referencing a pruned folder are intentionally kept.
    ///
    /// A server-side folder rename therefore looks like delete + create and
    /// re-syncs under the new path — accepted limitation.
    pub fn upsert_folders_for_account(&self, account_id: &str, folders: &[FolderUpsert]) -> Result<()> {
        let mut conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO folders (
                     id, account_id, server_path, display_name, role, delimiter,
                     last_seen_at, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (account_id, server_path) DO UPDATE SET
                     display_name = excluded.display_name,
                     role = excluded.role,
                     delimiter = excluded.delimiter,
                     last_seen_at = excluded.last_seen_at",
            )?;
            for folder in folders {
                stmt.execute(params![
                    format!("{}:{}", account_id, folder.server_path),
                    account_id,
                    folder.server_path,
                    folder.display_name,
                    folder.role.as_str(),
                    folder.delimiter,
                    now,
                    now,
                ])?;
            }
            // Prune folders the server no longer reports. Explicit NOT IN
            // (not a last_seen_at comparison — second-granularity timestamps
            // collide when two syncs run within the same second). Folder count
            // is capped upstream (~50), far below SQLite's parameter limit.
            let placeholders: Vec<String> = (2..folders.len() + 2).map(|i| format!("?{i}")).collect();
            let delete_sql = if folders.is_empty() {
                "DELETE FROM folders WHERE account_id = ?1".to_string()
            } else {
                format!(
                    "DELETE FROM folders WHERE account_id = ?1 AND server_path NOT IN ({})",
                    placeholders.join(", ")
                )
            };
            let mut delete_params: Vec<&dyn rusqlite::ToSql> = vec![&account_id];
            for folder in folders {
                delete_params.push(&folder.server_path);
            }
            tx.execute(&delete_sql, delete_params.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert or update a single folder row without touching the rest of the
    /// account's snapshot (unlike [`Self::upsert_folders_for_account`], which
    /// prunes). Used when a folder is created in-app, ahead of the next LIST.
    pub fn upsert_folder(&self, account_id: &str, folder: &FolderUpsert) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO folders (
                 id, account_id, server_path, display_name, role, delimiter,
                 last_seen_at, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (account_id, server_path) DO UPDATE SET
                 display_name = excluded.display_name,
                 role = excluded.role,
                 delimiter = excluded.delimiter,
                 last_seen_at = excluded.last_seen_at",
            params![
                format!("{}:{}", account_id, folder.server_path),
                account_id,
                folder.server_path,
                folder.display_name,
                folder.role.as_str(),
                folder.delimiter,
                now,
                now,
            ],
        )?;
        Ok(())
    }

    /// Fetch one folder row by its id (`'{account_id}:{server_path}'`),
    /// scoped to the account.
    pub fn get_folder(&self, account_id: &str, folder_id: &str) -> Result<Option<Folder>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, server_path, display_name, role, delimiter
             FROM folders WHERE account_id = ?1 AND id = ?2",
        )?;
        let mut rows = stmt.query_map(params![account_id, folder_id], |row| {
            Ok(Folder {
                id: row.get(0)?,
                account_id: row.get(1)?,
                server_path: row.get(2)?,
                display_name: row.get(3)?,
                role: row.get(4)?,
                delimiter: row.get(5)?,
            })
        })?;
        rows.next().transpose().map_err(Into::into)
    }

    /// Re-key a folder row after an in-app rename (id embeds the server path,
    /// so the primary key changes with it). Keeps `created_at`. Returns the
    /// new folder id.
    pub fn rename_folder_row(
        &self,
        account_id: &str,
        folder_id: &str,
        new_server_path: &str,
        new_display_name: &str,
    ) -> Result<String> {
        let new_id = format!("{account_id}:{new_server_path}");
        let updated = self.connection().execute(
            "UPDATE folders SET id = ?1, server_path = ?2, display_name = ?3
             WHERE account_id = ?4 AND id = ?5",
            params![new_id, new_server_path, new_display_name, account_id, folder_id],
        )?;
        if updated == 0 {
            return Err(crate::models::error::AppError::NotFound(format!(
                "Folder not found: {folder_id}"
            )));
        }
        Ok(new_id)
    }

    /// Delete a folder row by id, scoped to the account. Idempotent.
    pub fn delete_folder_row(&self, account_id: &str, folder_id: &str) -> Result<()> {
        self.connection().execute(
            "DELETE FROM folders WHERE account_id = ?1 AND id = ?2",
            params![account_id, folder_id],
        )?;
        Ok(())
    }

    /// List an account's folders, optionally restricted to one role, ordered
    /// by display name for stable sidebar rendering.
    pub fn list_folders(&self, account_id: &str, role: Option<FolderRole>) -> Result<Vec<Folder>> {
        let conn = self.reader();
        let mut sql = String::from(
            "SELECT id, account_id, server_path, display_name, role, delimiter
             FROM folders WHERE account_id = ?1",
        );
        if role.is_some() {
            sql.push_str(" AND role = ?2");
        }
        sql.push_str(" ORDER BY display_name COLLATE NOCASE ASC");
        let mut stmt = conn.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Folder> {
            Ok(Folder {
                id: row.get(0)?,
                account_id: row.get(1)?,
                server_path: row.get(2)?,
                display_name: row.get(3)?,
                role: row.get(4)?,
                delimiter: row.get(5)?,
            })
        };
        let rows = match role {
            Some(r) => stmt.query_map(params![account_id, r.as_str()], map_row)?,
            None => stmt.query_map(params![account_id], map_row)?,
        };
        let mut folders = Vec::new();
        for row in rows {
            folders.push(row?);
        }
        Ok(folders)
    }
}

#[cfg(test)]
mod tests {
    use super::FolderUpsert;
    use crate::db::Database;
    use crate::models::FolderRole;

    fn folder(path: &str, display: &str, role: FolderRole) -> FolderUpsert {
        FolderUpsert {
            server_path: path.to_string(),
            display_name: display.to_string(),
            role,
            delimiter: Some(".".to_string()),
        }
    }

    #[test]
    fn upsert_then_list_roundtrips_fields() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");

        db.upsert_folders_for_account(
            "acc-1",
            &[folder("INBOX.Projekte", "INBOX.Projekte", FolderRole::Custom)],
        )
        .unwrap();

        let all = db.list_folders("acc-1", None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "acc-1:INBOX.Projekte");
        assert_eq!(all[0].account_id, "acc-1");
        assert_eq!(all[0].server_path, "INBOX.Projekte");
        assert_eq!(all[0].display_name, "INBOX.Projekte");
        assert_eq!(all[0].role, "custom");
        assert_eq!(all[0].delimiter.as_deref(), Some("."));
    }

    #[test]
    fn upsert_replaces_in_place_and_prunes_missing() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");

        db.upsert_folders_for_account(
            "acc-1",
            &[
                folder("Alt", "Alt", FolderRole::Custom),
                folder("Kunden", "Kunden", FolderRole::Custom),
            ],
        )
        .unwrap();
        // Second LIST snapshot: "Alt" disappeared, "Kunden" got re-detected as
        // trash (role change updates in place), "Neu" appeared.
        db.upsert_folders_for_account(
            "acc-1",
            &[
                folder("Kunden", "Kunden (umbenannt)", FolderRole::Trash),
                folder("Neu", "Neu", FolderRole::Custom),
            ],
        )
        .unwrap();

        let all = db.list_folders("acc-1", None).unwrap();
        let paths: Vec<&str> = all.iter().map(|f| f.server_path.as_str()).collect();
        assert_eq!(paths, vec!["Kunden", "Neu"], "Alt must be pruned");
        let kunden = all.iter().find(|f| f.server_path == "Kunden").unwrap();
        assert_eq!(kunden.display_name, "Kunden (umbenannt)");
        assert_eq!(kunden.role, "trash");
    }

    #[test]
    fn list_is_scoped_per_account_and_role_sorted_by_display_name() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-2");

        db.upsert_folders_for_account(
            "acc-1",
            &[
                folder("Gesendete Objekte", "Gesendete Objekte", FolderRole::Sent),
                folder("Zulieferer", "Zulieferer", FolderRole::Custom),
                folder("Patienten", "Patienten", FolderRole::Custom),
            ],
        )
        .unwrap();
        db.upsert_folders_for_account("acc-2", &[folder("Other", "Other", FolderRole::Custom)])
            .unwrap();

        let custom = db.list_folders("acc-1", Some(FolderRole::Custom)).unwrap();
        let names: Vec<&str> = custom.iter().map(|f| f.display_name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Patienten", "Zulieferer"],
            "custom-only, sorted, no cross-account rows"
        );
    }

    #[test]
    fn single_upsert_and_get_and_delete_roundtrip() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.upsert_folders_for_account("acc-1", &[folder("Alt", "Alt", FolderRole::Custom)])
            .unwrap();

        // Single upsert must not prune the existing snapshot.
        db.upsert_folder("acc-1", &folder("Neu", "Neu", FolderRole::Custom))
            .unwrap();
        assert_eq!(db.list_folders("acc-1", None).unwrap().len(), 2);

        let fetched = db.get_folder("acc-1", "acc-1:Neu").unwrap().expect("row present");
        assert_eq!(fetched.server_path, "Neu");
        assert!(db.get_folder("acc-2", "acc-1:Neu").unwrap().is_none(), "account-scoped");

        db.delete_folder_row("acc-1", "acc-1:Neu").unwrap();
        assert!(db.get_folder("acc-1", "acc-1:Neu").unwrap().is_none());
        // Idempotent second delete.
        db.delete_folder_row("acc-1", "acc-1:Neu").unwrap();
    }

    #[test]
    fn rename_folder_row_rekeys_id_and_paths() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.upsert_folder("acc-1", &folder("Kunden", "Kunden", FolderRole::Custom))
            .unwrap();

        let new_id = db
            .rename_folder_row("acc-1", "acc-1:Kunden", "Klienten", "Klienten")
            .unwrap();

        assert_eq!(new_id, "acc-1:Klienten");
        assert!(db.get_folder("acc-1", "acc-1:Kunden").unwrap().is_none());
        let renamed = db.get_folder("acc-1", "acc-1:Klienten").unwrap().expect("renamed row");
        assert_eq!(renamed.server_path, "Klienten");
        assert_eq!(renamed.display_name, "Klienten");
        assert_eq!(renamed.role, "custom");

        assert!(db.rename_folder_row("acc-1", "acc-1:Nope", "X", "X").is_err());
    }

    #[test]
    fn deleting_an_account_cascades_folder_rows() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.upsert_folders_for_account("acc-1", &[folder("Kunden", "Kunden", FolderRole::Custom)])
            .unwrap();

        db.delete_account("acc-1").unwrap();

        let remaining: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM folders", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0, "folders must cascade on account delete");
    }
}
