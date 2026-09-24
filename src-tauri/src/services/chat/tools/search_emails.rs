use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SearchPage, Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::emails::search::TagQuery;
use crate::db::Database;
use crate::models::Email;
use crate::services::chat::{
    format_search_emails_output, format_search_emails_output_with_bodies, or_fallback_search, parse_iso_date_secs,
};
use crate::services::classification::TagGlossary;
use crate::services::{emails, thread_clean};

/// How far the total-count probe looks when a page comes back full. Past
/// this the note reports a floor ("500+"); counting further costs a wider
/// scan for a number nobody needs exactly.
const COUNT_PROBE_LIMIT: i32 = 500;

/// The line that leads a paged result so the model never presents the page
/// size as the total ("¿cuántos correos de X hay?" → "25" on a sender with
/// 156), and knows whether another page exists. `None` when this page holds
/// every match — there is nothing more to say.
fn total_count_note(shown: usize, offset: i32, total: i32) -> Option<String> {
    let first = offset + 1;
    let last = offset + shown as i32;
    if offset == 0 && last >= total {
        return None;
    }
    let total_text = if total >= COUNT_PROBE_LIMIT {
        format!("{COUNT_PROBE_LIMIT}+")
    } else {
        total.to_string()
    };
    // "muéstrame los últimos 5 correos de X" listed 5 and never said there
    // were 13: the total has to be something to say, not just a range.
    let tail = if last < total {
        format!(
            "tell the user there are {total_text} in total; call next_page for the next ones, or narrow with \
since/until, from, or a keyword"
        )
    } else {
        "this is the last page".to_string()
    };
    Some(format!(
        "(showing {first}-{last} of {total_text} matching threads — {tail})"
    ))
}

/// The line that explains a widened page: how many rows carry the tag that was
/// asked for, and how many were added behind them.
///
/// An email stores exactly ONE intent (`PRIMARY KEY (email_id, tag_type)`),
/// while a real email is often several things at once — a request that is also
/// a question — and older mail carries no tag at all. So an intent/topic filter
/// is a preference here, not a gate: the tagged rows come first, the rest of the
/// matches follow, and this note keeps the two apart so a count stays honest.
/// `None` when the page filled with tagged rows and nothing was added.
fn tag_coverage_note(tagged: usize, added: usize) -> Option<String> {
    if added == 0 {
        return None;
    }
    if tagged == 0 {
        return Some(format!(
            "(no email carries the intent/topic asked for; the {added} rows below match every other \
filter and are shown instead — each email stores only ONE intent, so the tag may simply be a \
different one)"
        ));
    }
    Some(format!(
        "({tagged} emails carry the intent/topic asked for; the {added} rows after them match every \
other filter but are tagged differently or not classified — each email stores only ONE intent, \
so count only the first {tagged} when asked how many)"
    ))
}

/// The address of the mailbox a tool call is scoped to, for the zero-result
/// line. A lookup failure degrades to an empty string — the message then keeps
/// its original wording rather than leaking an account id.
fn scoped_account_email(ctx: &ToolCtx<'_>) -> String {
    ctx.db
        .get_account(ctx.account_id)
        .ok()
        .flatten()
        .map(|a| a.email)
        .unwrap_or_default()
}

/// The zero-result line, naming the mailbox actually searched. Chat answers
/// from ONE account (see `docs/DECISIONS.md`, 2026-08-14), so a bare "No
/// matching emails found." reads as "you have no such email" when it only ever
/// meant "not in this account" — and the model reported that absence as fact.
/// Pure so every empty branch renders the same shape.
fn no_match_message(account_email: &str, prefix: &str, parenthetical: Option<&str>) -> String {
    let scope = if account_email.trim().is_empty() {
        String::new()
    } else {
        format!(" in {account_email}")
    };
    match parenthetical {
        Some(note) => format!("{prefix}No matching emails found{scope} ({note})."),
        None => format!("{prefix}No matching emails found{scope}."),
    }
}

/// The zero-result line for this call, plus a pointer to the calendar when
/// the account has one: a question about something scheduled searched the
/// mailbox, found nothing and gave up while the event sat in the calendar.
fn empty_result(ctx: &ToolCtx<'_>, prefix: &str, parenthetical: Option<&str>) -> String {
    let mut out = no_match_message(&scoped_account_email(ctx), prefix, parenthetical);
    let has_calendar = ctx.db.get_account(ctx.account_id).ok().flatten().is_some_and(|a| {
        crate::sync::calendar_provider::provider_supports_calendar(&a.provider)
            && ctx.db.calendar_enabled(&a.id).unwrap_or(false)
    });
    if has_calendar {
        out.push_str(
            " If the question is about a meeting or another scheduled event, call list_calendar_events \
now (with no range it covers last week and the next) and answer from it — do not offer to check.",
        );
    }
    out
}

/// How many candidates the semantic ranker is asked for when other filters
/// still have to be applied on top of it: meaning-ranked hits are cheap to
/// over-fetch and a sender or date filter can discard most of them.
const SEMANTIC_OVERFETCH: usize = 40;

/// Filters applied to a meaning-ranked candidate list after retrieval. The
/// keyword path pushes these into SQL; the semantic path ranks first and
/// filters second, so the same `from`/`to`/date/tag semantics are re-applied
/// here in plain Rust. Strings are matched case-insensitively.
#[derive(Default)]
pub(crate) struct PostFilters<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    /// "Emails with X": any of these terms in the sender, recipients or cc
    /// (see `emails::participant_terms`). Empty is no filter.
    pub participants: &'a [String],
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Intent / topic filters that must ALL hold on the email.
    pub tags: &'a [TagQuery],
    pub received_only: bool,
    /// Keep only mail the user has not read.
    pub unread_only: bool,
}

/// Split ranked candidates into the ones carrying every requested tag and the
/// rest, both in their original order.
///
/// The keyword path widens a short tag-filtered page with the other matches;
/// this is the same idea on the semantic path, where the candidates are
/// already ranked by meaning: prefer the tagged ones, keep the rest behind
/// them instead of dropping them.
pub(crate) fn partition_by_tags(
    emails: Vec<Email>,
    tags: &[TagQuery],
    tags_of: &dyn Fn(&str) -> Vec<(String, String)>,
) -> (Vec<Email>, Vec<Email>) {
    if tags.is_empty() {
        return (Vec::new(), emails);
    }
    emails.into_iter().partition(|e| {
        let have = tags_of(&e.id);
        tags.iter().all(|want| {
            have.iter().any(|(tag_type, value)| {
                value.eq_ignore_ascii_case(&want.value)
                    && want
                        .tag_type
                        .as_ref()
                        .is_none_or(|wanted| tag_type.eq_ignore_ascii_case(wanted))
            })
        })
    })
}

/// Keep the candidates that satisfy every filter, in their original order.
/// `tags_of` yields the classifier tags — type and value — attached to an id.
pub(crate) fn semantic_post_filter(
    emails: Vec<Email>,
    f: &PostFilters<'_>,
    tags_of: &dyn Fn(&str) -> Vec<(String, String)>,
) -> Vec<Email> {
    let contains_ci = |haystack: &str, needle: &str| haystack.to_lowercase().contains(&needle.to_lowercase());
    emails
        .into_iter()
        .filter(|e| {
            if let Some(from) = f.from {
                if !contains_ci(&e.sender, from) && !contains_ci(&e.sender_email, from) {
                    return false;
                }
            }
            if let Some(to) = f.to {
                if !e.recipients.iter().chain(e.cc.iter()).any(|r| contains_ci(r, to)) {
                    return false;
                }
            }
            if !f.participants.is_empty() {
                let exchanged = f.participants.iter().any(|p| {
                    contains_ci(&e.sender, p)
                        || contains_ci(&e.sender_email, p)
                        || e.recipients.iter().chain(e.cc.iter()).any(|r| contains_ci(r, p))
                });
                if !exchanged {
                    return false;
                }
            }
            if f.since.is_some_and(|s| e.timestamp < s) || f.until.is_some_and(|u| e.timestamp >= u) {
                return false;
            }
            if f.received_only && e.is_sent {
                return false;
            }
            if f.unread_only && e.is_read {
                return false;
            }
            if !f.tags.is_empty() {
                let have = tags_of(&e.id);
                let holds = |want: &TagQuery| {
                    have.iter().any(|(tag_type, value)| {
                        value.eq_ignore_ascii_case(&want.value)
                            && want
                                .tag_type
                                .as_ref()
                                .is_none_or(|wanted| tag_type.eq_ignore_ascii_case(wanted))
                    })
                };
                if !f.tags.iter().all(holds) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// The LLM-facing schema, rendered from the user's tag glossary so the
/// intent / topic menus (and their one-line meanings) follow Settings.
fn parameters_schema_with(glossary: &TagGlossary) -> Value {
    let intent_desc = format!(
        "Filter by the classifier's intent tag — WHAT THE SENDER WANTS. Use it when the question describes a kind of mail rather than words it contains (leads, complaints, quote requests, newsletters, cold outreach…); pick the tag whose definition matches and combine with from/to/since/until/query as needed. Values: {}.",
        TagGlossary::render_inline(&glossary.intents)
    );
    let topic_desc = format!(
        "Filter by the classifier's topic tag — WHAT THE MAIL IS ABOUT. Values: {}.",
        TagGlossary::render_inline(&glossary.topics)
    );
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Full-text keywords to match in subject/body. Leave empty when filtering purely by sender or date." },
            "mode": { "type": "string", "enum": ["keyword", "semantic"], "description": "How `query` is matched. 'keyword' (default): exact full-text match — best for names, codes, invoice numbers and distinctive words. 'semantic': meaning-based ranking of the whole mailbox — use when the question describes mail by meaning and the wording may differ ('emails where I ask a supplier for a quote', 'someone unhappy with a delivery'), when no intent/topic tag fits, or after a keyword search found nothing relevant. Semantic results are ranked by relevance, not date; the other filters still apply." },
            "from": { "type": "string", "description": "Filter by sender. Matches email address prefix (e.g. 'alice@emailops.com') or display name substring (e.g. 'Alice Smith')." },
            "to": { "type": "string", "description": "Filter by recipient — use this when the user says 'enviada a X' / 'sent to X' / 'para X'. Matches the To/CC field (substring, e.g. 'billing@emailops.com' or 'emailops.com')." },
            "with": { "type": "string", "description": "A person the mail was exchanged with, in either direction — use for 'emails with X' / 'correos con X' / 'my conversations with X'. Matches X as the sender or among the recipients, including mail sent to the addresses X writes from. Use instead of from/to when the question gives no direction." },
            "subject": { "type": "string", "description": "Filter by subject keywords (FTS5 match on subject column)." },
            "since": { "type": "string", "description": "Only return emails on or after this date. ISO-8601 date 'YYYY-MM-DD' (UTC). Example: '2026-04-17' for today." },
            "until": { "type": "string", "description": "Only return emails strictly before this date. ISO-8601 date 'YYYY-MM-DD' (UTC). Example: use until='2026-04-18' together with since='2026-04-17' to get today's emails only." },
            "limit": { "type": "integer", "description": "Max number of results to return. Default 20, max 25. Use 25 for 'all X' / 'todas' queries, 5 for 'latest X' / 'última'." },
            "offset": { "type": "integer", "description": "Skip this many matches before the page starts. Default 0 (the newest matches). To walk further into a long result set call `next_page` instead of setting this by hand." },
            "order": { "type": "string", "enum": ["newest", "oldest"], "description": "Sort direction. Default 'newest' (most recent first). Use 'oldest' with limit=1 for 'first / earliest' queries ('first email I sent to X', 'primer correo', 'el más antiguo')." },
            "intent": { "type": "string", "enum": glossary.intent_names(), "description": intent_desc },
            "topic": { "type": "string", "enum": glossary.topic_names(), "description": topic_desc },
            "with_bodies": { "type": "boolean", "description": "Return each email's cleaned body (budgeted per row) in this same call. Set it when you will summarise or extract from the results, instead of calling get_email_body once per email." },
            "unread": { "type": "boolean", "description": "true = only mail the user has not read yet. Combine with order/limit ('oldest unread' = order 'oldest', limit 1) and any other filter. Rows the user has not read are marked `unread` in every result." }
        },
        "required": []
    })
}

pub struct SearchEmailsTool;

#[async_trait]
impl Tool for SearchEmailsTool {
    fn name(&self) -> &'static str {
        "search_emails"
    }

    fn description(&self) -> &'static str {
        "Search the user's emails. Returns a list of matching emails with id, thread_id, subject, sender, date, category and a short snippet — THE SNIPPET DOES NOT INCLUDE ATTACHMENT FILENAMES. Results are grouped by Gmail category in priority order: Primary first (real people / direct mail), then Updates (receipts, shipping, automated notifications), then Other (social, forums, promotions). Keep that ordering when you summarise the results to the user. Combine filters to narrow results. Use `from` when the user asks about mail RECEIVED from someone ('de alice', 'from bob'); use `to` when they ask about mail SENT to someone ('enviada a emailops', 'para maria'). When the user keeps narrowing keywords (e.g. 'factura de emailops'), keep BOTH `query='factura'` AND `from/to='...emailops...'` — never drop the keyword. A date-bounded lookup is much more precise than a bare keyword query. At least one of query / from / to / subject / since / until / intent / topic must be non-empty. `intent` / `topic` reach the classifier's tags — the way to find a KIND of mail the question describes (their meanings are listed on the parameters); `mode='semantic'` ranks by meaning when wording varies. Spam and phishing flagged by the junk detector are never returned. When more emails match than the page shows, the result starts with '(showing N of M matching threads …)' — M is the real total; use it for 'how many' questions instead of counting rows. An `intent`/`topic` filter RANKS rather than excludes: each email stores only one intent, so the tagged rows come first and the other matches follow, and the result says how many carry the tag — count those for a 'how many of this kind', and treat the rest as what else matched. REQUIRED CHAIN: if the user asked about invoices / facturas / recibos / PDFs / attached documents, you MUST call `get_attachments(email_id)` on the top matching email before writing your final answer — the snippet alone is not enough to name the attached file."
    }

    fn prompt_summary(&self) -> &'static str {
        "search the inbox; ≥1 filter required, results newest-first. `from` = mail RECEIVED (\"from alice\"), `to` = mail SENT (\"sent to emailops\") — do not conflate."
    }

    fn parameters_schema(&self) -> Value {
        parameters_schema_with(&TagGlossary::defaults())
    }

    fn parameters_schema_for(&self, db: &Database) -> Value {
        parameters_schema_with(&TagGlossary::load(db))
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
        let from_filter = args
            .get("from")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let to_filter = args
            .get("to")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let with_filter = args
            .get("with")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        // The person's name plus the addresses they write from.
        let participants: Vec<String> = with_filter
            .map(|w| emails::resolve_participant(ctx.db, ctx.account_id, w))
            .unwrap_or_default();
        let participants_arg: Option<&[String]> = (!participants.is_empty()).then_some(participants.as_slice());
        let subject_filter = args
            .get("subject")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let since_str = args.get("since").and_then(|v| v.as_str());
        let until_str = args.get("until").and_then(|v| v.as_str());
        // Internal, heuristic-only arg (deliberately absent from
        // `parameters_schema`, so the LLM never sees or sets it). The summary
        // shortcuts preseed it so each result carries its full cleaned body —
        // letting a weak local model summarise complete emails in one pass
        // instead of chaining a `get_email_body` call per row.
        // `with_bodies` is the model-facing name; `include_bodies` the
        // shortcuts' internal one — same effect.
        let include_bodies = args.get("include_bodies").and_then(|v| v.as_bool()).unwrap_or(false)
            || args.get("with_bodies").and_then(|v| v.as_bool()).unwrap_or(false);
        // Classification filters: intent / topic, carried WITH their type so a
        // company or topic that shares the name cannot answer for them.
        let tag_filters: Vec<TagQuery> = ["intent", "topic"]
            .iter()
            .filter_map(|k| {
                args.get(*k)
                    .and_then(|v| v.as_str())
                    .map(|v| (*k, v.trim().to_lowercase()))
            })
            .filter(|(_, v)| !v.is_empty())
            .map(|(tag_type, value)| TagQuery::typed(tag_type, value))
            .collect();
        let tag_filter_arg: Option<&[TagQuery]> = if tag_filters.is_empty() {
            None
        } else {
            Some(&tag_filters)
        };
        // Internal too: the "emails I received today/this week" shortcuts set
        // it so the user's own sent replies do not show up as received mail.
        let received_only = args.get("received_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let unread_only = args.get("unread").and_then(|v| v.as_bool()).unwrap_or(false);
        // Sort direction: "oldest" (ascending) is the only way to surface the
        // FIRST email matching a filter ("primer correo", "first email I sent to
        // X"). Anything other than "oldest" keeps the default newest-first.
        let ascending = args
            .get("order")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().eq_ignore_ascii_case("oldest"))
            == Some(true);

        // All of query / from / to / subject empty would scan the whole
        // mailbox. Reject early so the model corrects its call instead of
        // us dumping N random rows.
        if query.is_empty()
            && from_filter.is_none()
            && to_filter.is_none()
            && with_filter.is_none()
            && subject_filter.is_none()
            && since_str.is_none()
            && until_str.is_none()
            && tag_filter_arg.is_none()
            && !unread_only
        {
            // Include a concrete call to imitate: a flaky model that emitted a
            // name-only call (`search_emails({})`) recovers far more reliably
            // from an example than from a list of parameter names.
            return Ok(ToolOutput::text(
                "Error: search_emails requires at least one of query / from / to / subject / since / until. \
Example: search_emails({\"from\": \"alice@example.com\", \"limit\": 25}).",
            ));
        }

        let since_ts = match since_str {
            Some(s) => match parse_iso_date_secs(s) {
                Ok(ts) => Some(ts),
                Err(e) => return Ok(ToolOutput::text(format!("Error: invalid 'since' date: {}", e))),
            },
            None => None,
        };
        let until_ts = match until_str {
            Some(s) => match parse_iso_date_secs(s) {
                Ok(ts) => Some(ts),
                Err(e) => return Ok(ToolOutput::text(format!("Error: invalid 'until' date: {}", e))),
            },
            None => None,
        };

        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20).clamp(1, 25) as i32;
        // Paging is applied after the DB call: the search is thread-deduped and
        // ordered, so the page is a slice of the first `offset + limit` rows.
        let offset = args
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .clamp(0, COUNT_PROBE_LIMIT as i64) as i32;

        // An explicit sender / recipient / subject lookup must not be silently
        // narrowed by the chat turn's category scope (default ["primary"]). That
        // scope is meant for broad keyword/RAG retrieval; when the user names a
        // target we return the newest matching mail regardless of Gmail category
        // — a newsletter landing in `updates` must still surface for `from:X`.
        let has_explicit_target =
            from_filter.is_some() || to_filter.is_some() || with_filter.is_some() || subject_filter.is_some();
        let cat_filter: Option<&[String]> = if has_explicit_target || ctx.categories.is_empty() {
            None
        } else {
            Some(ctx.categories)
        };

        // ── Semantic mode: rank by meaning, then filter ─────────────────
        // Reuses the chat's hybrid retrieval (embeddings + FTS + fusion) so a
        // question phrased unlike the mail it wants ("where I ask a supplier
        // for a quote") still lands. Needs the AI provider for the query
        // embedding; without one (or with an empty query) it degrades to the
        // keyword path below and says so, rather than failing the call.
        let semantic = args
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|m| m.trim().eq_ignore_ascii_case("semantic"))
            == Some(true);
        let mut mode_note: Option<&str> = None;
        if semantic && query.is_empty() {
            mode_note =
                Some("(mode=semantic needs a `query` to rank by — ran the other filters as a keyword search)\n");
        }
        if semantic && !query.is_empty() {
            match crate::services::ai::AiService::load_provider(ctx.db) {
                Ok(provider) => {
                    let post = PostFilters {
                        from: from_filter,
                        to: to_filter,
                        participants: &participants,
                        since: since_ts,
                        until: until_ts,
                        tags: &tag_filters,
                        received_only,
                        unread_only,
                    };
                    return self
                        .execute_semantic(ctx, provider.as_ref(), query, cat_filter, &post, limit, include_bodies)
                        .await;
                }
                Err(e) => {
                    crate::services::logger::log(
                        "warn",
                        "chat",
                        format!("search_emails: semantic mode unavailable ({e}); using keyword search"),
                    );
                    mode_note = Some("(semantic search unavailable — keyword match instead)\n");
                }
            }
        }

        let primary = emails::search_emails_filtered(
            ctx.db,
            ctx.account_id,
            query,
            cat_filter,
            from_filter,
            to_filter,
            subject_filter,
            since_ts,
            until_ts,
            tag_filter_arg,
            offset + limit,
            ascending,
            unread_only,
            participants_arg,
        );

        // Each successful branch below builds `ToolOutput::text_with_email_refs`
        // so the chat-turn aggregator can validate any `email://EMAIL_ID`
        // link the LLM later emits about these results.
        let ids = |emails: &[crate::models::Email]| -> Vec<String> { emails.iter().map(|e| e.id.clone()).collect() };

        // Fetch + clean the full body of each result, keyed by id. Reuses the
        // same cleaning pipeline as the `get_email_body` tool so the model sees
        // bodies nearly whole (HTML/quotes/signatures stripped) rather than the
        // raw stored payload. Bodies that fail to load are simply absent from
        // the map; the formatter then falls back to the snippet for that row.
        let fetch_bodies = |emails: &[crate::models::Email]| -> std::collections::HashMap<String, String> {
            // Fair-share a tight total budget across the rows so one long
            // newsletter can't swallow the context and derail the summary — each
            // row only needs a gist, not the full 8000-char body.
            let per_email = thread_clean::summary_chars_per_email(emails.len());
            let mut map = std::collections::HashMap::new();
            for e in emails {
                if let Ok(body) = emails::get_email_body(ctx.db, &e.id) {
                    if !body.is_empty() {
                        // What the email adds to its thread, not what it repeats.
                        let new = crate::services::thread_reader::message_new_content(ctx.db, e, &body);
                        map.insert(e.id.clone(), thread_clean::clean_email_body(&new, per_email));
                    }
                }
            }
            map
        };

        let primary = primary.map(|emails| {
            if received_only {
                emails.into_iter().filter(|e| !e.is_sent).collect()
            } else {
                emails
            }
        });

        // The classifier stores ONE intent per email and older mail carries
        // none, so a tag is a preference, not a gate: when the tagged rows do
        // not fill the page, the same search without the tag tops it up. The
        // note below keeps the two blocks apart so a count stays honest.
        let (primary, widened_by) = match (primary, tag_filter_arg) {
            (Ok(tagged), Some(_)) if (tagged.len() as i32) < offset + limit => {
                let wanted = (offset + limit) as usize;
                let seen: std::collections::HashSet<String> = tagged.iter().map(|e| e.id.clone()).collect();
                let extra = emails::search_emails_filtered(
                    ctx.db,
                    ctx.account_id,
                    query,
                    cat_filter,
                    from_filter,
                    to_filter,
                    subject_filter,
                    since_ts,
                    until_ts,
                    None,
                    (offset + limit) * 2,
                    ascending,
                    unread_only,
                    participants_arg,
                )
                .unwrap_or_default();
                let mut rows = tagged;
                let tagged_len = rows.len();
                for email in extra {
                    if rows.len() >= wanted {
                        break;
                    }
                    if received_only && email.is_sent {
                        continue;
                    }
                    if !seen.contains(&email.id) {
                        rows.push(email);
                    }
                }
                let added = rows.len() - tagged_len;
                (Ok(rows), added)
            }
            (primary, _) => (primary, 0),
        };

        match primary {
            Err(e) => Ok(ToolOutput::text(format!("Search error: {}", e))),
            Ok(matches) if !matches.is_empty() => {
                let tagged_on_page = matches.len() - widened_by;
                let emails: Vec<crate::models::Email> = matches.into_iter().skip(offset as usize).collect();
                if emails.is_empty() {
                    return Ok(ToolOutput::text(
                        "No more results — the previous page already showed every match.",
                    ));
                }
                let mut body = if include_bodies {
                    render_rows(ctx, &emails, Some(&fetch_bodies(&emails)))
                } else {
                    render_rows(ctx, &emails, None)
                };
                // A full page is only a slice: probe how many threads match
                // in total so the model can say "at least 156", not "25", and
                // knows whether a next page exists.
                let total = if emails.len() as i32 >= limit {
                    emails::search_emails_filtered(
                        ctx.db,
                        ctx.account_id,
                        query,
                        cat_filter,
                        from_filter,
                        to_filter,
                        subject_filter,
                        since_ts,
                        until_ts,
                        tag_filter_arg,
                        COUNT_PROBE_LIMIT,
                        ascending,
                        unread_only,
                        participants_arg,
                    )
                    .map(|all| all.len() as i32)
                    .unwrap_or(offset + emails.len() as i32)
                } else {
                    offset + emails.len() as i32
                };
                if let Some(note) = total_count_note(emails.len(), offset, total) {
                    body = format!("{note}\n{body}");
                }
                if let Some(note) = tag_coverage_note(tagged_on_page.saturating_sub(offset as usize), widened_by) {
                    body = format!("{note}\n{body}");
                }
                if let Some(note) = mode_note {
                    body = format!("{note}{body}");
                }
                // Remember this page so `next_page` can continue it, including
                // from a later turn (the turn seeds the cell from history).
                if let Some(state) = ctx.page {
                    state.remember(SearchPage {
                        args: args.clone(),
                        next_offset: offset + emails.len() as i32,
                        total,
                    });
                }
                Ok(ToolOutput::text_with_email_refs(body, ids(&emails)))
            }
            Ok(_) => {
                // ── Empty-result fallback ladder ───────────────────────
                let has_non_date_anchor = !query.is_empty()
                    || from_filter.is_some()
                    || to_filter.is_some()
                    || subject_filter.is_some()
                    || tag_filter_arg.is_some()
                    || unread_only;

                if (since_ts.is_some() || until_ts.is_some()) && has_non_date_anchor {
                    let retry = emails::search_emails_filtered(
                        ctx.db,
                        ctx.account_id,
                        query,
                        cat_filter,
                        from_filter,
                        to_filter,
                        subject_filter,
                        None,
                        None,
                        tag_filter_arg,
                        limit,
                        ascending,
                        unread_only,
                        participants_arg,
                    );
                    match &retry {
                        Ok(emails) if !emails.is_empty() => {
                            let mut out = String::from(
                                "(no matches in the requested date window — \
showing recent matches without since/until instead)\n",
                            );
                            out.push_str(&render_rows(ctx, emails, None));
                            return Ok(ToolOutput::text_with_email_refs(out, ids(emails)));
                        }
                        Ok(_) => {
                            return Ok(ToolOutput::text(empty_result(
                                ctx,
                                "",
                                Some("also tried without the date window"),
                            )));
                        }
                        Err(e) => {
                            return Ok(ToolOutput::text(format!("Search error on retry: {}", e)));
                        }
                    }
                }

                if let Some(merged) = or_fallback_search(
                    ctx.db,
                    ctx.account_id,
                    query,
                    cat_filter,
                    from_filter,
                    to_filter,
                    subject_filter,
                    tag_filter_arg,
                    limit,
                    unread_only,
                    participants_arg,
                ) {
                    let mut out = String::from("(no email matched all keywords — broadened to any keyword)\n");
                    out.push_str(&render_rows(ctx, &merged, None));
                    return Ok(ToolOutput::text_with_email_refs(out, ids(&merged)));
                }

                Ok(ToolOutput::text(empty_result(ctx, mode_note.unwrap_or_default(), None)))
            }
        }
    }
}

impl SearchEmailsTool {
    /// The `mode="semantic"` branch: hybrid retrieval over the account, then
    /// the sender / recipient / date / tag filters applied in memory, cut to
    /// `limit`. Rendered like the keyword path so the model sees one shape.
    #[allow(clippy::too_many_arguments)]
    async fn execute_semantic(
        &self,
        ctx: &ToolCtx<'_>,
        provider: &dyn crate::ai::provider::AIProvider,
        query: &str,
        categories: Option<&[String]>,
        post: &PostFilters<'_>,
        limit: i32,
        include_bodies: bool,
    ) -> Result<ToolOutput, ToolError> {
        let has_post_filter = post.from.is_some()
            || post.to.is_some()
            || post.since.is_some()
            || post.until.is_some()
            || !post.tags.is_empty()
            || post.received_only;
        let k = if has_post_filter {
            SEMANTIC_OVERFETCH
        } else {
            limit as usize
        };
        let cats: Vec<String> = categories.map(|c| c.to_vec()).unwrap_or_default();
        let scored =
            match crate::services::chat::retrieval::retrieve_context(ctx.db, provider, ctx.account_id, query, &cats, k)
                .await
            {
                Ok(s) => s,
                Err(e) => return Ok(ToolOutput::text(format!("Search error: {}", e))),
            };
        let mut bodies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut candidates: Vec<Email> = Vec::with_capacity(scored.len());
        for s in scored {
            if include_bodies && !s.body.is_empty() {
                bodies.insert(s.email.id.clone(), s.body);
            }
            candidates.push(s.email);
        }
        let ids: Vec<String> = candidates.iter().map(|e| e.id.clone()).collect();
        let tag_rows = if post.tags.is_empty() {
            Vec::new()
        } else {
            ctx.db.get_email_tags_batch(&ids).unwrap_or_default()
        };
        let tags_of = |id: &str| -> Vec<(String, String)> {
            tag_rows
                .iter()
                .filter(|t| t.email_id == id)
                .map(|t| (t.tag_type.clone(), t.tag_value.clone()))
                .collect()
        };
        // Every filter except the tags is a hard one; the tags only decide the
        // order, the same way the keyword path widens a short tagged page.
        let hard = PostFilters { tags: &[], ..*post };
        let kept_all = semantic_post_filter(candidates, &hard, &tags_of);
        let (tagged, rest) = partition_by_tags(kept_all, post.tags, &tags_of);
        let tagged_len = tagged.len().min(limit as usize);
        let mut kept = tagged;
        kept.extend(rest);
        kept.truncate(limit as usize);
        if kept.is_empty() {
            return Ok(ToolOutput::text(empty_result(
                ctx,
                "",
                Some("semantic search; try other words, or drop a filter"),
            )));
        }
        let per_email = thread_clean::summary_chars_per_email(kept.len());
        let mut out = String::from("(semantic search — ranked by relevance to the query, not by date)\n");
        if let Some(note) = tag_coverage_note(tagged_len, kept.len() - tagged_len) {
            out.push_str(&note);
            out.push('\n');
        }
        if include_bodies {
            let cleaned: std::collections::HashMap<String, String> = bodies
                .into_iter()
                .filter_map(|(id, b)| {
                    let email = kept.iter().find(|e| e.id == id)?;
                    // What the email adds to its thread, not what it repeats.
                    let new = crate::services::thread_reader::message_new_content(ctx.db, email, &b);
                    Some((id, thread_clean::clean_email_body(&new, per_email)))
                })
                .collect();
            out.push_str(&render_rows(ctx, &kept, Some(&cleaned)));
        } else {
            out.push_str(&render_rows(ctx, &kept, None));
        }
        let ids: Vec<String> = kept.iter().map(|e| e.id.clone()).collect();
        Ok(ToolOutput::text_with_email_refs(out, ids))
    }
}

/// Formats result rows with each thread's message count, so the model can
/// tell a lone email from the latest message of a longer exchange. A failed
/// count only drops the `messages=` field; the results still go out.
fn render_rows(
    ctx: &ToolCtx<'_>,
    rows: &[crate::models::Email],
    bodies: Option<&std::collections::HashMap<String, String>>,
) -> String {
    let threads: Vec<(&str, &str)> = rows
        .iter()
        .map(|e| (e.account_id.as_str(), e.thread_id.as_str()))
        .collect();
    let sizes = emails::thread_sizes(ctx.db, &threads).unwrap_or_else(|e| {
        crate::services::logger::log("warn", "chat", format!("search_emails: thread sizes unavailable ({e})"));
        std::collections::HashMap::new()
    });
    match bodies {
        Some(bodies) => format_search_emails_output_with_bodies(rows, bodies, &sizes),
        None => format_search_emails_output(rows, &sizes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_note_when_the_page_is_the_whole_result() {
        // Everything matched fits on this page — the old note claimed
        // "5 of 5 … to see the rest", which sent the model looking for more.
        assert_eq!(total_count_note(10, 0, 10), None);
        assert_eq!(total_count_note(25, 0, 25), None);
    }

    #[test]
    fn the_note_states_the_range_the_total_and_the_next_page() {
        assert_eq!(
            total_count_note(25, 0, 156).as_deref(),
            Some(
                "(showing 1-25 of 156 matching threads — tell the user there are 156 in total; call next_page for the next ones, or narrow with since/until, from, or a keyword)"
            )
        );
    }

    #[test]
    fn a_later_page_counts_from_its_offset() {
        assert_eq!(
            total_count_note(25, 25, 156).as_deref(),
            Some(
                "(showing 26-50 of 156 matching threads — tell the user there are 156 in total; call next_page for the next ones, or narrow with since/until, from, or a keyword)"
            )
        );
    }

    #[test]
    fn the_last_page_says_so_instead_of_offering_more() {
        assert_eq!(
            total_count_note(4, 50, 54).as_deref(),
            Some("(showing 51-54 of 54 matching threads — this is the last page)")
        );
    }

    #[test]
    fn no_coverage_note_when_the_tag_answered_on_its_own() {
        // The page filled with tagged rows: nothing was widened, nothing to say.
        assert_eq!(tag_coverage_note(25, 0), None);
    }

    #[test]
    fn the_coverage_note_separates_the_two_blocks() {
        // The classifier stores ONE intent per email, so a tag-filtered list is
        // a preference, not the whole truth. The model needs both numbers: the
        // tagged count answers "how many", the rest is context.
        assert_eq!(
            tag_coverage_note(3, 12).as_deref(),
            Some(
                "(3 emails carry the intent/topic asked for; the 12 rows after them match every \
other filter but are tagged differently or not classified — each email stores only ONE intent, \
so count only the first 3 when asked how many)"
            )
        );
    }

    #[test]
    fn the_coverage_note_says_when_nothing_carries_the_tag() {
        assert_eq!(
            tag_coverage_note(0, 5).as_deref(),
            Some(
                "(no email carries the intent/topic asked for; the 5 rows below match every other \
filter and are shown instead — each email stores only ONE intent, so the tag may simply be a \
different one)"
            )
        );
    }

    #[test]
    fn total_note_marks_a_capped_probe() {
        // The probe itself stops at COUNT_PROBE_LIMIT; past it the total is a floor.
        let note = total_count_note(25, 0, COUNT_PROBE_LIMIT).unwrap();
        assert!(note.contains(&format!("of {COUNT_PROBE_LIMIT}+ matching")), "{note}");
    }

    use crate::db::Database;
    use std::sync::Arc;

    #[tokio::test]
    async fn empty_args_error_includes_an_example_call() {
        // A degenerate `search_emails({})` call (name-only tool call from a
        // flaky model) must come back with an error the model can imitate on
        // its next round — a concrete example call, not just the list of
        // accepted parameter names.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let categories: Vec<String> = Vec::new();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acct",
            categories: &categories,
            page: None,
        };
        let out = SearchEmailsTool.execute(&ctx, json!({})).await.expect("tool ran");
        assert!(
            out.text.starts_with("Error:"),
            "kept as a model-facing error: {}",
            out.text
        );
        assert!(
            out.text.contains("Example:"),
            "error must include an example call to imitate: {}",
            out.text
        );
    }

    #[test]
    fn intent_and_topic_schema_lists_the_configured_tags_with_definitions() {
        let db = Database::new_for_testing().expect("test db");
        db.set_preference("classify_intents", r#"["request","escalation"]"#)
            .expect("pref");
        let schema = SearchEmailsTool.parameters_schema_for(&db);
        let intent = &schema["properties"]["intent"];
        let values: Vec<&str> = intent["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(values, ["request", "escalation"], "enum follows the user's tag list");
        let desc = intent["description"].as_str().unwrap();
        assert!(desc.contains("request ("), "definition inline: {desc}");
        assert!(desc.contains("escalation"), "custom tag listed: {desc}");
        assert!(!desc.contains("prospect"), "no per-concept rules in the schema: {desc}");
        let topic = &schema["properties"]["topic"];
        assert!(topic["enum"].as_array().unwrap().len() > 5, "default topics when unset");
        assert!(topic["description"].as_str().unwrap().contains("billing ("));
    }

    #[test]
    fn static_schema_uses_the_default_glossary() {
        let schema = SearchEmailsTool.parameters_schema();
        let desc = schema["properties"]["intent"]["description"].as_str().unwrap();
        assert!(desc.contains("introduction ("), "{desc}");
        assert!(desc.contains("complaint ("), "{desc}");
        assert!(schema["properties"]["mode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("semantic")));
    }

    fn email(id: &str, sender: &str, to: &str, ts: i64) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acct".to_string(),
            thread_id: format!("t-{id}"),
            message_id: None,
            references: None,
            subject: String::new(),
            sender: format!("{sender} <{sender}@example.com>"),
            sender_email: format!("{sender}@example.com"),
            recipients: vec![to.to_string()],
            cc: vec![],
            body: String::new(),
            snippet: String::new(),
            timestamp: ts,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "inbox".to_string(),
            is_sent: false,
            headers: None,
        }
    }

    #[test]
    fn semantic_post_filter_keeps_only_unread_when_asked() {
        let mut read = email("r", "alice", "me@x.com", 100);
        read.is_read = true;
        let emails = vec![read, email("u", "alice", "me@x.com", 200)];
        let kept: Vec<String> = semantic_post_filter(
            emails,
            &PostFilters {
                unread_only: true,
                ..Default::default()
            },
            &|_: &str| Vec::new(),
        )
        .into_iter()
        .map(|e| e.id)
        .collect();
        assert_eq!(kept, ["u"]);
    }

    #[test]
    fn result_rows_mark_unread_mail_and_only_unread_mail() {
        let mut read = email("r", "alice", "me@x.com", 100);
        read.is_read = true;
        let out = crate::services::chat::format_search_emails_output(
            &[read, email("u", "bob", "me@x.com", 200)],
            &std::collections::HashMap::new(),
        );
        let row = |id: &str| {
            out.lines()
                .find(|l| l.contains(&format!("id={id} ")))
                .unwrap_or_default()
                .to_string()
        };
        assert!(row("u").contains(" unread"), "{out}");
        assert!(!row("r").contains("unread"), "{out}");
    }

    #[test]
    fn the_semantic_path_ranks_by_tag_instead_of_dropping_rows() {
        // Same contract as the keyword path: the tag decides the order, never
        // what the model gets to see.
        let emails = vec![
            email("a", "alice", "me@x.com", 100),
            email("b", "bob", "me@x.com", 200),
            email("c", "carol", "me@x.com", 300),
        ];
        let tags = |id: &str| -> Vec<(String, String)> {
            match id {
                "b" => vec![("intent".into(), "request".into())],
                _ => vec![],
            }
        };
        let want = vec![TagQuery::typed("intent", "request")];

        let (tagged, rest) = partition_by_tags(emails, &want, &tags);

        assert_eq!(tagged.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(rest.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "c"]);
    }

    #[test]
    fn partitioning_without_tags_leaves_everything_in_order() {
        let emails = vec![email("a", "alice", "me@x.com", 100), email("b", "bob", "me@x.com", 200)];
        let tags = |_: &str| -> Vec<(String, String)> { Vec::new() };

        let (tagged, rest) = partition_by_tags(emails, &[], &tags);

        assert!(tagged.is_empty(), "no tag asked for → nothing is preferred");
        assert_eq!(rest.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn semantic_post_filter_applies_sender_recipient_dates_and_tags() {
        let emails = vec![
            email("a", "alice", "me@x.com", 100),
            email("b", "bob", "me@x.com", 200),
            email("c", "alice", "other@x.com", 300),
        ];
        let tags = |id: &str| -> Vec<(String, String)> {
            match id {
                "a" => vec![("intent".into(), "request".into())],
                "c" => vec![("intent".into(), "request".into()), ("topic".into(), "sales".into())],
                // A company that shares a tag's name must not answer for it.
                "b" => vec![("company".into(), "request".into())],
                _ => vec![],
            }
        };
        let keep = |f: PostFilters| -> Vec<String> {
            semantic_post_filter(emails.clone(), &f, &tags)
                .into_iter()
                .map(|e| e.id)
                .collect()
        };
        assert_eq!(keep(PostFilters::default()), ["a", "b", "c"]);
        assert_eq!(
            keep(PostFilters {
                from: Some("alice"),
                ..Default::default()
            }),
            ["a", "c"]
        );
        assert_eq!(
            keep(PostFilters {
                to: Some("me@x.com"),
                ..Default::default()
            }),
            ["a", "b"]
        );
        assert_eq!(
            keep(PostFilters {
                since: Some(150),
                until: Some(300),
                ..Default::default()
            }),
            ["b"]
        );
        let both = vec![TagQuery::typed("intent", "request"), TagQuery::typed("topic", "sales")];
        assert_eq!(
            keep(PostFilters {
                tags: &both,
                ..Default::default()
            }),
            ["c"],
            "intent AND topic must both hold"
        );
        let intent_only = vec![TagQuery::typed("intent", "request")];
        assert_eq!(
            keep(PostFilters {
                tags: &intent_only,
                ..Default::default()
            }),
            ["a", "c"],
            "the company tagged `request` is not an intent"
        );
        let any_type = vec![TagQuery::any_type("request")];
        assert_eq!(
            keep(PostFilters {
                tags: &any_type,
                ..Default::default()
            }),
            ["a", "b", "c"],
            "an untyped tag: filter still matches any type"
        );
        assert_eq!(
            keep(PostFilters {
                received_only: true,
                ..Default::default()
            }),
            ["a", "b", "c"]
        );
    }

    #[tokio::test]
    async fn semantic_mode_without_a_provider_falls_back_to_keyword_search() {
        // No AI provider is configured on a bare test DB. The tool must not
        // error out: it runs the keyword search and says so.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let categories: Vec<String> = Vec::new();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acct",
            categories: &categories,
            page: None,
        };
        let out = SearchEmailsTool
            .execute(&ctx, json!({"query": "presupuesto proveedor", "mode": "semantic"}))
            .await
            .expect("tool ran");
        assert!(!out.text.starts_with("Search error"), "{}", out.text);
        assert!(out.text.contains("No matching emails found"), "{}", out.text);
    }

    #[tokio::test]
    async fn zero_results_name_the_account_the_search_was_scoped_to() {
        // "No matching emails found." reads as "you have no such email" when it
        // really means "not in the ONE account this chat can search" — the
        // ambiguity DECISIONS.md (2026-08-14) calls out. Naming the mailbox is
        // what lets the model report a scoped absence instead of an absolute
        // one.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acct', 'gmail', 'me@acme.com', 'Test', 0)",
                [],
            )
            .unwrap();
        let categories: Vec<String> = Vec::new();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acct",
            categories: &categories,
            page: None,
        };
        let out = SearchEmailsTool
            .execute(&ctx, json!({"from": "nobody@example.com", "limit": 5}))
            .await
            .expect("tool ran");
        assert!(
            out.text.contains("in me@acme.com"),
            "empty result must name the mailbox it searched: {}",
            out.text
        );
    }

    /// "cuándo es la demo del sprint" searched the mailbox, found nothing and
    /// asked the user for context: the demo was on the calendar all along.
    #[tokio::test]
    async fn zero_results_point_to_the_calendar_when_the_account_has_one() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acct', 'gmail', 'me@acme.com', 'Test', 0),
                        ('imap', 'imap', 'me@imap.test', 'Test', 0)",
                [],
            )
            .unwrap();
        let categories: Vec<String> = Vec::new();
        for (account, expect_hint) in [("acct", true), ("imap", false)] {
            let ctx = ToolCtx {
                db: &db,
                account_id: account,
                categories: &categories,
                page: None,
            };
            let out = SearchEmailsTool
                .execute(&ctx, json!({"query": "demo", "limit": 5}))
                .await
                .expect("tool ran");
            assert_eq!(
                out.text.contains("list_calendar_events"),
                expect_hint,
                "{account}: {}",
                out.text
            );
        }
    }

    #[tokio::test]
    async fn zero_results_keep_the_bare_contract_when_the_account_is_unknown() {
        // A lookup failure must degrade to the original wording rather than
        // leak an id or an empty "in ." — same graceful-degradation rule the
        // identity line follows.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let categories: Vec<String> = Vec::new();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acct",
            categories: &categories,
            page: None,
        };
        let out = SearchEmailsTool
            .execute(&ctx, json!({"from": "nobody@example.com", "limit": 5}))
            .await
            .expect("tool ran");
        assert_eq!(out.text, "No matching emails found.");
    }
}
