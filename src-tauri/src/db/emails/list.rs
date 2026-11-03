//! The one list behind every search and filter: one row per conversation, in
//! `thread_ts, timestamp, id` order, read page by page from a cursor.
//!
//! The Tag Board keeps `get_filtered_emails` and the chat keeps `Lookup`; see
//! `docs/DECISIONS.md` for why neither shares this query.

use super::crud::mailbox_scope_sql;
use super::search::{from_match_sql, FromMatch, TagQuery};
use super::*;
use crate::db::AccountScope;
use crate::models::ThreadCursor;

type Binds = Vec<Box<dyn rusqlite::ToSql>>;

/// The view a query runs in. Validated at the command boundary, so the rest of
/// the path never sees a free-form string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListView {
    Inbox,
    Sent,
    Archive,
    Folder(String),
    Spam,
    Trash,
    Starred,
    Snoozed,
}

impl ListView {
    pub fn parse(view: &str) -> Result<Self> {
        match view {
            "inbox" => Ok(Self::Inbox),
            "sent" => Ok(Self::Sent),
            "archive" => Ok(Self::Archive),
            "spam" => Ok(Self::Spam),
            "deleted" | "trash" => Ok(Self::Trash),
            "starred" => Ok(Self::Starred),
            "snoozed" => Ok(Self::Snoozed),
            v if v.len() > "folder:".len() && v.starts_with("folder:") => Ok(Self::Folder(v.to_string())),
            other => Err(crate::models::error::AppError::InvalidInput(format!(
                "Unknown view: {other}"
            ))),
        }
    }

    /// Which mail coincides in this view. A folder path is user data, so it
    /// comes back as a bind.
    fn match_sql(&self, alias: &str, next_index: &mut usize) -> (String, Binds) {
        let view = match self {
            Self::Inbox | Self::Starred | Self::Snoozed => return (everywhere_live(alias), Vec::new()),
            Self::Sent => "sent",
            Self::Archive => "archive",
            Self::Spam => "spam",
            Self::Trash => "deleted",
            Self::Folder(path) => path.as_str(),
        };
        let scope = mailbox_scope_sql(view, &format!("{alias}."), &format!("?{next_index}"), false);
        let mut binds: Binds = Vec::new();
        if let Some(folder) = scope.folder_value {
            binds.push(Box::new(folder));
            *next_index += 1;
        }
        (scope.conditions.join(" AND "), binds)
    }

    /// Which mail counts as the conversation's activity. It always contains
    /// [`Self::match_sql`], so `thread_ts` is never NULL.
    fn thread_sql(&self, alias: &str) -> String {
        match self {
            Self::Sent => format!("{alias}.is_deleted = 0 AND {alias}.mailbox NOT IN ('spam', 'trash')"),
            Self::Spam => format!("{alias}.is_deleted = 0 AND {alias}.mailbox = 'spam'"),
            Self::Trash => format!("({alias}.mailbox = 'trash' OR {alias}.is_deleted = 1)"),
            _ => everywhere_live(alias),
        }
    }
}

/// Live mail and custom folders, the reach of a query from the inbox.
fn everywhere_live(alias: &str) -> String {
    format!("{alias}.is_deleted = 0 AND {}", Database::filterable_mailbox(alias))
}

/// Which table a query starts from. Fixed with `CROSS JOIN`, so SQLite cannot
/// invert a plan that a statistics change would otherwise flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOrigin {
    Tag,
    SenderName,
    Words,
    Domain,
    Emails,
}

/// What a query asks of the list, already interpreted.
#[derive(Debug, Clone)]
pub struct ListPlan {
    pub origin: ListOrigin,
    pub view: ListView,
    pub words: String,
    pub subject: Option<String>,
    pub from_exact: Option<String>,
    pub from_name: Option<String>,
    pub to: Option<String>,
    pub domain: Option<String>,
    pub attachment_ext: Option<String>,
    pub tags: Vec<TagQuery>,
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub message_unread: bool,
    pub thread_unread: bool,
}

impl Default for ListPlan {
    fn default() -> Self {
        Self {
            origin: ListOrigin::Emails,
            view: ListView::Inbox,
            words: String::new(),
            subject: None,
            from_exact: None,
            from_name: None,
            to: None,
            domain: None,
            attachment_ext: None,
            tags: Vec::new(),
            after: None,
            before: None,
            message_unread: false,
            thread_unread: false,
        }
    }
}

/// The FTS query for a `subject:` value, when it has a word FTS can index.
pub fn subject_fts_query(subject: &str) -> Option<String> {
    let terms: Vec<String> = subject
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| format!("subject:\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// The SQL and binds of one page, or `None` when the query cannot match
/// anything. One set of conditions for every query: the origin only names the
/// table SQLite starts from.
pub(crate) fn build_list_query(
    plan: &ListPlan,
    scope: AccountScope<'_>,
    cursor: Option<&ThreadCursor>,
    limit: i32,
) -> Option<(String, Binds)> {
    let mut binds: Binds = Vec::new();
    let mut idx = 1usize;

    let account_cond = match scope {
        AccountScope::Account(id) => {
            binds.push(Box::new(id.to_string()));
            idx += 1;
            "account_id = ?1".to_string()
        }
        AccountScope::AllEnabled => "account_id IN (SELECT id FROM accounts WHERE enabled = 1)".to_string(),
    };
    let mut conditions = vec![format!("e2.{account_cond}")];

    let (view_sql, view_binds) = plan.view.match_sql("e2", &mut idx);
    conditions.push(view_sql);
    binds.extend(view_binds);

    let mut text_queries = Vec::new();
    if !plan.words.is_empty() {
        let query = sanitize_fts_query(&plan.words);
        // A word of only symbols can match nothing; dropping the condition
        // instead would match every conversation.
        if query.is_empty() {
            return None;
        }
        text_queries.push(query);
    }
    if let Some(subject) = &plan.subject {
        match subject_fts_query(subject) {
            Some(query) => text_queries.push(query),
            None => {
                conditions.push(format!("e2.subject LIKE ?{idx}"));
                binds.push(Box::new(format!("%{subject}%")));
                idx += 1;
            }
        }
    }

    let mut source = "emails e2".to_string();
    let mut from_cte = String::new();

    if plan.origin == ListOrigin::Domain {
        source = "emails e2 INDEXED BY idx_emails_sender_domain".to_string();
    }

    let mut tags: Vec<&TagQuery> = plan.tags.iter().collect();
    if plan.origin == ListOrigin::Tag {
        if let Some(position) = tags.iter().position(|t| t.tag_type.is_some()) {
            let driver = tags.remove(position);
            source = "email_tags et INDEXED BY idx_email_tags_type_value
                      CROSS JOIN emails e2 ON e2.id = et.email_id"
                .to_string();
            conditions.push(format!(
                "et.tag_type = ?{idx} AND et.tag_value = ?{} COLLATE NOCASE",
                idx + 1
            ));
            binds.push(Box::new(driver.tag_type.clone()));
            binds.push(Box::new(driver.value.clone()));
            idx += 2;
        }
    }

    if let Some(from) = &plan.from_name {
        let (from_match, from_binds) = from_match_sql(from, &account_cond, "e2", &mut idx);
        binds.extend(from_binds);
        match from_match {
            FromMatch::Cte(sql) if plan.origin == ListOrigin::SenderName => {
                from_cte = format!("{sql},");
                source = "from_match fm CROSS JOIN emails e2 ON e2.id = fm.email_id".to_string();
            }
            FromMatch::Cte(sql) => {
                from_cte = format!("{sql},");
                conditions.push("e2.id IN (SELECT email_id FROM from_match)".to_string());
            }
            FromMatch::Condition(sql) => conditions.push(sql),
        }
    }

    for (i, query) in text_queries.into_iter().enumerate() {
        if i == 0 && plan.origin == ListOrigin::Words {
            source = format!(
                "(SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{idx}) f
                 CROSS JOIN emails e2 ON e2.id = f.email_id"
            );
        } else {
            conditions.push(format!(
                "e2.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{idx})"
            ));
        }
        binds.push(Box::new(query));
        idx += 1;
    }

    if let Some(address) = &plan.from_exact {
        conditions.push(format!("e2.sender_email = ?{idx} COLLATE NOCASE"));
        binds.push(Box::new(address.clone()));
        idx += 1;
    }
    if let Some(to) = &plan.to {
        conditions.push(format!("e2.recipients_json LIKE ?{idx}"));
        binds.push(Box::new(format!("%{to}%")));
        idx += 1;
    }
    if let Some(domain) = &plan.domain {
        conditions.push(format!("e2.sender_domain = ?{idx}"));
        binds.push(Box::new(domain.to_lowercase()));
        idx += 1;
    }
    if let Some(ext) = &plan.attachment_ext {
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM email_attachment_meta am WHERE am.email_id = e2.id AND LOWER(am.filename) LIKE ?{idx})"
        ));
        binds.push(Box::new(format!("%.{}", ext.to_lowercase())));
        idx += 1;
    }
    for tag in tags {
        let mut condition = format!(
            "EXISTS (SELECT 1 FROM email_tags t WHERE t.email_id = e2.id AND t.tag_value = ?{idx} COLLATE NOCASE"
        );
        binds.push(Box::new(tag.value.clone()));
        idx += 1;
        if let Some(tag_type) = &tag.tag_type {
            condition.push_str(&format!(" AND t.tag_type = ?{idx}"));
            binds.push(Box::new(tag_type.clone()));
            idx += 1;
        }
        condition.push(')');
        conditions.push(condition);
    }
    if let Some(after) = plan.after {
        conditions.push(format!("e2.timestamp >= ?{idx}"));
        binds.push(Box::new(after));
        idx += 1;
    }
    if let Some(before) = plan.before {
        conditions.push(format!("e2.timestamp <= ?{idx}"));
        binds.push(Box::new(before));
        idx += 1;
    }
    if plan.message_unread {
        conditions.push("e2.is_read = 0".to_string());
    }
    // Same thread seek, and the same index hint, as the inbox's unread check.
    if plan.thread_unread {
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM emails u INDEXED BY idx_emails_thread_latest \
             WHERE u.account_id = e2.account_id AND u.thread_id = e2.thread_id \
             AND u.is_read = 0 AND {})",
            plan.view.thread_sql("u")
        ));
    }

    let mut after = String::new();
    if let Some(cursor) = cursor {
        after = format!(
            " AND (l.thread_ts, e.timestamp, e.id) < (?{idx}, ?{}, ?{})",
            idx + 1,
            idx + 2
        );
        binds.push(Box::new(cursor.thread_ts));
        binds.push(Box::new(cursor.timestamp));
        binds.push(Box::new(cursor.id.clone()));
        idx += 3;
    }
    binds.push(Box::new(limit));

    let sql = format!(
        "WITH {from_cte} matched AS (
             SELECT e2.account_id AS aid, e2.thread_id AS tid, e2.id AS mid, e2.timestamp AS mts
             FROM {source}
             WHERE {conditions}
         ),
         {tail}",
        conditions = conditions.join(" AND "),
        tail = Database::thread_tail(&plan.view.thread_sql("e3"), &after, &format!("LIMIT ?{idx}")),
    );
    Some((sql, binds))
}

impl Database {
    /// One page of the list. `thread_latest_at` carries each row's `thread_ts`.
    pub fn list_threads(
        &self,
        plan: &ListPlan,
        scope: AccountScope<'_>,
        cursor: Option<&ThreadCursor>,
        limit: i32,
    ) -> Result<crate::models::FilteredEmailsResult> {
        let Some((sql, binds)) = build_list_query(plan, scope, cursor, limit) else {
            return Ok(crate::models::FilteredEmailsResult {
                emails: Vec::new(),
                total_count: -1,
                thread_latest_at: std::collections::HashMap::new(),
            });
        };
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(refs.as_slice())?;
        Self::read_representatives(&mut rows)
    }

    #[cfg(test)]
    fn explain_list_threads(&self, plan: &ListPlan, scope: AccountScope<'_>) -> Vec<String> {
        let (sql, binds) = build_list_query(plan, scope, None, 51).expect("the explained plan can match");
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(refs.as_slice()).unwrap();
        let mut plan = Vec::new();
        while let Some(row) = rows.next().unwrap() {
            plan.push(row.get::<_, String>(3).unwrap());
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::*;
    use crate::db::AccountScope::{Account, AllEnabled};

    fn ids(result: &crate::models::FilteredEmailsResult) -> Vec<&str> {
        result.emails.iter().map(|e| e.id.as_str()).collect()
    }

    fn page(db: &Database, plan: &ListPlan) -> Vec<String> {
        db.list_threads(plan, Account("acc"), None, 100)
            .unwrap()
            .emails
            .into_iter()
            .map(|e| e.id)
            .collect()
    }

    fn words(text: &str) -> ListPlan {
        ListPlan {
            origin: ListOrigin::Words,
            words: text.to_string(),
            ..ListPlan::default()
        }
    }

    fn in_view(view: ListView) -> ListPlan {
        ListPlan {
            view,
            ..ListPlan::default()
        }
    }

    fn mail(db: &Database, id: &str, thread: &str, mailbox: &str, ts: i64) {
        insert_search_email_in_mailbox(
            db,
            id,
            "acc",
            thread,
            "Ada",
            "ada@empresa.example",
            "subj",
            "invoice",
            ts,
            mailbox,
        );
    }

    #[test]
    fn a_view_name_is_validated() {
        assert_eq!(ListView::parse("deleted").unwrap(), ListView::Trash);
        assert_eq!(ListView::parse("trash").unwrap(), ListView::Trash);
        assert_eq!(
            ListView::parse("folder:Work/2026").unwrap(),
            ListView::Folder("folder:Work/2026".into())
        );
        assert!(ListView::parse("folder:").is_err());
        assert!(ListView::parse("all").is_err());
    }

    #[test]
    fn a_page_lists_one_row_per_conversation_by_its_latest_mail() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "old", "t1", "inbox", 100);
        mail(&db, "reply", "t1", "inbox", 900);
        mail(&db, "other", "t2", "inbox", 500);

        let got = db.list_threads(&words("invoice"), Account("acc"), None, 50).unwrap();

        assert_eq!(ids(&got), ["reply", "other"]);
        assert_eq!(got.thread_latest_at.get("reply"), Some(&900));
    }

    #[test]
    fn the_row_is_the_newest_match_and_the_sort_is_the_conversation() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "old_match", "t1", "inbox", 100);
        insert_search_email(
            &db,
            "reply",
            "acc",
            "t1",
            "Bob",
            "bob@otra.example",
            "s",
            "nothing",
            900,
        );
        mail(&db, "mid_match", "t2", "inbox", 500);

        let got = db.list_threads(&words("invoice"), Account("acc"), None, 50).unwrap();

        assert_eq!(ids(&got), ["old_match", "mid_match"]);
        assert_eq!(got.thread_latest_at.get("old_match"), Some(&900));
    }

    #[test]
    fn the_cursor_walks_past_rows_stamped_in_the_same_second() {
        let db = Database::new_for_testing().unwrap();
        for i in 0..7 {
            mail(&db, &format!("e{i}"), &format!("t{i}"), "inbox", 100);
        }
        let plan = words("invoice");
        let mut seen = Vec::new();
        let mut cursor: Option<ThreadCursor> = None;
        loop {
            let got = db.list_threads(&plan, Account("acc"), cursor.as_ref(), 3).unwrap();
            let Some(last) = got.emails.last() else { break };
            cursor = Some(ThreadCursor {
                thread_ts: got.thread_latest_at[&last.id],
                timestamp: last.timestamp,
                id: last.id.clone(),
            });
            seen.extend(got.emails.iter().map(|e| e.id.clone()));
        }
        assert_eq!(seen, ["e6", "e5", "e4", "e3", "e2", "e1", "e0"]);
    }

    #[test]
    fn inbox_searches_live_mail_and_folders_but_not_spam_or_trash() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "in", "t1", "inbox", 500);
        mail(&db, "arc", "t2", "archive", 400);
        mail(&db, "fol", "t3", "folder:Work", 300);
        mail(&db, "spam", "t4", "spam", 200);
        mail(&db, "trash", "t5", "trash", 100);

        assert_eq!(page(&db, &words("invoice")), ["in", "arc", "fol"]);
    }

    #[test]
    fn each_view_lists_what_it_holds() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "in", "t1", "inbox", 500);
        mail(&db, "arc", "t2", "archive", 400);
        mail(&db, "fol", "t3", "folder:Work", 300);
        mail(&db, "spam", "t4", "spam", 200);
        mail(&db, "trash", "t5", "trash", 100);
        mail(&db, "gone", "t6", "inbox", 50);
        set_email_deleted(&db, "gone");
        mail(&db, "sent", "t7", "sent", 25);

        let cases = [
            (ListView::Archive, vec!["arc"]),
            (ListView::Folder("folder:Work".into()), vec!["fol"]),
            (ListView::Spam, vec!["spam"]),
            (ListView::Trash, vec!["trash", "gone"]),
            (ListView::Sent, vec!["sent"]),
            (ListView::Starred, vec!["in", "arc", "fol", "sent"]),
            (ListView::Snoozed, vec!["in", "arc", "fol", "sent"]),
        ];
        for (view, want) in cases {
            let plan = ListPlan {
                view: view.clone(),
                ..words("invoice")
            };
            assert_eq!(page(&db, &plan), want, "{view:?}");
        }
    }

    #[test]
    fn sent_lists_its_three_ways_and_not_spam_copies() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "filed", "t1", "sent", 500);
        mail(&db, "flagged", "t2", "inbox", 400);
        db.connection()
            .execute("UPDATE emails SET is_sent = 1 WHERE id = 'flagged'", [])
            .unwrap();
        mail(&db, "own", "t3", "inbox", 300);
        db.connection()
            .execute("UPDATE accounts SET email = 'ada@empresa.example' WHERE id = 'acc'", [])
            .unwrap();
        mail(&db, "spammed", "t4", "spam", 200);

        assert_eq!(page(&db, &in_view(ListView::Sent)), ["filed", "flagged", "own"]);
    }

    #[test]
    fn thread_ts_is_never_null_in_any_view() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "in", "t1", "inbox", 500);
        mail(&db, "spam", "t2", "spam", 400);
        mail(&db, "trash", "t3", "trash", 300);
        mail(&db, "gone", "t4", "inbox", 200);
        set_email_deleted(&db, "gone");
        mail(&db, "fol", "t5", "folder:Work", 100);

        for view in [
            ListView::Inbox,
            ListView::Sent,
            ListView::Archive,
            ListView::Folder("folder:Work".into()),
            ListView::Spam,
            ListView::Trash,
            ListView::Starred,
            ListView::Snoozed,
        ] {
            let plan = in_view(view.clone());
            let (sql, binds) = build_list_query(&plan, Account("acc"), None, 50).unwrap();
            let conn = db.reader();
            let mut stmt = conn.prepare(&sql).unwrap();
            let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
            let mut rows = stmt.query(refs.as_slice()).unwrap();
            let ts_idx = EMAIL_COLUMNS.split(',').count();
            while let Some(row) = rows.next().unwrap() {
                let ts: Option<i64> = row.get(ts_idx).unwrap();
                assert!(ts.is_some(), "{view:?}");
            }
        }
    }

    #[test]
    fn a_folder_path_is_bound_not_written_into_the_sql() {
        let plan = in_view(ListView::Folder("folder:O'Brien".into()));
        let (sql, binds) = build_list_query(&plan, Account("acc"), None, 50).unwrap();
        assert!(!sql.contains("O'Brien"), "{sql}");
        assert_eq!(binds.len(), 3, "account, folder and limit");
    }

    #[test]
    fn an_exact_sender_does_not_match_a_longer_address() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "ana", "acc", "t1", "Ana", "ana@empresa.example", "s", "b", 200);
        insert_search_email(&db, "mar", "acc", "t2", "Mar", "mariana@empresa.example", "s", "b", 100);

        let plan = ListPlan {
            from_exact: Some("ANA@empresa.example".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["ana"]);
    }

    #[test]
    fn a_partial_sender_uses_the_union() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "ana", "acc", "t1", "Ana", "ana@empresa.example", "s", "b", 200);
        insert_search_email(&db, "bob", "acc", "t2", "Bob", "bob@otra.example", "s", "b", 100);

        let plan = ListPlan {
            origin: ListOrigin::SenderName,
            from_name: Some("ana@empresa".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["ana"]);
    }

    #[test]
    fn a_sender_too_short_to_tokenize_still_matches_by_address() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "ana", "acc", "t1", "Ana", "a@empresa.example", "s", "b", 200);
        insert_search_email(&db, "bob", "acc", "t2", "Bob", "b@otra.example", "s", "b", 100);

        let plan = ListPlan {
            origin: ListOrigin::SenderName,
            from_name: Some("a".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["ana"]);
    }

    #[test]
    fn a_sender_works_in_every_account() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "one", "acc", "t1", "Ana", "ana@empresa.example", "s", "b", 200);
        insert_search_email(&db, "two", "acc2", "t2", "Ana", "ana@empresa.example", "s", "b", 100);
        insert_search_email(&db, "off", "acc3", "t3", "Ana", "ana@empresa.example", "s", "b", 50);
        set_account_enabled(&db, "acc3", false);

        let plan = ListPlan {
            origin: ListOrigin::SenderName,
            from_name: Some("nadia ana".into()),
            ..ListPlan::default()
        };
        let got = db.list_threads(&plan, AllEnabled, None, 50).unwrap();
        assert!(got.emails.is_empty());

        let plan = ListPlan {
            from_name: Some("ana".into()),
            ..plan
        };
        let got = db.list_threads(&plan, AllEnabled, None, 50).unwrap();
        assert_eq!(ids(&got), ["one", "two"]);
    }

    #[test]
    fn a_tag_matches_without_regard_to_ascii_case() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "stored", "t1", "inbox", 200);
        tag_email(&db, "stored", "company", "Globex");

        let plan = ListPlan {
            origin: ListOrigin::Tag,
            tags: vec![TagQuery::typed("company", "globex")],
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["stored"]);
    }

    #[test]
    fn a_thread_counts_when_any_of_its_mail_carries_the_tag() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "tagged", "t1", "inbox", 100);
        tag_email(&db, "tagged", "intent", "urgent");
        mail(&db, "reply", "t1", "inbox", 900);

        let plan = ListPlan {
            origin: ListOrigin::Tag,
            tags: vec![TagQuery::typed("intent", "urgent")],
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["tagged"]);
    }

    #[test]
    fn every_condition_narrows_the_same_list() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "yes", "t1", "inbox", 500);
        tag_email(&db, "yes", "intent", "urgent");
        tag_email(&db, "yes", "topic", "billing");
        mail(&db, "other_tag", "t2", "inbox", 400);
        tag_email(&db, "other_tag", "intent", "urgent");
        mail(&db, "too_old", "t3", "inbox", 50);
        tag_email(&db, "too_old", "intent", "urgent");
        tag_email(&db, "too_old", "topic", "billing");

        let plan = ListPlan {
            origin: ListOrigin::Tag,
            tags: vec![TagQuery::typed("intent", "urgent"), TagQuery::any_type("billing")],
            after: Some(100),
            words: "invoice".into(),
            from_exact: Some("ada@empresa.example".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["yes"]);
    }

    #[test]
    fn before_includes_its_bound() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "on", "t1", "inbox", 100);
        mail(&db, "past", "t2", "inbox", 101);

        let plan = ListPlan {
            before: Some(100),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["on"]);
    }

    #[test]
    fn is_unread_acts_on_the_message_and_the_toggle_on_the_conversation() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "read_latest", "t1", "inbox", 500);
        db.connection()
            .execute("UPDATE emails SET is_read = 1 WHERE id = 'read_latest'", [])
            .unwrap();
        mail(&db, "unread_old", "t1", "inbox", 100);

        let message = ListPlan {
            message_unread: true,
            ..ListPlan::default()
        };
        assert_eq!(page(&db, &message), ["unread_old"]);

        let thread = ListPlan {
            thread_unread: true,
            ..ListPlan::default()
        };
        assert_eq!(page(&db, &thread), ["read_latest"]);
    }

    #[test]
    fn the_unread_toggle_follows_the_view() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "binned", "t1", "inbox", 100);
        set_email_deleted(&db, "binned");
        mail(&db, "spam_read", "t2", "spam", 200);
        db.connection()
            .execute("UPDATE emails SET is_read = 1 WHERE id = 'spam_read'", [])
            .unwrap();
        mail(&db, "live_unread", "t2", "inbox", 300);

        let trash = ListPlan {
            thread_unread: true,
            ..in_view(ListView::Trash)
        };
        assert_eq!(page(&db, &trash), ["binned"]);

        let spam = ListPlan {
            thread_unread: true,
            ..in_view(ListView::Spam)
        };
        assert!(
            page(&db, &spam).is_empty(),
            "a live message must not lift a read spam row"
        );
    }

    #[test]
    fn a_word_of_only_symbols_matches_nothing_without_running_sql() {
        assert!(build_list_query(&words("?!"), Account("acc"), None, 50).is_none());
    }

    #[test]
    fn a_short_subject_falls_back_to_like() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "q", "acc", "t1", "A", "a@e.example", "Q report", "b", 100);
        insert_search_email(&db, "n", "acc", "t2", "A", "a@e.example", "nothing", "b", 90);

        let plan = ListPlan {
            subject: Some("q".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["q"]);
    }

    #[test]
    fn a_subject_uses_the_text_index() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "q", "acc", "t1", "A", "a@e.example", "quarterly report", "b", 100);
        insert_search_email(&db, "n", "acc", "t2", "A", "a@e.example", "nothing", "quarterly", 90);

        let plan = ListPlan {
            origin: ListOrigin::Words,
            subject: Some("quarterly".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["q"]);
    }

    #[test]
    fn domain_and_attachment_narrow_the_list() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(&db, "doc", "acc", "t1", "A", "a@empresa.example", "s", "b", 200);
        insert_search_email(&db, "no", "acc", "t2", "B", "b@otra.example", "s", "b", 100);
        db.connection()
            .execute(
                "INSERT INTO email_attachment_meta (id, email_id, account_id, filename, mime_type)
                 VALUES ('m1', 'doc', 'acc', 'Report.PDF', 'application/pdf')",
                [],
            )
            .unwrap();

        let by_domain = ListPlan {
            origin: ListOrigin::Domain,
            domain: Some("Empresa.Example".into()),
            ..ListPlan::default()
        };
        assert_eq!(page(&db, &by_domain), ["doc"]);

        let by_ext = ListPlan {
            attachment_ext: Some("pdf".into()),
            ..ListPlan::default()
        };
        assert_eq!(page(&db, &by_ext), ["doc"]);
    }

    #[test]
    fn to_searches_the_recipients() {
        let db = Database::new_for_testing().unwrap();
        mail(&db, "yes", "t1", "inbox", 200);
        mail(&db, "no", "t2", "inbox", 100);
        db.connection()
            .execute(
                "UPDATE emails SET recipients_json = '[\"lia@empresa.example\"]' WHERE id = 'yes'",
                [],
            )
            .unwrap();

        let plan = ListPlan {
            to: Some("lia@".into()),
            ..ListPlan::default()
        };

        assert_eq!(page(&db, &plan), ["yes"]);
    }

    fn assert_starts_from(plan: &[String], table: &str) {
        let first = plan
            .iter()
            .find(|l| !l.starts_with("CO-ROUTINE") && !l.starts_with("MATERIALIZE") && !l.contains("compound"))
            .expect("plan is not empty");
        assert!(
            plan.iter().any(|l| l.contains(table)),
            "{table} not in {plan:#?} (first: {first})"
        );
    }

    #[test]
    fn each_origin_keeps_its_index_in_both_scopes() {
        let db = Database::new_for_testing().unwrap();
        let tag = ListPlan {
            origin: ListOrigin::Tag,
            tags: vec![TagQuery::typed("intent", "urgent")],
            ..ListPlan::default()
        };
        let domain = ListPlan {
            origin: ListOrigin::Domain,
            domain: Some("empresa.example".into()),
            ..ListPlan::default()
        };
        let sender = ListPlan {
            origin: ListOrigin::SenderName,
            from_name: Some("ana".into()),
            ..ListPlan::default()
        };
        for scope in [Account("acc"), AllEnabled] {
            assert_starts_from(&db.explain_list_threads(&tag, scope), "idx_email_tags_type_value");
            assert_starts_from(&db.explain_list_threads(&domain, scope), "idx_emails_sender_domain");
            assert_starts_from(
                &db.explain_list_threads(&sender, scope),
                "idx_emails_sender_email_nocase",
            );
            assert_starts_from(&db.explain_list_threads(&words("invoice"), scope), "emails_fts");
        }
    }

    /// Goal 9, on generated mail: `cargo test --release list_page_budget -- --ignored`.
    #[test]
    #[ignore = "generates 60,000 emails; meant for a release build"]
    fn list_page_budget() {
        let db = Database::new_for_testing().unwrap();
        {
            let conn = db.connection();
            ensure_account(&conn, "acc");
            ensure_account(&conn, "acc2");
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..60_000i64 {
                let id = format!("e{i}");
                let account = if i % 3 == 0 { "acc2" } else { "acc" };
                let thread = format!("t{}", i / 3);
                let sender = format!("person{}@empresa{}.example", i % 200, i % 40);
                let domain = format!("empresa{}.example", i % 40);
                let body = format!("word{} word{} informe", i % 500, i % 7);
                tx.execute(
                    "INSERT INTO emails
                         (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                          recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1,?2,?3,'s','Persona',?4,?5,'[]','[]','s',?6,0,'primary','inbox',0)",
                    rusqlite::params![id, account, thread, sender, domain, 1_000_000 + i],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, 's', 'Persona', ?2)",
                    rusqlite::params![id, body],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO email_tags (email_id, tag_type, tag_value, created_at) VALUES (?1, 'priority', 'low', 0)",
                    rusqlite::params![id],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }

        let plans = [
            ("all mail", ListPlan::default()),
            (
                "common tag",
                ListPlan {
                    origin: ListOrigin::Tag,
                    tags: vec![TagQuery::typed("priority", "low")],
                    ..ListPlan::default()
                },
            ),
            ("common word", words("informe")),
            (
                "sender name",
                ListPlan {
                    origin: ListOrigin::SenderName,
                    from_name: Some("person7".into()),
                    ..ListPlan::default()
                },
            ),
            (
                "two-word sender",
                ListPlan {
                    origin: ListOrigin::SenderName,
                    from_name: Some("person7 empresa7".into()),
                    ..ListPlan::default()
                },
            ),
        ];
        for (scope, budget_ms) in [(Account("acc"), 200.0), (AllEnabled, 400.0)] {
            for (name, plan) in &plans {
                let mut worst = 0f64;
                let mut cursor: Option<ThreadCursor> = None;
                for _ in 0..20 {
                    let started = std::time::Instant::now();
                    let page = db.list_threads(plan, scope, cursor.as_ref(), 51).unwrap();
                    worst = worst.max(started.elapsed().as_secs_f64() * 1000.0);
                    let Some(last) = page.emails.get(49) else { break };
                    cursor = Some(ThreadCursor {
                        thread_ts: page.thread_latest_at[&last.id],
                        timestamp: last.timestamp,
                        id: last.id.clone(),
                    });
                }
                println!("{scope:?} {name}: {worst:.0} ms");
                assert!(
                    worst <= budget_ms,
                    "{name} in {scope:?}: {worst:.0} ms over {budget_ms} ms"
                );
            }
        }
    }
}
