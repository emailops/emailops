//! The order of a mailbox list, picked with the column headers of the
//! full-width list. Pure: `crud.rs::email_list_query` puts the terms in its
//! ORDER BY, so the order holds across every page, not just the loaded one.

use crate::models::error::{AppError, Result};

/// The column a mailbox list is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSortKey {
    Date,
    Sender,
    Subject,
    Size,
}

/// How a mailbox list is ordered. The default, newest first, is the order
/// every list had before it could be sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSort {
    pub key: ListSortKey,
    pub ascending: bool,
}

impl Default for ListSort {
    fn default() -> Self {
        Self {
            key: ListSortKey::Date,
            ascending: false,
        }
    }
}

impl ListSort {
    /// Read the frontend's `sort_by` / `sort_dir`; each omitted part keeps the
    /// default. Only the known names pass, so the values never reach SQL as
    /// text.
    pub fn parse(by: Option<&str>, dir: Option<&str>) -> Result<Self> {
        let key = match by.unwrap_or("date") {
            "date" => ListSortKey::Date,
            "sender" => ListSortKey::Sender,
            "subject" => ListSortKey::Subject,
            "size" => ListSortKey::Size,
            other => return Err(AppError::InvalidInput(format!("Unknown sort column: {other}"))),
        };
        let ascending = match dir.unwrap_or("desc") {
            "asc" => true,
            "desc" => false,
            other => return Err(AppError::InvalidInput(format!("Unknown sort direction: {other}"))),
        };
        Ok(Self { key, ascending })
    }

    /// ORDER BY terms for this sort. `prefix` qualifies the columns (`"e."`,
    /// or `""` over a derived table) and `date` is the query's date expression
    /// (the inbox merge dates a woken conversation by its wake time). Rows that
    /// tie on the column stay newest first, and the id makes the order total,
    /// so OFFSET pages never repeat or skip a row. An unknown size (NULL) sorts
    /// as the smallest.
    pub fn order_by(&self, prefix: &str, date: &str) -> String {
        let dir = if self.ascending { "ASC" } else { "DESC" };
        let column = match self.key {
            ListSortKey::Date => return format!("{date} {dir}, {prefix}id {dir}"),
            ListSortKey::Sender => "sender COLLATE NOCASE",
            ListSortKey::Subject => "subject COLLATE NOCASE",
            ListSortKey::Size => "size_bytes",
        };
        format!("{prefix}{column} {dir}, {date} DESC, {prefix}id DESC")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sort(key: ListSortKey, ascending: bool) -> ListSort {
        ListSort { key, ascending }
    }

    #[test]
    fn omitted_parts_keep_newest_first() {
        assert_eq!(ListSort::parse(None, None).unwrap(), ListSort::default());
        assert_eq!(ListSort::parse(Some("date"), None).unwrap(), ListSort::default());
        assert_eq!(
            ListSort::parse(None, Some("asc")).unwrap(),
            sort(ListSortKey::Date, true)
        );
    }

    #[test]
    fn every_column_and_direction_is_read() {
        let cases = [
            ("date", "desc", sort(ListSortKey::Date, false)),
            ("date", "asc", sort(ListSortKey::Date, true)),
            ("sender", "asc", sort(ListSortKey::Sender, true)),
            ("sender", "desc", sort(ListSortKey::Sender, false)),
            ("subject", "asc", sort(ListSortKey::Subject, true)),
            ("subject", "desc", sort(ListSortKey::Subject, false)),
            ("size", "asc", sort(ListSortKey::Size, true)),
            ("size", "desc", sort(ListSortKey::Size, false)),
        ];
        for (by, dir, expected) in cases {
            assert_eq!(ListSort::parse(Some(by), Some(dir)).unwrap(), expected, "{by} {dir}");
        }
    }

    #[test]
    fn an_unknown_column_or_direction_is_rejected() {
        for (by, dir) in [
            (Some("attachment"), None),
            (Some("Sender"), None),
            (Some("sender; DROP TABLE emails"), None),
            (Some(""), None),
            (None, Some("up")),
            (None, Some("DESC")),
            (None, Some("")),
        ] {
            assert!(
                matches!(ListSort::parse(by, dir), Err(AppError::InvalidInput(_))),
                "{by:?} {dir:?}"
            );
        }
    }

    #[test]
    fn the_default_is_the_list_order_before_sorting() {
        assert_eq!(
            ListSort::default().order_by("e.", "e.timestamp"),
            crate::db::emails::thread_order_clause("e", false)
        );
    }

    #[test]
    fn each_column_orders_by_itself_then_newest_first() {
        let cases = [
            (sort(ListSortKey::Date, false), "e.timestamp DESC, e.id DESC"),
            (sort(ListSortKey::Date, true), "e.timestamp ASC, e.id ASC"),
            (
                sort(ListSortKey::Sender, true),
                "e.sender COLLATE NOCASE ASC, e.timestamp DESC, e.id DESC",
            ),
            (
                sort(ListSortKey::Sender, false),
                "e.sender COLLATE NOCASE DESC, e.timestamp DESC, e.id DESC",
            ),
            (
                sort(ListSortKey::Subject, true),
                "e.subject COLLATE NOCASE ASC, e.timestamp DESC, e.id DESC",
            ),
            (
                sort(ListSortKey::Subject, false),
                "e.subject COLLATE NOCASE DESC, e.timestamp DESC, e.id DESC",
            ),
            (
                sort(ListSortKey::Size, true),
                "e.size_bytes ASC, e.timestamp DESC, e.id DESC",
            ),
            (
                sort(ListSortKey::Size, false),
                "e.size_bytes DESC, e.timestamp DESC, e.id DESC",
            ),
        ];
        for (s, expected) in cases {
            assert_eq!(s.order_by("e.", "e.timestamp"), expected, "{s:?}");
        }
    }

    #[test]
    fn a_derived_table_orders_by_its_own_columns_and_date() {
        assert_eq!(
            sort(ListSortKey::Size, false).order_by("", "sort_ts"),
            "size_bytes DESC, sort_ts DESC, id DESC"
        );
        assert_eq!(
            sort(ListSortKey::Date, true).order_by("", "sort_ts"),
            "sort_ts ASC, id ASC"
        );
    }
}
