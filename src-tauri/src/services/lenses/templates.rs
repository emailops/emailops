//! Built-in Lens templates (PRD §10).
//!
//! Manifest is consumed by `list_lens_templates` and copied into the `lenses`
//! table when the user creates from a template. Once copied the user's Lens is
//! independent of the template, so changing prompts here does not affect
//! already-created Lenses.

use serde::Serialize;

use crate::models::lens::{DateRange, Direction, LensColumn, LensColumnType, LensSchema, LensScope};

/// Manifest entry surfaced by `list_lens_templates`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensTemplate {
    pub key: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub default_scope: LensScope,
    pub schema: LensSchema,
    pub prompt: String,
}

// ── Small builder helpers ────────────────────────────────────────────────────

fn col(key: &str, label: &str, ty: LensColumnType, desc: &str, required: bool) -> LensColumn {
    LensColumn {
        key: key.into(),
        label: label.into(),
        column_type: ty,
        description: desc.into(),
        enum_values: None,
        required,
        is_unique_key: false,
    }
}

fn enum_col(key: &str, label: &str, desc: &str, required: bool, values: &[&str]) -> LensColumn {
    LensColumn {
        key: key.into(),
        label: label.into(),
        column_type: LensColumnType::Enum,
        description: desc.into(),
        enum_values: Some(values.iter().map(|s| (*s).into()).collect()),
        required,
        is_unique_key: false,
    }
}

fn last_days(n: i64) -> Option<DateRange> {
    Some(DateRange {
        last_days: Some(n),
        from: None,
        to: None,
    })
}

// ── Templates ────────────────────────────────────────────────────────────────

fn tpl_awaiting_reply() -> LensTemplate {
    LensTemplate {
        key: "awaiting_reply".into(),
        name: "Awaiting reply".into(),
        icon: "⏳".into(),
        description: "Outbound emails where you asked something and got no answer.".into(),
        default_scope: LensScope {
            mailboxes: Some(vec!["sent".into()]),
            direction: Some(Direction::Outbound),
            date_range: last_days(60),
            query: Some("?".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("recipient", "Recipient", LensColumnType::Email, "Primary recipient address.", true),
                col("subject", "Subject", LensColumnType::String, "Email subject as sent.", true),
                col("question_summary", "Question", LensColumnType::Text, "Short summary of what you asked.", true),
                col("days_silent", "Days silent", LensColumnType::Number, "Approximate days since the email was sent without a reply.", false),
                enum_col("priority_guess", "Priority", "Best guess of urgency based on tone and ask.", true, &["low", "medium", "high"]),
            ],
        },
        prompt: "You are reviewing an OUTBOUND email the user sent that asked a question and (per the filter) has not been answered.\n\nExtract:\n- recipient: the primary recipient address\n- subject: as written\n- question_summary: one short sentence summarising what was asked\n- days_silent: approximate days since the email was sent; leave null if unclear\n- priority_guess: low / medium / high — guess from tone, deadline language, and the seniority of the recipient\n\nReturn null for any field you cannot extract confidently rather than guessing.".into(),
    }
}

fn tpl_promises_made() -> LensTemplate {
    LensTemplate {
        key: "promises_made".into(),
        name: "Promises made".into(),
        icon: "🤝".into(),
        description: "Things you committed to do in outbound emails.".into(),
        default_scope: LensScope {
            mailboxes: Some(vec!["sent".into()]),
            direction: Some(Direction::Outbound),
            date_range: last_days(90),
            query: Some("\"I will\" OR \"I'll\" OR \"let me\" OR \"by Friday\" OR \"by Monday\" OR \"next week\"".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("recipient", "Recipient", LensColumnType::Email, "Person you made the commitment to.", true),
                col("promise", "Promise", LensColumnType::Text, "What you committed to deliver.", true),
                col("by_when", "Due", LensColumnType::Date, "Date by which you said you would deliver. Use ISO 8601 YYYY-MM-DD.", false),
                enum_col("confidence", "Confidence", "How confident you are this is a real commitment vs polite filler.", true, &["low", "medium", "high"]),
            ],
        },
        prompt: "You are reading an OUTBOUND email the user sent. Identify any concrete commitment the user made to the recipient.\n\nA commitment is a specific deliverable or follow-up the user said they would do. Vague pleasantries (\"let's catch up\") are NOT commitments unless tied to an action.\n\nExtract:\n- recipient: who the promise is to\n- promise: one-sentence description of what was promised\n- by_when: ISO date if a deadline is stated or strongly implied (e.g. \"by Friday\" — resolve relative to the email date); else null\n- confidence: low / medium / high — penalise vague language\n\nIf the email contains no commitment at all, leave promise empty and confidence = \"low\".".into(),
    }
}

fn tpl_invoices_received() -> LensTemplate {
    LensTemplate {
        key: "invoices_received".into(),
        name: "Invoices received".into(),
        icon: "🧾".into(),
        description: "Bills and invoices addressed to you.".into(),
        default_scope: LensScope {
            direction: Some(Direction::Inbound),
            date_range: last_days(365),
            query: Some("invoice OR receipt OR \"amount due\" OR \"payment due\"".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("vendor", "Vendor", LensColumnType::String, "Company or person issuing the invoice.", true),
                col("amount", "Amount", LensColumnType::Currency, "Total amount due, including currency.", true),
                // Invoice # is the natural unique key — receipts and reminders
                // for the same invoice should collapse to one row.
                LensColumn {
                    is_unique_key: true,
                    ..col("invoice_number", "Invoice #", LensColumnType::String, "Vendor's invoice number / reference.", false)
                },
                col("due_date", "Due", LensColumnType::Date, "Date by which the invoice must be paid. ISO 8601.", false),
                enum_col("status", "Status", "Best guess from the email content.", true, &["unpaid", "paid", "overdue", "unknown"]),
            ],
        },
        prompt: "You are reading an INBOUND email that may contain an invoice or bill.\n\nExtract:\n- vendor: the company or person you owe\n- amount: { amount, currency } — currency as 3-letter ISO 4217 (USD, EUR, GBP, ...). If multiple amounts appear, prefer the TOTAL DUE, not subtotals or taxes alone.\n- invoice_number: the vendor's reference if present\n- due_date: ISO date; leave null if not stated\n- status: unpaid (default for new invoices), paid (if the email is a paid receipt), overdue (if it explicitly says so), unknown (if you cannot tell)\n\nPrefer null over guessing.".into(),
    }
}

fn tpl_invoices_sent() -> LensTemplate {
    LensTemplate {
        key: "invoices_sent".into(),
        name: "Invoices sent".into(),
        icon: "💸".into(),
        description: "Invoices you have sent to clients.".into(),
        default_scope: LensScope {
            mailboxes: Some(vec!["sent".into()]),
            direction: Some(Direction::Outbound),
            date_range: last_days(365),
            query: Some("invoice OR \"please find attached\" OR \"amount due\"".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("client", "Client", LensColumnType::String, "Client billed.", true),
                col("amount", "Amount", LensColumnType::Currency, "Total amount billed.", true),
                LensColumn {
                    is_unique_key: true,
                    ..col("invoice_number", "Invoice #", LensColumnType::String, "Your invoice number.", false)
                },
                col("sent_date", "Sent", LensColumnType::Date, "Date you sent the invoice. ISO 8601.", false),
                col("due_date", "Due", LensColumnType::Date, "Payment due date if stated.", false),
                col("paid", "Paid", LensColumnType::Boolean, "True/false if the email reveals payment state; otherwise leave null.", false),
                col("days_overdue", "Days overdue", LensColumnType::Number, "Days past the due date if known and unpaid.", false),
            ],
        },
        prompt: "You are reading an OUTBOUND email in which the user appears to have sent an invoice to a client.\n\nExtract:\n- client: who you billed (look for greeting/recipient or body)\n- amount: { amount, currency } — use ISO 4217 codes\n- invoice_number: your reference if visible\n- sent_date: ISO date the invoice covers / was sent; leave null if not stated\n- due_date: ISO date if mentioned\n- paid: true if email confirms payment received, false if it explicitly says unpaid/overdue, otherwise null\n- days_overdue: integer days past due if calculable from the email; otherwise null\n\nReturn null for fields you cannot extract confidently.".into(),
    }
}

fn tpl_travel() -> LensTemplate {
    LensTemplate {
        key: "travel".into(),
        name: "Travel".into(),
        icon: "✈️".into(),
        description: "Flights, hotels, cars, and other travel confirmations.".into(),
        default_scope: LensScope {
            direction: Some(Direction::Inbound),
            date_range: last_days(365),
            query: Some("confirmation OR itinerary OR booking OR reservation".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                enum_col("travel_type", "Type", "Category of travel reservation.", true, &["flight", "hotel", "car", "train", "other"]),
                col("provider", "Provider", LensColumnType::String, "Airline, hotel chain, rental company.", true),
                col("start_date", "Start", LensColumnType::Date, "Departure / check-in date.", true),
                col("end_date", "End", LensColumnType::Date, "Return / check-out date if applicable.", false),
                col("confirmation_code", "Confirmation #", LensColumnType::String, "Booking reference or PNR.", false),
                col("destination", "Destination", LensColumnType::String, "City or airport code; leave null if unclear.", false),
            ],
        },
        prompt: "You are reading an INBOUND travel confirmation.\n\nExtract:\n- travel_type: flight / hotel / car / train / other\n- provider: airline name, hotel chain, rental company, etc.\n- start_date: ISO date of departure or check-in\n- end_date: ISO date of return or check-out (null for one-way flights)\n- confirmation_code: booking reference, PNR, etc.\n- destination: city, airport code, or hotel city — null if ambiguous\n\nIf the email is NOT a travel confirmation (e.g. a marketing email), return null for all fields except travel_type=\"other\".".into(),
    }
}

fn tpl_subscriptions() -> LensTemplate {
    LensTemplate {
        key: "subscriptions".into(),
        name: "Subscriptions".into(),
        icon: "🔁".into(),
        description: "Recurring SaaS bills and renewals.".into(),
        default_scope: LensScope {
            direction: Some(Direction::Inbound),
            categories: Some(vec!["Updates".into(), "Promotions".into()]),
            date_range: last_days(180),
            query: Some("subscription OR renewal OR \"your plan\" OR billing".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("service", "Service", LensColumnType::String, "Name of the subscription service.", true),
                col("amount", "Amount", LensColumnType::Currency, "Recurring charge amount and currency.", false),
                enum_col("cadence", "Cadence", "Billing cadence.", true, &["monthly", "yearly", "other", "unknown"]),
                col("next_renewal", "Next renewal", LensColumnType::Date, "Date of next billing if mentioned.", false),
                col("cancel_url", "Cancel link", LensColumnType::Url, "URL to manage or cancel the subscription.", false),
            ],
        },
        prompt: "You are reading an INBOUND email that mentions a subscription, renewal, or recurring bill.\n\nExtract:\n- service: name of the product/service\n- amount: { amount, currency } using ISO 4217 codes; null if not stated\n- cadence: monthly / yearly / other / unknown\n- next_renewal: ISO date of the next charge if mentioned\n- cancel_url: a URL pointing to a manage/cancel page if present\n\nIf the email is a one-off receipt rather than a recurring subscription, set cadence=\"other\" and leave next_renewal null.".into(),
    }
}

fn tpl_wise_transfers() -> LensTemplate {
    LensTemplate {
        key: "wise_transfers".into(),
        name: "Wise transfers".into(),
        icon: "💱".into(),
        description: "Cross-border payments via Wise.".into(),
        default_scope: LensScope {
            direction: Some(Direction::Inbound),
            sender_domains: Some(vec!["wise.com".into()]),
            date_range: last_days(365),
            query: Some("\"sent you\" OR received OR transfer".into()),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("sender_name", "From", LensColumnType::String, "The person/company who sent the money (from the body, NOT the From: header).", true),
                col("amount", "Amount", LensColumnType::Number, "Numeric amount received.", true),
                col("currency", "Currency", LensColumnType::String, "3-letter ISO 4217 currency code, e.g. EUR, USD, GBP.", true),
                col("received_date", "Received", LensColumnType::Date, "Date the money arrived (often differs from email timestamp).", false),
                col("reference", "Reference", LensColumnType::Text, "Sender's note or reference if present.", false),
                col("transfer_id", "Transfer ID", LensColumnType::String, "Wise transfer/reference number if visible.", false),
            ],
        },
        prompt: "Wise notification emails follow the pattern 'X sent you Y CURRENCY'. The From: address is ALWAYS Wise — you must read the body to extract the sender's identity.\n\nExtract:\n- sender_name: the person or company who sent the money, from the body text (e.g. 'Jane Doe sent you €1,200.00' → 'Jane Doe')\n- amount: the numeric amount as a number (no symbols)\n- currency: 3-letter ISO 4217 code (EUR, USD, GBP, ...)\n- received_date: ISO date the transfer arrived (if shown); else null\n- reference: free-text note from the sender if shown\n- transfer_id: Wise reference number if shown\n\nReturn null for any field not present in the body rather than guessing.".into(),
    }
}

fn tpl_newsletter_digest() -> LensTemplate {
    LensTemplate {
        key: "newsletter_digest".into(),
        name: "Newsletter digest".into(),
        icon: "📰".into(),
        description: "One-line summaries of recent newsletters.".into(),
        default_scope: LensScope {
            direction: Some(Direction::Inbound),
            categories: Some(vec!["Updates".into()]),
            date_range: last_days(14),
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                col("newsletter", "Newsletter", LensColumnType::String, "Name of the newsletter / sender.", true),
                col("top_topic", "Top topic", LensColumnType::Text, "Headline topic or main story.", true),
                col("summary", "Summary", LensColumnType::Text, "One-line skim summary.", true),
                col("worth_clicking", "Worth opening?", LensColumnType::Boolean, "True if this looks substantive for the user, false if filler.", true),
            ],
        },
        prompt: "You are reading an INBOUND newsletter email.\n\nExtract:\n- newsletter: name of the newsletter or sender brand\n- top_topic: the headline topic / main story in 3–6 words\n- summary: one-line skim summary (under 25 words)\n- worth_clicking: true if the content looks substantive (deep analysis, breaking news, specific actionable info) vs filler (link roundups, ads, promotional content)\n\nBe honest about worth_clicking — most newsletters are filler.".into(),
    }
}

fn tpl_contact_form_leads() -> LensTemplate {
    let mut contact_email = col(
        "contact_email",
        "Email",
        LensColumnType::Email,
        "Email address of the person who filled in the form, taken from the body (NOT the From: header).",
        true,
    );
    contact_email.is_unique_key = true;
    LensTemplate {
        key: "contact_form_leads".into(),
        name: "Contact form leads".into(),
        icon: "📨".into(),
        description: "People who wrote in through your website's contact form.".into(),
        default_scope: LensScope {
            direction: Some(Direction::Inbound),
            date_range: last_days(365),
            // Form mailers vary the subject; the plugin footer in the body is
            // the stable marker. Implicit AND binds tighter than OR in FTS5.
            query: Some("contact form OR formulario de contacto OR formulaire de contact OR Kontaktformular".into()),
            query_search_body: true,
            ..Default::default()
        },
        schema: LensSchema {
            columns: vec![
                contact_email,
                col("contact_name", "Name", LensColumnType::String, "Name of the person who filled in the form.", false),
                col("company", "Company", LensColumnType::String, "Their company, stated or implied by a non-free-mail email domain.", false),
                col("phone", "Phone", LensColumnType::String, "Their phone number if they gave one.", false),
                enum_col(
                    "request_type",
                    "Type",
                    "quote_request = they want to BUY from you (a quote, budget, price or proposal); question = any other enquiry from a prospect or client; sales_pitch = they want to SELL something to you; other = anything else.",
                    true,
                    &["quote_request", "question", "sales_pitch", "other"],
                ),
                col("summary", "Summary", LensColumnType::Text, "One sentence on what they want.", true),
            ],
        },
        prompt: "You are reading a notification sent by a website contact form (Contact Form 7, Webflow, Formspree and similar). The message starts with header lines (From:, Date:, Subject:) and a blank line; that first From: line is the website or its mailer — NEVER the person who wrote in. Everything after the first blank line is the BODY, even when it repeats its own From:/Subject: lines. The submitter's details are in the BODY, as labelled fields such as 'De:', 'From:', 'Nombre:', 'Name:', 'Email:', 'Correo electrónico:', 'Teléfono:', 'Phone:', or a single line like 'De: Jane Doe jane@example.com'. A 'From:' or 'De:' field inside the body IS the submitter. The footer (e.g. 'This e-mail was sent from a contact form on …' / 'Este mensaje se ha enviado desde un formulario de contacto en …') names the website, not the submitter.\n\nExtract:\n- contact_email: the submitter's email address copied character for character from the body. Do not shorten, correct or invent it; never use the header From: address or the website's own address\n- contact_name: the submitter's name as written in the body\n- company: the company they state; otherwise the organisation implied by their email domain (e.g. 'acme.example' → 'Acme'); null for free mail providers (gmail, hotmail, outlook, yahoo, icloud, …)\n- phone: their phone number if given\n- request_type: quote_request when they want to BUY from you — they ask for a quote, presupuesto, budget, price or proposal; question for any other enquiry from a prospect or client; sales_pitch when THEY want to SELL to you (SEO, advertising, press articles, software, outsourcing); other otherwise\n- summary: one sentence, in the language of the message, on what they want\n\nFor a field the body does not give, return JSON null — never the text \"null\". If the email is not a contact-form notification, return null for every field".into(),
    }
}

/// All built-in templates.
pub fn manifest() -> Vec<LensTemplate> {
    vec![
        tpl_awaiting_reply(),
        tpl_promises_made(),
        tpl_invoices_received(),
        tpl_invoices_sent(),
        tpl_travel(),
        tpl_subscriptions(),
        tpl_wise_transfers(),
        tpl_newsletter_digest(),
        tpl_contact_form_leads(),
    ]
}

/// Look up a template by `key`.
pub fn get(key: &str) -> Option<LensTemplate> {
    manifest().into_iter().find(|t| t.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_expected_keys() {
        let keys: Vec<String> = manifest().into_iter().map(|t| t.key).collect();
        for expected in [
            "awaiting_reply",
            "promises_made",
            "invoices_received",
            "invoices_sent",
            "travel",
            "subscriptions",
            "wise_transfers",
            "newsletter_digest",
            "contact_form_leads",
        ] {
            assert!(keys.iter().any(|k| k == expected), "missing template {expected}");
        }
        assert_eq!(keys.len(), 9);
    }

    #[test]
    fn enum_columns_have_values() {
        for tpl in manifest() {
            for col in tpl.schema.columns {
                if col.column_type == LensColumnType::Enum {
                    let values = col
                        .enum_values
                        .as_ref()
                        .unwrap_or_else(|| panic!("{}::{} missing enum_values", tpl.key, col.key));
                    assert!(!values.is_empty(), "{}::{} has empty enum_values", tpl.key, col.key);
                }
            }
        }
    }

    #[test]
    fn wise_template_targets_wise_domain() {
        let tpl = get("wise_transfers").expect("wise template exists");
        let domains = tpl
            .default_scope
            .sender_domains
            .as_ref()
            .expect("wise scope has sender_domains");
        assert!(domains.iter().any(|d| d == "wise.com"));
    }

    #[test]
    fn contact_form_template_keys_rows_on_the_submitter_address() {
        // One row per person who wrote in: the address comes from the body,
        // and repeat submissions collapse onto it.
        let tpl = get("contact_form_leads").expect("contact form template exists");
        let email = tpl
            .schema
            .columns
            .iter()
            .find(|c| c.key == "contact_email")
            .expect("contact_email column");
        assert_eq!(email.column_type, LensColumnType::Email);
        assert!(email.required);
        assert!(email.is_unique_key);
        assert_eq!(tpl.schema.columns.iter().filter(|c| c.is_unique_key).count(), 1);
    }

    fn insert_email(db: &crate::db::Database, id: &str, subject: &str, body: &str) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at) \
             VALUES ('acct', 'imap', 'owner@example.test', 'Owner', 0)",
            [],
        )
        .expect("seed account");
        conn.execute(
            "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, sender_domain, \
                                 recipients_json, cc_json, snippet, timestamp, mailbox, category, created_at) \
             VALUES (?1, 'acct', ?1, ?2, 'Website', 'noreply@site.example', 'site.example', '[]', '[]', '', \
                     strftime('%s','now'), 'inbox', 'primary', 0)",
            rusqlite::params![id, subject],
        )
        .expect("insert email");
        conn.execute(
            "INSERT INTO emails_fts (email_id, subject, sender, body) VALUES (?1, ?2, 'Website', ?3)",
            rusqlite::params![id, subject, body],
        )
        .expect("index email");
    }

    #[test]
    fn contact_form_scope_finds_form_notifications_by_their_body_footer() {
        // Form mailers put the site in From: and vary the subject, so the
        // reliable marker is the plugin footer in the body (English and
        // Spanish shown), not the subject line.
        let db = crate::db::Database::new_for_testing().expect("db");
        insert_email(
            &db,
            "es",
            "Propuesta para Example Studio",
            "De: Ana Ruiz ana@cliente.example\n\nHola...\n-- \nEste mensaje se ha enviado desde un formulario de contacto en Example Studio",
        );
        insert_email(
            &db,
            "en",
            "New message",
            "From: Sam Lee <sam@buyer.example>\n\nHi...\n-- \nThis e-mail was sent from a contact form on Example Studio",
        );
        insert_email(
            &db,
            "plain",
            "Quarterly form update",
            "Please fill in the tax form by Friday.",
        );

        let tpl = get("contact_form_leads").expect("contact form template exists");
        let mut ids = crate::services::lenses::scope::evaluate(&db, &tpl.default_scope).expect("scope");
        ids.sort();
        assert_eq!(ids, vec!["en".to_string(), "es".to_string()]);
    }
}
