//! Every search and filter, from any surface, reaches the list through here:
//! `plan_list` decides what a query asks for, and `list_threads` runs one page.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::ai::ollama::{parse_search_query_patterns, ParsedSearchQuery};
use crate::db::emails::list::{subject_fts_query, ListOrigin, ListPlan, ListView};
use crate::db::emails::search::TagQuery;
use crate::db::{AccountScope, Database};
use crate::models::error::Result;
use crate::models::{Email, ThreadCursor};
use crate::services::emails::{widen_stars_to_threads, widen_unread_to_threads};
use crate::services::search::matches_parsed_filters;

/// One page of a list, as the frontend and the CLI ask for it.
#[derive(Debug, Clone)]
pub struct ListRequest<'a> {
    /// `None` lists every enabled account.
    pub account_id: Option<&'a str>,
    pub query: &'a str,
    pub view: ListView,
    /// The Unread toggle: conversations with something unread.
    pub unread_only: bool,
    /// `None` starts at the top.
    pub cursor: Option<ThreadCursor>,
    pub limit: i32,
}

/// One page of a list. `next_cursor` is `None` at the end of the list, and only
/// then, so a list never claims to be over from the size of a page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPage {
    pub emails: Vec<Email>,
    pub thread_latest_at: HashMap<String, i64>,
    pub next_cursor: Option<ThreadCursor>,
    pub parsed_query: Option<ParsedSearchQuery>,
}

/// An address with a local part and a dotted domain. Anything less is still
/// being typed, and goes to the wider sender match.
pub fn is_full_address(value: &str) -> bool {
    match value.trim().split_once('@') {
        Some((local, domain)) => {
            !local.is_empty() && !value.contains(char::is_whitespace) && domain.trim_matches('.').contains('.')
        }
        None => false,
    }
}

/// Read one `tag:` token. Unlike the chat's reader it keeps the value's case:
/// the comparison folds ASCII, and lowering here would lose `Ñ`.
fn parse_list_tag(raw: &str) -> Option<TagQuery> {
    let raw = raw.trim();
    match raw.split_once('=') {
        Some((tag_type, value)) => {
            let (tag_type, value) = (tag_type.trim(), value.trim());
            match (tag_type.is_empty(), value.is_empty()) {
                (_, true) => None,
                (true, false) => Some(TagQuery::any_type(value)),
                (false, false) => Some(TagQuery::typed(tag_type.to_lowercase(), value)),
            }
        }
        None if raw.is_empty() => None,
        None => Some(TagQuery::any_type(raw)),
    }
}

/// What a query asks of the list. Pure: one set of conditions for every query,
/// and the origin only says which table SQLite starts from.
pub fn plan_list(raw: &str, parsed: Option<&ParsedSearchQuery>, view: ListView, unread_only: bool) -> ListPlan {
    let mut plan = ListPlan {
        view,
        thread_unread: unread_only,
        ..ListPlan::default()
    };
    match parsed {
        None => plan.words = raw.trim().to_string(),
        Some(parsed) => {
            plan.words = parsed.text.clone();
            plan.subject = parsed.subject_filter.clone();
            plan.to = parsed.to_filter.clone();
            plan.cc = parsed.cc_filter.clone();
            plan.domain = parsed.domain_filter.clone();
            plan.attachment_ext = parsed.attachment_ext.clone();
            plan.has_attachment = parsed.has_attachment;
            plan.tags = parsed
                .tag_filters
                .iter()
                .filter_map(|raw| parse_list_tag(raw))
                .collect();
            plan.after = parsed.after_timestamp;
            plan.before = parsed.before_timestamp;
            plan.message_unread = parsed.is_unread == Some(true);
            match &parsed.from_filter {
                Some(from) if is_full_address(from) => plan.from_exact = Some(from.trim().to_string()),
                Some(from) => plan.from_name = Some(from.clone()),
                None => {}
            }
        }
    }
    let has_subject_terms = plan.subject.as_deref().and_then(subject_fts_query).is_some();
    plan.origin = if plan.tags.iter().any(|t| t.tag_type.is_some()) {
        ListOrigin::Tag
    } else if plan.from_name.is_some() {
        ListOrigin::SenderName
    } else if !plan.words.is_empty() || has_subject_terms {
        ListOrigin::Words
    } else if plan.domain.is_some() {
        ListOrigin::Domain
    } else {
        ListOrigin::Emails
    };
    plan
}

/// The accounts an `id:` query looks in: the given one, or every enabled one.
fn search_target_accounts(db: &Database, account_id: Option<&str>) -> Result<Vec<String>> {
    match account_id {
        Some(id) => Ok(vec![id.to_string()]),
        None => Ok(db
            .list_accounts()?
            .into_iter()
            .filter(|a| a.enabled)
            .map(|a| a.id)
            .collect()),
    }
}

/// `id:` is the one exception to the list's rules: chat's "show in list" names
/// exact mail, so it gets one page, one row per email and no view.
fn id_page(db: &Database, account_id: Option<&str>, parsed: ParsedSearchQuery) -> Result<ThreadPage> {
    let mut emails = Vec::new();
    for account in search_target_accounts(db, account_id)? {
        emails.extend(db.get_account_emails_by_ids(&account, &parsed.id_filters, None)?);
    }
    emails.retain(|email| matches_parsed_filters(email, &parsed));
    emails.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.id.cmp(&a.id)));
    Ok(ThreadPage {
        emails,
        thread_latest_at: HashMap::new(),
        next_cursor: None,
        parsed_query: Some(parsed),
    })
}

/// One page of the list a query names.
pub fn list_threads(db: &Arc<Database>, request: ListRequest<'_>) -> Result<ThreadPage> {
    let started = std::time::Instant::now();
    let parsed = parse_search_query_patterns(request.query);
    let (mut page, origin) = match parsed {
        Some(parsed) if !parsed.id_filters.is_empty() => (id_page(db, request.account_id, parsed)?, None),
        parsed => {
            let plan = plan_list(request.query, parsed.as_ref(), request.view, request.unread_only);
            let scope = match request.account_id {
                Some(id) => AccountScope::Account(id),
                None => AccountScope::AllEnabled,
            };
            let mut rows = db.list_threads(&plan, scope, request.cursor.as_ref(), request.limit + 1)?;
            let has_more = rows.emails.len() > request.limit as usize;
            rows.emails.truncate(request.limit as usize);
            let next_cursor = rows.emails.last().filter(|_| has_more).map(|last| ThreadCursor {
                thread_ts: rows.thread_latest_at.get(&last.id).copied().unwrap_or(last.timestamp),
                timestamp: last.timestamp,
                id: last.id.clone(),
            });
            let page = ThreadPage {
                emails: rows.emails,
                thread_latest_at: rows.thread_latest_at,
                next_cursor,
                parsed_query: parsed,
            };
            (page, Some(plan.origin))
        }
    };
    widen_stars_to_threads(db, &mut page.emails)?;
    widen_unread_to_threads(db, &mut page.emails)?;
    // The query's shape only: a query can hold names and addresses.
    crate::services::logger::log(
        "debug",
        "search",
        format!(
            "[{:.0}ms] list page: origin={origin:?} rows={} more={}",
            started.elapsed().as_secs_f64() * 1000.0,
            page.emails.len(),
            page.next_cursor.is_some(),
        ),
    );
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Arc<Database> {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a");
        db.seed_test_account("b");
        db
    }

    fn insert(db: &Database, id: &str, account: &str, thread: &str, ts: i64) {
        insert_from(db, id, account, thread, ts, "ana@empresa.example");
    }

    fn insert_from(db: &Database, id: &str, account: &str, thread: &str, ts: i64, sender: &str) {
        let domain = sender.rsplit_once('@').map(|(_, d)| d).unwrap_or_default();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
             VALUES (?1,?2,?3,'factura','Ana',?5,?6,'[]','[]','s',?4,0,'primary','inbox',0)",
            rusqlite::params![id, account, thread, ts, sender, domain],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, 'factura', 'Ana', 'cuerpo')",
            rusqlite::params![id],
        )
        .unwrap();
    }

    fn request<'a>(account_id: Option<&'a str>, query: &'a str, cursor: Option<ThreadCursor>) -> ListRequest<'a> {
        ListRequest {
            account_id,
            query,
            view: ListView::Inbox,
            unread_only: false,
            cursor,
            limit: 50,
        }
    }

    /// Bottom of the list, as the scroll trigger would reach it, calling
    /// `between` after every page so a test can change the mailbox mid-walk.
    fn walk(
        db: &Arc<Database>,
        account_id: Option<&str>,
        query: &str,
        mut between: impl FnMut(usize, &Database),
    ) -> Vec<String> {
        let mut seen = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page = list_threads(db, request(account_id, query, cursor)).unwrap();
            seen.extend(page.emails.into_iter().map(|e| e.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                return seen;
            }
            pages += 1;
            assert!(pages < 100, "the walk must end");
            between(pages, db);
        }
    }

    #[test]
    fn a_walk_reaches_the_end_past_the_old_cap_in_every_account() {
        let db = db();
        for i in 0..130 {
            insert(&db, &format!("a{i}"), "a", &format!("ta{i}"), 1000 + i);
        }
        for i in 0..30 {
            insert(&db, &format!("b{i}"), "b", &format!("tb{i}"), 500 + i);
        }

        let unified = walk(&db, None, "factura", |_, _| {});
        assert_eq!(unified.len(), 160);
        assert_eq!(unified.iter().collect::<std::collections::HashSet<_>>().len(), 160);

        assert_eq!(walk(&db, Some("a"), "factura", |_, _| {}).len(), 130);
    }

    #[test]
    fn a_row_that_stops_matching_mid_walk_costs_no_other_row() {
        let db = db();
        for i in 0..120 {
            insert(&db, &format!("e{i:03}"), "a", &format!("t{i}"), 1000 - i);
        }
        let mut first_page_of_each = 0;

        // `is:unread` with the first ten rows of every page opened meanwhile:
        // numbering rows would skip ten per page.
        let seen = walk(&db, Some("a"), "is:unread", |page, db| {
            for i in 0..10 {
                let id = format!("e{:03}", first_page_of_each + i);
                db.connection()
                    .execute("UPDATE emails SET is_read = 1 WHERE id = ?1", [id])
                    .unwrap();
            }
            first_page_of_each = page * 50;
        });

        assert_eq!(seen.len(), 120);
    }

    #[test]
    fn a_conversation_that_jumps_up_mid_walk_appears_once() {
        let db = db();
        for i in 0..120 {
            insert(&db, &format!("e{i:03}"), "a", &format!("t{i}"), 1000 - i);
        }

        // A row below the cursor gets a reply and rises above it. This walk has
        // no refresh; in the app the next refresh brings it in at the top.
        let seen = walk(&db, Some("a"), "factura", |page, db| {
            if page == 1 {
                insert(db, "reply", "a", "t100", 5000);
            }
        });

        assert_eq!(seen.iter().collect::<std::collections::HashSet<_>>().len(), seen.len());
        assert_eq!(
            seen.len(),
            119,
            "t100 left the walk by moving above the cursor: the hole the design accepts"
        );
    }

    #[test]
    fn a_filter_and_a_wider_query_apply_the_same_rules() {
        let db = db();
        insert(&db, "e1", "a", "t1", 100);
        insert_from(&db, "e2", "a", "t2", 200, "mariana@empresa.example");
        db.connection()
            .execute(
                "INSERT INTO email_junk (email_id, account_id, spam_score, phish_score, gray_score, band,
                     primary_kind, reasons_json, method, model_version, scored_at)
                 VALUES ('e1', 'a', 0.9, 0, 0, 'junk', 'spam', '[]', 'deterministic', 1, 0)",
                [],
            )
            .unwrap();

        let filter = list_threads(&db, request(Some("a"), "from:ana@empresa.example", None)).unwrap();
        let wider = list_threads(
            &db,
            request(Some("a"), "from:ana@empresa.example after:01/01/1970", None),
        )
        .unwrap();

        let ids = |page: &ThreadPage| page.emails.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(
            ids(&filter),
            ["e1"],
            "flagged mail stays and a longer address does not match"
        );
        assert_eq!(ids(&filter), ids(&wider));
    }

    #[test]
    fn a_filter_counter_matches_the_rows_it_lists() {
        let db = db();
        insert(&db, "plain", "a", "t1", 400);
        insert(&db, "foldered", "a", "t2", 300);
        insert(&db, "flagged", "a", "t3", 200);
        insert(&db, "spammed", "a", "t4", 100);
        {
            let conn = db.connection();
            conn.execute("UPDATE emails SET mailbox = 'folder:Work' WHERE id = 'foldered'", [])
                .unwrap();
            conn.execute("UPDATE emails SET mailbox = 'spam' WHERE id = 'spammed'", [])
                .unwrap();
            conn.execute(
                "INSERT INTO email_junk (email_id, account_id, spam_score, phish_score, gray_score, band,
                     primary_kind, reasons_json, method, model_version, scored_at)
                 VALUES ('flagged', 'a', 0.9, 0, 0, 'junk', 'spam', '[]', 'deterministic', 1, 0)",
                [],
            )
            .unwrap();
        }
        for id in ["plain", "foldered", "flagged", "spammed"] {
            db.upsert_email_tag(id, "company", "acme", None).unwrap();
        }
        let scope = AccountScope::Account("a");
        let listed = |query: &str| walk(&db, Some("a"), query, |_, _| {}).len() as i32;

        assert_eq!(listed("from:ana@empresa.example"), 3);
        assert_eq!(
            db.count_filter_threads(scope, "sender", "ana@empresa.example").unwrap(),
            listed("from:ana@empresa.example")
        );
        assert_eq!(
            db.count_filter_threads(scope, "domain", "empresa.example").unwrap(),
            listed("domain:empresa.example")
        );
        assert_eq!(
            db.count_filter_threads(scope, "company", "acme").unwrap(),
            listed("tag:company=acme")
        );

        let window = crate::models::EmailWindow {
            match_list: true,
            ..Default::default()
        };
        let stats = db.get_tag_board_stats(scope, "company", &window, 0, 10).unwrap();
        assert_eq!(stats[0].count, listed("tag:company=acme"));

        let suggestions = db.get_quick_filter_stats(scope, &[], &[]).unwrap();
        assert_eq!(suggestions.top_domains[0].count, listed("domain:empresa.example"));
    }

    fn ids_of(db: &Arc<Database>, query: &str) -> Vec<String> {
        let page = list_threads(db, request(Some("a"), query, None)).unwrap();
        page.emails.into_iter().map(|e| e.id).collect()
    }

    #[test]
    fn a_typed_tag_ignores_a_company_of_the_same_name() {
        let db = db();
        insert(&db, "billing_intent", "a", "t1", 200);
        insert(&db, "billing_company", "a", "t2", 100);
        db.upsert_email_tag("billing_intent", "intent", "billing", None)
            .unwrap();
        db.upsert_email_tag("billing_company", "company", "billing", None)
            .unwrap();

        assert_eq!(ids_of(&db, "tag:intent=billing"), ["billing_intent"]);
        assert_eq!(ids_of(&db, "tag:billing"), ["billing_intent", "billing_company"]);
    }

    #[test]
    fn a_query_spans_every_category() {
        let db = db();
        insert(&db, "primary", "a", "t1", 200);
        insert(&db, "promo", "a", "t2", 100);
        db.connection()
            .execute("UPDATE emails SET category = 'promotions' WHERE id = 'promo'", [])
            .unwrap();

        assert_eq!(ids_of(&db, "factura"), ["primary", "promo"]);
    }

    #[test]
    fn id_still_applies_the_other_operators() {
        let db = db();
        insert(&db, "e1", "a", "t1", 100);
        insert_from(&db, "e2", "a", "t2", 200, "bob@otra.example");

        assert_eq!(ids_of(&db, "id:e1 id:e2 from:bob"), ["e2"]);
    }

    #[test]
    fn id_lists_exact_mail_in_every_account_on_one_page() {
        let db = db();
        insert(&db, "e1", "a", "t1", 100);
        insert(&db, "e2", "b", "t2", 200);
        insert(&db, "e3", "b", "t2", 300);

        let page = list_threads(&db, request(None, "id:e1 id:e2 id:e3", None)).unwrap();

        let ids: Vec<_> = page.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["e3", "e2", "e1"]);
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn a_word_finds_its_form_without_accents() {
        let db = db();
        insert(&db, "e1", "a", "t1", 100);

        let page = list_threads(&db, request(Some("a"), "fáctura", None)).unwrap();

        assert_eq!(page.emails.len(), 1);
    }

    #[test]
    fn only_a_full_address_is_exact() {
        for (value, want) in [
            ("ana@empresa.example", true),
            ("ANA@empresa.example", true),
            ("ana@empresa", false),
            ("@empresa.example", false),
            ("ana@", false),
            ("ana", false),
            ("ana maria@empresa.example", false),
        ] {
            assert_eq!(is_full_address(value), want, "{value}");
        }
    }

    #[test]
    fn a_tag_token_keeps_its_value_and_folds_its_type() {
        assert_eq!(
            parse_list_tag("Company=Ñandú Ltd"),
            Some(TagQuery::typed("company", "Ñandú Ltd"))
        );
        assert_eq!(parse_list_tag("Urgent"), Some(TagQuery::any_type("Urgent")));
        assert_eq!(parse_list_tag("=urgent"), Some(TagQuery::any_type("urgent")));
        assert_eq!(parse_list_tag("intent="), None);
        assert_eq!(parse_list_tag(""), None);
    }

    fn origin_of(query: &str) -> ListOrigin {
        let parsed = parse_search_query_patterns(query);
        plan_list(query, parsed.as_ref(), ListView::Inbox, false).origin
    }

    #[test]
    fn the_origin_follows_the_most_selective_condition() {
        for (query, want) in [
            ("tag:intent=urgent factura from:ana", ListOrigin::Tag),
            ("tag:urgent", ListOrigin::Emails),
            ("from:ana factura", ListOrigin::SenderName),
            ("from:ana@empresa.example factura", ListOrigin::Words),
            ("factura", ListOrigin::Words),
            ("subject:quarterly", ListOrigin::Words),
            ("subject:q", ListOrigin::Emails),
            ("domain:empresa.example", ListOrigin::Domain),
            ("from:ana@empresa.example", ListOrigin::Emails),
            ("to:lia@empresa.example", ListOrigin::Emails),
            ("after:01/01/2026", ListOrigin::Emails),
        ] {
            assert_eq!(origin_of(query), want, "{query}");
        }
    }

    #[test]
    fn the_plan_takes_the_boolean_text_cc_and_attachment_operators() {
        let query = "from:ana (factura OR recibo) -borrador cc:bea -has:attachment";
        let parsed = parse_search_query_patterns(query);
        let plan = plan_list(query, parsed.as_ref(), ListView::Inbox, false);

        assert_eq!(plan.words, "(factura OR recibo) -borrador");
        assert_eq!(plan.cc.as_deref(), Some("bea"));
        assert_eq!(plan.has_attachment, Some(false));
    }

    #[test]
    fn a_boolean_query_next_to_an_operator_lists_what_it_names() {
        let db = db();
        insert(&db, "e1", "a", "t1", 300);
        insert(&db, "e2", "a", "t2", 200);
        db.connection()
            .execute(
                "INSERT INTO email_attachment_meta (id, email_id, account_id, filename, mime_type)
                 VALUES ('m1', 'e1', 'a', 'factura.pdf', 'application/pdf')",
                [],
            )
            .unwrap();

        assert_eq!(
            walk(&db, Some("a"), "has:attachment factura OR nada", |_, _| {}),
            ["e1"]
        );
        assert_eq!(walk(&db, Some("a"), "-has:attachment factura -nada", |_, _| {}), ["e2"]);
    }

    #[test]
    fn the_toggle_and_is_unread_stay_apart() {
        let parsed = parse_search_query_patterns("is:unread");
        let plan = plan_list("is:unread", parsed.as_ref(), ListView::Inbox, false);
        assert!(plan.message_unread && !plan.thread_unread);

        let plan = plan_list("factura", None, ListView::Inbox, true);
        assert!(plan.thread_unread && !plan.message_unread);
    }
}
