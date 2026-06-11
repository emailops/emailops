// `draft_eval` — evaluation harness for the AI draft generator.
//
// Samples N reply pairs from a temp copy of the production SQLite DB where the
// user actually replied to an inbound message, regenerates a draft for the inbound message
// using `services::emails::generate_draft`, and scores the draft against the
// real reply along four axes:
//   - Style match    (does it sound like the user?)
//   - Completeness   (covers the same key points?)
//   - Tone fit       (appropriate response to the inbound email?)
//   - Length fit     (length in the same ballpark?)
//
// Scoring is done by the same AiService configured for the app — so a remote
// judge model can be selected via the `ai_provider` / `ai_model` prefs (the
// eval reads them as the app does). For unbiased scoring, configure a
// different model than the one generating drafts.
//
// Usage:
//   cargo run --bin draft_eval -- --n 5
//
// Flags:
//   --n            Number of cases (default 5).
//   --prod-db      Path to the prod DB. Default: macOS app-data dir.
//   --out          Output directory. Default: src-tauri/reports/evaluations/drafts.
//   --account      Account id or email. Defaults to the single enabled account.
//
// Requirements:
//   - By default, the eval copies the DB to a temp directory before running.
//   - The configured AI provider must be reachable.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use serde::Serialize;

use emailops_lib::ai::provider::CompletionOptions;
use emailops_lib::db::Database;
use emailops_lib::evals::shared::apply_eval_model_override_from_env;
use emailops_lib::models::Email;
use emailops_lib::services::ai::AiService;
use emailops_lib::services::emails::{generate_draft, DraftSource};
use emailops_lib::util::private_eval_db::{prepare_eval_db, EvalDbMode};

#[derive(Parser, Debug)]
#[command(
    name = "draft_eval",
    about = "Evaluate AI draft generation against ground-truth user replies.",
    long_about = None,
)]
struct Args {
    /// Number of reply pairs to evaluate.
    #[arg(long, default_value_t = 5)]
    n: usize,

    /// Account id or email. Defaults to the single enabled account.
    #[arg(long)]
    account: Option<String>,

    /// Path to the production SQLite DB.
    #[arg(long)]
    prod_db: Option<PathBuf>,

    /// Open the production DB in place instead of copying it to a temp DB.
    #[arg(long, hide = true)]
    in_place_dangerous: bool,

    /// Output directory. Created if missing.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct JudgeScores {
    style_match: u8,
    completeness: u8,
    tone_fit: u8,
    length_fit: u8,
    comment: String,
}

#[derive(Debug, Serialize)]
struct CaseResult {
    case_id: String,
    thread_id: String,
    inbound_email_id: String,
    inbound_subject: String,
    inbound_sender: String,
    ground_truth: String,
    predicted: String,
    sources_count: usize,
    sources: Vec<DraftSource>,
    char_ratio: f32,
    word_overlap: f32,
    elapsed_ms: u128,
    scores: Option<JudgeScores>,
    error: Option<String>,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();
    let prod_db = args
        .prod_db
        .clone()
        .or_else(default_prod_db)
        .unwrap_or_else(|| PathBuf::from("emailops.db"));
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("reports/evaluations/drafts"));
    let db_mode = if args.in_place_dangerous {
        EvalDbMode::InPlaceDangerous
    } else {
        EvalDbMode::CopyToTemp
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(args, prod_db, db_mode, out_dir)) {
        Ok(report_path) => {
            eprintln!("[draft_eval] done → {}", report_path.display());
        }
        Err(e) => {
            eprintln!("[draft_eval] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run(
    args: Args,
    prod_db: PathBuf,
    db_mode: EvalDbMode,
    out_dir: PathBuf,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let prepared_db = prepare_eval_db(&prod_db, db_mode, "draft")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);

    apply_eval_model_override_from_env(&db).map_err(|e| e.to_string())?;

    let account_id = resolve_account_id(&db, args.account.as_deref())?;
    let user_email = db.get_account(&account_id)?.map(|a| a.email).unwrap_or_default();

    let pairs = sample_reply_pairs(&db, &account_id, &user_email, args.n)?;
    if pairs.is_empty() {
        return Err("No usable reply pairs found in DB (need sent emails with prior inbound).".into());
    }
    eprintln!(
        "[draft_eval] sampled {} reply pairs from account {} ({})",
        pairs.len(),
        account_id,
        user_email
    );

    let ai = AiService::new(db.clone())?;
    let ai_config = AiService::get_config(&db)?;
    eprintln!(
        "[draft_eval] AI provider={} model={}",
        ai_config.provider, ai_config.model
    );

    let mut results: Vec<CaseResult> = Vec::with_capacity(pairs.len());
    for (i, pair) in pairs.iter().enumerate() {
        eprintln!(
            "[draft_eval] [{}/{}] inbound={} subject={:?}",
            i + 1,
            pairs.len(),
            pair.inbound.id,
            pair.inbound.subject
        );
        let started = Instant::now();
        match generate_draft(&db, &pair.inbound.id, None).await {
            Ok(result) => {
                let elapsed = started.elapsed().as_millis();
                let predicted = result.body.clone();
                let char_ratio = if pair.ground_truth.is_empty() {
                    0.0
                } else {
                    predicted.chars().count() as f32 / pair.ground_truth.chars().count() as f32
                };
                let word_overlap = compute_word_overlap(&predicted, &pair.ground_truth);
                let scores = match judge_draft(&ai, &pair.inbound, &pair.ground_truth, &predicted).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("[draft_eval] judge error: {}", e);
                        None
                    }
                };
                results.push(CaseResult {
                    case_id: format!("case_{:02}", i + 1),
                    thread_id: pair.inbound.thread_id.clone(),
                    inbound_email_id: pair.inbound.id.clone(),
                    inbound_subject: pair.inbound.subject.clone(),
                    inbound_sender: pair.inbound.sender.clone(),
                    ground_truth: pair.ground_truth.clone(),
                    predicted,
                    sources_count: result.sources.len(),
                    sources: result.sources,
                    char_ratio,
                    word_overlap,
                    elapsed_ms: elapsed,
                    scores,
                    error: None,
                });
            }
            Err(e) => {
                eprintln!("[draft_eval] generation error: {}", e);
                results.push(CaseResult {
                    case_id: format!("case_{:02}", i + 1),
                    thread_id: pair.inbound.thread_id.clone(),
                    inbound_email_id: pair.inbound.id.clone(),
                    inbound_subject: pair.inbound.subject.clone(),
                    inbound_sender: pair.inbound.sender.clone(),
                    ground_truth: pair.ground_truth.clone(),
                    predicted: String::new(),
                    sources_count: 0,
                    sources: vec![],
                    char_ratio: 0.0,
                    word_overlap: 0.0,
                    elapsed_ms: started.elapsed().as_millis(),
                    scores: None,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    std::fs::create_dir_all(&out_dir)?;
    let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let json_path = out_dir.join(format!("draft_eval_{}.json", stamp));
    let md_path = out_dir.join(format!("draft_eval_{}.md", stamp));

    std::fs::write(&json_path, serde_json::to_string_pretty(&results)?)?;
    std::fs::write(
        &md_path,
        render_markdown(&results, &ai_config.provider, &ai_config.model),
    )?;

    Ok(md_path)
}

struct ReplyPair {
    inbound: Email,
    ground_truth: String,
}

/// Sample up to `n` reply pairs. We walk sent emails newest-first, and for
/// each one find the immediately preceding inbound message in the same thread
/// (the message the user was responding to). Skips pairs where either side
/// has unusably short / long body content so the judge has a fair target.
fn sample_reply_pairs(
    db: &Arc<Database>,
    account_id: &str,
    user_email: &str,
    n: usize,
) -> Result<Vec<ReplyPair>, Box<dyn std::error::Error>> {
    let mut pairs: Vec<ReplyPair> = Vec::new();
    let mut offset = 0i32;
    let batch_size = 100i32;
    let max_scan = 1000i32;

    while pairs.len() < n && offset < max_scan {
        let batch = db.get_emails(account_id, batch_size, offset, None, Some("sent"), None)?;
        if batch.is_empty() {
            break;
        }
        for sent in batch {
            if pairs.len() >= n {
                break;
            }
            // Skip if not actually authored by the user.
            if !user_email.is_empty() && !sent.sender_email.eq_ignore_ascii_case(user_email) {
                continue;
            }
            let thread = match db.get_thread(account_id, &sent.thread_id) {
                Ok(t) => t,
                Err(_) => continue,
            };
            // Find the latest inbound message that predates this sent reply.
            let inbound = thread
                .iter()
                .filter(|m| m.timestamp < sent.timestamp)
                .filter(|m| !user_email.is_empty() && !m.sender_email.eq_ignore_ascii_case(user_email))
                .max_by_key(|m| m.timestamp);
            let Some(inbound) = inbound else { continue };

            let truth_body = db.get_email_body(&sent.id).unwrap_or_default();
            let truth_plain = html_to_plain(&truth_body);
            let truth_trim = truth_plain.trim();
            // Filter out auto-replies, one-liners, or essay-length replies — they
            // skew the judge in obvious ways without measuring anything useful.
            let len = truth_trim.chars().count();
            if !(80..=4000).contains(&len) {
                continue;
            }

            pairs.push(ReplyPair {
                inbound: inbound.clone(),
                ground_truth: truth_trim.to_string(),
            });
        }
        offset += batch_size;
    }
    Ok(pairs)
}

fn resolve_account_id(db: &Arc<Database>, hint: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    let accounts = db.list_accounts()?;
    match hint {
        Some(h) => {
            let h = h.trim();
            accounts
                .iter()
                .find(|a| a.id.eq_ignore_ascii_case(h) || a.email.eq_ignore_ascii_case(h))
                .map(|a| a.id.clone())
                .ok_or_else(|| format!("account '{}' not found in DB", h).into())
        }
        None => {
            let enabled: Vec<_> = accounts.into_iter().filter(|a| a.enabled).collect();
            match enabled.len() {
                0 => Err("no enabled accounts in DB".into()),
                1 => Ok(enabled[0].id.clone()),
                _ => Err("multiple enabled accounts — pass --account <id|email>".into()),
            }
        }
    }
}

/// Score a draft against ground truth on four 1–5 axes using the configured
/// AI provider as judge. Returns the parsed scores; falls back to None on
/// parse failure so the eval can still write a report.
async fn judge_draft(
    ai: &AiService,
    inbound: &Email,
    ground_truth: &str,
    predicted: &str,
) -> Result<JudgeScores, Box<dyn std::error::Error>> {
    let prompt = format!(
        r#"You are evaluating an AI-generated email reply against the user's actual reply.

Score the AI draft on four axes, integer 1–5 each (5 = excellent):
- style_match: does the AI draft match the user's voice/register as shown by GROUND_TRUTH?
- completeness: does it cover the same key points as GROUND_TRUTH?
- tone_fit: is the tone appropriate as a reply to INBOUND?
- length_fit: is the length in the same ballpark as GROUND_TRUTH?

Respond with ONLY a JSON object (no markdown fences) of this exact shape:
{{"style_match": N, "completeness": N, "tone_fit": N, "length_fit": N, "comment": "one-sentence rationale"}}

INBOUND (the email being replied to)
From: {sender}
Subject: {subject}
Body:
{inbound_body}

GROUND_TRUTH (user's actual reply)
{ground_truth}

AI_DRAFT
{predicted}
"#,
        sender = inbound.sender,
        subject = inbound.subject,
        inbound_body = truncate(&inbound.snippet, 1500),
        ground_truth = truncate(ground_truth, 2000),
        predicted = truncate(predicted, 2000),
    );

    let response = ai
        .complete(
            &prompt,
            "draft_eval_judge",
            Some(CompletionOptions {
                temperature: Some(0.0),
                max_tokens: Some(300),
                think: Some(false),
            }),
        )
        .await?;

    // Extract a JSON object from the response — models sometimes wrap in
    // explanation or markdown fences. We find the first '{' / last '}' pair.
    let trimmed = response.trim();
    let start = trimmed.find('{').ok_or("judge response had no JSON object")?;
    let end = trimmed.rfind('}').ok_or("judge response had no JSON object")?;
    let json_slice = &trimmed[start..=end];

    #[derive(serde::Deserialize)]
    struct Raw {
        style_match: u8,
        completeness: u8,
        tone_fit: u8,
        length_fit: u8,
        #[serde(default)]
        comment: String,
    }
    let raw: Raw = serde_json::from_str(json_slice)
        .map_err(|e| format!("failed to parse judge JSON: {} (raw: {:?})", e, json_slice))?;

    let clamp = |v: u8| v.clamp(1, 5);
    Ok(JudgeScores {
        style_match: clamp(raw.style_match),
        completeness: clamp(raw.completeness),
        tone_fit: clamp(raw.tone_fit),
        length_fit: clamp(raw.length_fit),
        comment: raw.comment,
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut)
}

fn html_to_plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            out.push(' ');
            continue;
        }
        if !in_tag {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lightweight token overlap (Jaccard on lowercased ≥3-char tokens). Useful
/// as a sanity check alongside the LLM judge — if both score 0 on a case,
/// the model produced something semantically far from the ground truth.
fn compute_word_overlap(a: &str, b: &str) -> f32 {
    use std::collections::HashSet;
    let tokenize = |s: &str| -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .map(|w| w.to_lowercase())
            .filter(|w| w.chars().count() >= 3)
            .collect()
    };
    let sa = tokenize(a);
    let sb = tokenize(b);
    if sa.is_empty() && sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn render_markdown(results: &[CaseResult], provider: &str, model: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# AI Draft Evaluation — {}\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    s.push_str(&format!(
        "- Cases: {}\n- Provider: `{}`\n- Model: `{}`\n\n",
        results.len(),
        provider,
        model
    ));

    // Aggregate scores
    let scored: Vec<&CaseResult> = results.iter().filter(|r| r.scores.is_some()).collect();
    if !scored.is_empty() {
        let n = scored.len() as f32;
        let avg = |f: fn(&JudgeScores) -> u8| -> f32 {
            scored.iter().map(|r| f(r.scores.as_ref().unwrap()) as f32).sum::<f32>() / n
        };
        let style = avg(|s| s.style_match);
        let comp = avg(|s| s.completeness);
        let tone = avg(|s| s.tone_fit);
        let length = avg(|s| s.length_fit);
        s.push_str("## Aggregate scores (1–5)\n\n");
        s.push_str("| style_match | completeness | tone_fit | length_fit | mean |\n");
        s.push_str("|---|---|---|---|---|\n");
        s.push_str(&format!(
            "| {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |\n\n",
            style,
            comp,
            tone,
            length,
            (style + comp + tone + length) / 4.0
        ));
    }

    // Per-case table
    s.push_str("## Cases\n\n");
    s.push_str("| case | inbound | sources | char_ratio | word_overlap | scores (s/c/t/l) | ms |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for r in results {
        let score_str = match &r.scores {
            Some(sc) => format!(
                "{}/{}/{}/{}",
                sc.style_match, sc.completeness, sc.tone_fit, sc.length_fit
            ),
            None => "-".to_string(),
        };
        s.push_str(&format!(
            "| {} | {} | {} | {:.2} | {:.2} | {} | {} |\n",
            r.case_id,
            truncate(&r.inbound_subject, 50).replace('|', "\\|"),
            r.sources_count,
            r.char_ratio,
            r.word_overlap,
            score_str,
            r.elapsed_ms
        ));
    }

    // Per-case detail
    s.push_str("\n## Details\n\n");
    for r in results {
        s.push_str(&format!("### {} — `{}`\n\n", r.case_id, r.inbound_email_id));
        s.push_str(&format!(
            "**Inbound:** {} — *{}*\n\n",
            r.inbound_sender, r.inbound_subject
        ));
        if let Some(err) = &r.error {
            s.push_str(&format!("**Error:** `{}`\n\n", err));
            continue;
        }
        if let Some(sc) = &r.scores {
            s.push_str(&format!(
                "**Scores:** style={} completeness={} tone={} length={}  \n*{}*\n\n",
                sc.style_match, sc.completeness, sc.tone_fit, sc.length_fit, sc.comment
            ));
        }
        s.push_str("**Ground truth:**\n\n");
        s.push_str("```\n");
        s.push_str(&truncate(&r.ground_truth, 1500));
        s.push_str("\n```\n\n");
        s.push_str("**Predicted:**\n\n");
        s.push_str("```\n");
        s.push_str(&truncate(&r.predicted, 1500));
        s.push_str("\n```\n\n");
        if !r.sources.is_empty() {
            s.push_str(&format!("**RAG sources ({}):**\n\n", r.sources.len()));
            for (i, src) in r.sources.iter().enumerate() {
                s.push_str(&format!(
                    "- [{}] *{}* — {} {}\n",
                    i + 1,
                    src.subject,
                    src.sender,
                    if src.sent_by_user { "(your reply)" } else { "" }
                ));
            }
            s.push('\n');
        }
        s.push_str("---\n\n");
    }
    s
}

#[cfg(target_os = "macos")]
fn default_prod_db() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("com.emailops.app")
            .join("emailops.db")
    })
}

#[cfg(not(target_os = "macos"))]
fn default_prod_db() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("com.emailops.app").join("emailops.db"))
}
