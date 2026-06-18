use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::chat::{
    format_search_emails_output, format_search_emails_output_with_bodies, or_fallback_search, parse_iso_date_secs,
};
use crate::services::{emails, thread_clean};

pub struct SearchEmailsTool;

#[async_trait]
impl Tool for SearchEmailsTool {
    fn name(&self) -> &'static str {
        "search_emails"
    }

    fn description(&self) -> &'static str {
        "Search the user's emails. Returns a list of matching emails with id, thread_id, subject, sender, date, category and a short snippet — THE SNIPPET DOES NOT INCLUDE ATTACHMENT FILENAMES. Results are grouped by Gmail category in priority order: Primary first (real people / direct mail), then Updates (receipts, shipping, automated notifications), then Other (social, forums, promotions). Keep that ordering when you summarise the results to the user. Combine filters to narrow results. Use `from` when the user asks about mail RECEIVED from someone ('de alice', 'from bob'); use `to` when they ask about mail SENT to someone ('enviada a emailops', 'para maria'). When the user keeps narrowing keywords (e.g. 'factura de emailops'), keep BOTH `query='factura'` AND `from/to='...emailops...'` — never drop the keyword. A date-bounded lookup is much more precise than a bare keyword query. At least one of query / from / to / subject / since / until must be non-empty. REQUIRED CHAIN: if the user asked about invoices / facturas / recibos / PDFs / attached documents, you MUST call `get_attachments(email_id)` on the top matching email before writing your final answer — the snippet alone is not enough to name the attached file."
    }

    fn prompt_summary(&self) -> &'static str {
        "search the inbox; ≥1 filter required, results newest-first. `from` = mail RECEIVED (\"from alice\"), `to` = mail SENT (\"sent to emailops\") — do not conflate."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Full-text keywords to match in subject/body. Leave empty when filtering purely by sender or date." },
                "from": { "type": "string", "description": "Filter by sender. Matches email address prefix (e.g. 'alice@emailops.com') or display name substring (e.g. 'Alice Smith')." },
                "to": { "type": "string", "description": "Filter by recipient — use this when the user says 'enviada a X' / 'sent to X' / 'para X'. Matches the To/CC field (substring, e.g. 'billing@emailops.com' or 'emailops.com')." },
                "subject": { "type": "string", "description": "Filter by subject keywords (FTS5 match on subject column)." },
                "since": { "type": "string", "description": "Only return emails on or after this date. ISO-8601 date 'YYYY-MM-DD' (UTC). Example: '2026-04-17' for today." },
                "until": { "type": "string", "description": "Only return emails strictly before this date. ISO-8601 date 'YYYY-MM-DD' (UTC). Example: use until='2026-04-18' together with since='2026-04-17' to get today's emails only." },
                "limit": { "type": "integer", "description": "Max number of results to return. Default 20, max 25. Use 25 for 'all X' / 'todas' queries, 5 for 'latest X' / 'última'." },
                "order": { "type": "string", "enum": ["newest", "oldest"], "description": "Sort direction. Default 'newest' (most recent first). Use 'oldest' with limit=1 for 'first / earliest' queries ('first email I sent to X', 'primer correo', 'el más antiguo')." }
            },
            "required": []
        })
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
        let include_bodies = args.get("include_bodies").and_then(|v| v.as_bool()).unwrap_or(false);
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
            && subject_filter.is_none()
            && since_str.is_none()
            && until_str.is_none()
        {
            return Ok(ToolOutput::text(
                "Error: search_emails requires at least one of query / from / to / subject / since / until.",
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

        // An explicit sender / recipient / subject lookup must not be silently
        // narrowed by the chat turn's category scope (default ["primary"]). That
        // scope is meant for broad keyword/RAG retrieval; when the user names a
        // target we return the newest matching mail regardless of Gmail category
        // — a newsletter landing in `updates` must still surface for `from:X`.
        let has_explicit_target = from_filter.is_some() || to_filter.is_some() || subject_filter.is_some();
        let cat_filter: Option<&[String]> = if has_explicit_target || ctx.categories.is_empty() {
            None
        } else {
            Some(ctx.categories)
        };

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
            None,
            limit,
            ascending,
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
                        map.insert(e.id.clone(), thread_clean::clean_email_body(&body, per_email));
                    }
                }
            }
            map
        };

        match primary {
            Err(e) => Ok(ToolOutput::text(format!("Search error: {}", e))),
            Ok(emails) if !emails.is_empty() => {
                let body = if include_bodies {
                    format_search_emails_output_with_bodies(&emails, &fetch_bodies(&emails))
                } else {
                    format_search_emails_output(&emails)
                };
                Ok(ToolOutput::text_with_email_refs(body, ids(&emails)))
            }
            Ok(_) => {
                // ── Empty-result fallback ladder ───────────────────────
                let has_non_date_anchor =
                    !query.is_empty() || from_filter.is_some() || to_filter.is_some() || subject_filter.is_some();

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
                        None,
                        limit,
                        ascending,
                    );
                    match &retry {
                        Ok(emails) if !emails.is_empty() => {
                            let mut out = String::from(
                                "(no matches in the requested date window — \
showing recent matches without since/until instead)\n",
                            );
                            out.push_str(&format_search_emails_output(emails));
                            return Ok(ToolOutput::text_with_email_refs(out, ids(emails)));
                        }
                        Ok(_) => {
                            return Ok(ToolOutput::text(
                                "No matching emails found (also tried without the date window).",
                            ));
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
                    limit,
                ) {
                    let mut out = String::from("(no email matched all keywords — broadened to any keyword)\n");
                    out.push_str(&format_search_emails_output(&merged));
                    return Ok(ToolOutput::text_with_email_refs(out, ids(&merged)));
                }

                Ok(ToolOutput::text("No matching emails found."))
            }
        }
    }
}
