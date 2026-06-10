// Orchestrates the email_classification eval.
//
// Wire format:
//   The eval sends a Qwen-flavoured classifier prompt (taxonomy + class
//   definitions, adapted from the reference `model_client.py` shipped with
//   `distil-labs/distil-email-classifier`) and a user-style turn wrapping
//   the email in <question>…</question>. The model emits
//   `<output>AI/Label</output>` (or the bare label, which `parse_label`
//   accepts).
//
// Backends:
//   * `llamacpp` (default) — routes through `AiService::complete()` against
//     the catalog model id passed in (e.g. `qwen3.5-4b-q4_k_m`,
//     `qwen3.5-9b-q4_k_m`). This is what `make eval-all MODEL=…` exercises
//     when comparing two Qwen variants side by side.
//   * `ollama` — kept for back-compat with the original distil flow that
//     calls `/api/chat` directly, because that fine-tune requires its
//     specific chat template injected by Ollama (the embedded llama.cpp
//     path can't reproduce it bit-for-bit).
//
// Steps:
//   1. Copy/open the benchmark SQLite DB and resolve the target account.
//   2. Apply any `EMAILOPS_EVAL_MODEL` override (sets `ai_provider` +
//      `ai_model` on the temp DB only).
//   3. Sample N newest primary-category emails.
//   4. For each email: send classifier prompt → parse label → record latency.
//   5. (Optional) call OpenRouter judge to score reasonableness.
//   6. Render HTML report.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::params;
use serde::Deserialize;

use crate::ai::provider::CompletionOptions;
use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::email_classification::report::{render_report, ReportCase};
use crate::evals::email_classification::{parse_label, LABELS};
use crate::evals::{EvalError, EvalResult};
use crate::services::ai::AiService;

const OLLAMA_BASE_URL: &str = "http://localhost:11434";

#[derive(Debug, Clone)]
pub struct EmailClassificationConfig {
    pub account_hint: String,
    pub limit: usize,
    pub model: String,
    pub provider_name: String,
    pub no_judge: bool,
    pub yes: bool,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
    pub out_dir: PathBuf,
}

/// Abstracts the chat backend so the runner can be swapped between the
/// embedded llama.cpp runtime (default) and a local Ollama daemon (the
/// original distil fine-tune path) without per-call branching.
#[async_trait::async_trait]
trait Classifier: Send + Sync {
    async fn classify(&self, question: &str) -> EvalResult<String>;
}

pub async fn run(mut cfg: EmailClassificationConfig) -> EvalResult<PathBuf> {
    // 1. Optional judge gate.
    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let judge_model_env = std::env::var("OPENROUTER_JUDGE_MODEL").ok();
    let judge_enabled = !cfg.no_judge && api_key.is_some();
    if judge_enabled && !cfg.yes {
        return Err(EvalError::Aborted(
            "judge requires --yes (sends email excerpts to OpenRouter) or --no-judge".into(),
        ));
    }
    if cfg.no_judge {
        eprintln!("[classify-eval] judge DISABLED — descriptive metrics only.");
    } else if api_key.is_none() {
        eprintln!("[classify-eval] judge DISABLED (no OPENROUTER_API_KEY).");
    } else {
        eprintln!(
            "[classify-eval] judge ENABLED — model={}",
            judge_model_env
                .clone()
                .unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into())
        );
    }

    // 2. Prepare DB.
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "email-classification")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);

    // Env-var override (set by `make eval-all MODEL=…`).
    if let Some((provider, model)) = crate::evals::shared::apply_eval_model_override_from_env(&db)? {
        cfg.provider_name = provider;
        cfg.model = model;
    }

    // 3. Resolve account.
    let accounts = db.list_accounts()?;
    let hint = cfg.account_hint.trim();
    let account = accounts
        .iter()
        .find(|a| a.id.eq_ignore_ascii_case(hint) || a.email.eq_ignore_ascii_case(hint))
        .ok_or_else(|| EvalError::Config(format!("account '{}' not found in DB", hint)))?;
    eprintln!(
        "[classify-eval] account = {} ({}), model = {}, limit = {}",
        account.email, account.id, cfg.model, cfg.limit
    );

    // 4. Sample.
    let email_ids = sample_email_ids(&db, &account.id, cfg.limit)?;
    if email_ids.is_empty() {
        return Err(EvalError::Config(format!(
            "no primary-category emails found for {}",
            account.email
        )));
    }
    eprintln!("[classify-eval] sampled {} email(s)", email_ids.len());

    // 5. Build classifier client per provider.
    let classifier: Box<dyn Classifier> = match cfg.provider_name.to_ascii_lowercase().as_str() {
        "ollama" => {
            let c = OllamaChat::new(cfg.model.clone());
            c.ping().await?;
            Box::new(c)
        }
        "llamacpp" => Box::new(LlamaCppClassifier::new(db.clone(), &cfg.provider_name, &cfg.model)?),
        other => {
            return Err(EvalError::Config(format!(
                "unsupported provider '{}': use 'llamacpp' (default, embedded) or 'ollama' (distil fine-tune)",
                other
            )));
        }
    };
    let judge = if judge_enabled {
        Some(Judge::new(
            api_key.expect("api_key is Some when judge_enabled"),
            judge_model_env,
        ))
    } else {
        None
    };

    // 6. Run cases.
    let mut cases: Vec<ReportCase> = Vec::new();
    for (i, email_id) in email_ids.iter().enumerate() {
        eprintln!("[classify-eval]   [{}/{}] {}", i + 1, email_ids.len(), email_id);
        match run_case(&db, classifier.as_ref(), judge.as_ref(), email_id).await {
            Ok(c) => {
                if let Some(label) = c.predicted_label.as_deref() {
                    eprintln!("[classify-eval]      → {} ({} ms)", label, c.classify_ms);
                } else {
                    eprintln!(
                        "[classify-eval]      → UNPARSED ({} ms): {:?}",
                        c.classify_ms,
                        c.raw_output.chars().take(120).collect::<String>()
                    );
                }
                cases.push(c);
            }
            Err(e) => {
                eprintln!("[classify-eval]     ERROR: {}", e);
                cases.push(ReportCase::error(email_id, format!("{}", e)));
            }
        }
    }

    // 7. Render.
    let judge_model_for_report = judge
        .as_ref()
        .map(|j| j.model.clone())
        .unwrap_or_else(|| "(judge disabled)".into());
    let path = render_report(
        &cfg.out_dir,
        &cases,
        &account.email,
        &cfg.model,
        judge_enabled,
        &judge_model_for_report,
    )?;
    eprintln!("[classify-eval] report → {}", path.display());
    Ok(path)
}

async fn run_case(
    db: &Arc<Database>,
    classifier: &dyn Classifier,
    judge: Option<&Judge>,
    email_id: &str,
) -> EvalResult<ReportCase> {
    let email = db
        .get_email_by_id(email_id)?
        .ok_or_else(|| EvalError::Config(format!("email {} missing", email_id)))?;
    let body_raw = db.get_email_body(email_id).unwrap_or_default();
    let body_plain = if body_raw.is_empty() {
        email.snippet.clone()
    } else {
        crate::util::html::strip_html_for_fts(&body_raw)
    };
    let body_for_prompt = truncate_chars(&body_plain, 1500);

    let question = format!(
        "Subject: {}\nFrom: {} <{}>\n\n{}",
        email.subject, email.sender, email.sender_email, body_for_prompt
    );

    let started = Instant::now();
    let raw_output = classifier.classify(&question).await?;
    let classify_ms = started.elapsed().as_millis() as i64;
    let predicted_label = parse_label(&raw_output).map(|s| s.to_string());

    let verdict = match (judge, predicted_label.as_deref()) {
        (Some(j), Some(label)) => Some(j.score(&email.subject, &email.sender, &body_plain, label).await),
        _ => None,
    };

    Ok(ReportCase::ok(
        email_id,
        &email,
        body_plain,
        predicted_label,
        raw_output,
        classify_ms,
        verdict,
    ))
}

fn sample_email_ids(db: &Database, account_id: &str, limit: usize) -> EvalResult<Vec<String>> {
    let conn = db.reader();
    let mut stmt = conn.prepare(
        "SELECT id FROM emails \
         WHERE account_id = ?1 \
           AND is_deleted = 0 \
           AND category = 'primary' \
         ORDER BY timestamp DESC \
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![account_id, limit as i64], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 4));
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

// ── Ollama chat client ──────────────────────────────────────────────────────
//
// Direct `/api/chat` integration. The system prompt is the verbatim
// classifier task description from `model_client.py`; the user message
// wraps the email in <question>…</question> blocks. We disable thinking
// (`think: false`) because it is a non-thinking fine-tune — the chat
// template injects an empty `<think></think>` block when `Think` is unset
// which the model treats as a stop signal for thinking, then emits its
// answer directly.

struct OllamaChat {
    model: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct ChatResp {
    message: ChatMsg,
}

#[derive(Deserialize)]
struct ChatMsg {
    #[serde(default)]
    content: String,
}

#[async_trait::async_trait]
impl Classifier for OllamaChat {
    async fn classify(&self, question: &str) -> EvalResult<String> {
        self.classify_impl(question).await
    }
}

impl OllamaChat {
    fn new(model: String) -> Self {
        Self {
            model,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
        }
    }

    async fn ping(&self) -> EvalResult<()> {
        let resp = self
            .client
            .get(format!("{}/api/tags", OLLAMA_BASE_URL))
            .send()
            .await
            .map_err(|e| {
                EvalError::Config(format!(
                    "Ollama not reachable at {} ({}). Is `ollama serve` running?",
                    OLLAMA_BASE_URL, e
                ))
            })?;
        if !resp.status().is_success() {
            return Err(EvalError::Config(format!(
                "Ollama /api/tags returned {}",
                resp.status()
            )));
        }
        // Verify the model is registered.
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| EvalError::Config(format!("Ollama /api/tags response unparseable: {}", e)))?;
        let models = body
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let known: Vec<String> = models
            .iter()
            .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
            .collect();
        let model_root = self.model.split(':').next().unwrap_or(&self.model);
        let found = known
            .iter()
            .any(|n| n == &self.model || n.split(':').next().unwrap_or(n) == model_root);
        if !found {
            return Err(EvalError::Config(format!(
                "Ollama model '{}' not registered. Run:\n  cd distil-email-classifier && ollama create email-classifier -f Modelfile\nKnown: {}",
                self.model,
                known.join(", ")
            )));
        }
        Ok(())
    }

    async fn classify_impl(&self, question: &str) -> EvalResult<String> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": false,
            // Disable Qwen3 thinking — this fine-tune is non-thinking and
            // training data has answers immediately after the empty think block.
            "think": false,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": format!("\n\nNow for the real task, classify the following example\n<question>{}</question>\n", question)},
            ],
            "options": {
                "temperature": 0.0,
                "num_predict": 64,
            }
        });
        let resp = self
            .client
            .post(format!("{}/api/chat", OLLAMA_BASE_URL))
            .json(&body)
            .send()
            .await
            .map_err(|e| EvalError::Config(format!("ollama request: {}", e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EvalError::Config(format!("ollama {}: {}", status, text)));
        }
        let parsed: ChatResp = resp
            .json()
            .await
            .map_err(|e| EvalError::Config(format!("ollama parse: {}", e)))?;
        Ok(parsed.message.content)
    }
}

// ── Embedded llama.cpp classifier ───────────────────────────────────────────
//
// For Qwen-style general chat models served by the embedded llama.cpp
// runtime, we cannot reproduce the distil fine-tune's chat template.
// Instead we feed the classifier prompt as a single completion: SYSTEM_PROMPT
// followed by the email wrapped in <question>…</question>. Models that
// follow the format (Qwen 3 / 3.5) emit `<output>AI/Label</output>` reliably
// enough for `parse_label` to recover the answer. `AiService::complete()`
// drives the provider — same code path the running app uses.

struct LlamaCppClassifier {
    service: AiService,
}

impl LlamaCppClassifier {
    fn new(db: Arc<Database>, provider_name: &str, model: &str) -> EvalResult<Self> {
        // `build_provider` resolves the catalog id → GGUF path and reuses the
        // cached runtime when one is already loaded for the same paths.
        let provider = AiService::build_provider(&db, provider_name, model)?;
        let service = AiService::with_provider(db, provider);
        Ok(Self { service })
    }
}

#[async_trait::async_trait]
impl Classifier for LlamaCppClassifier {
    async fn classify(&self, question: &str) -> EvalResult<String> {
        let prompt = format!(
            "{system}\n\nNow for the real task, classify the following example\n<question>{q}</question>\n",
            system = SYSTEM_PROMPT,
            q = question,
        );
        let out = self
            .service
            .complete(
                &prompt,
                "email_classification_eval",
                Some(CompletionOptions {
                    temperature: Some(0.0),
                    max_tokens: Some(64),
                    think: Some(false),
                }),
            )
            .await
            .map_err(|e| EvalError::Config(format!("llama.cpp completion failed: {}", e)))?;
        Ok(out)
    }
}

// Verbatim classifier system prompt from the model's reference
// `model_client.py`. The fine-tune was trained on this exact prompt; any
// deviation degrades accuracy noticeably.
const SYSTEM_PROMPT: &str = r#"
You are a classifier working on a problem described in task_description XML block:
<task_description>## Task
Classify incoming emails into one of ten predefined categories to enable intelligent email organization, prioritization, and automation workflows. The classification must accurately determine the email's primary purpose and intent based on comprehensive analysis of sender information, subject line, body content, formatting patterns, and contextual signals. The system should handle multi-lingual emails (English and French), mixed personal/professional contexts, and edge cases where emails may contain elements of multiple categories.

## Inputs
Complete email content including:
- Subject line (required)
- Email body text (required)
- Sender information (when available)
- Metadata such as timestamps, domains, and formatting (when available)

## Outputs
A single category label that best represents the email's primary purpose and expected user action.

## Classification Guidelines
1. **Multi-category Resolution:** When an email contains elements of multiple categories, classify based on the PRIMARY user action required. Priority order for ambiguous cases: Security > Spam > Billing > Work > Travel > Shipping > Personal > Promotional > Newsletter > Other.
2. **Language Handling:** The system must accurately classify emails in both English and French based on content meaning. French keywords (e.g., 'facture', 'virement', 'livraison') must be recognized.
3. **Context Awareness:** Consider sender domain and structure. E.g., '@linkedin.com' about jobs is AI/Work, but about network posts is AI/Newsletter.
4. **Edge Case Principles:**
   - Security concerns always take precedence.
   - Obvious spam/phishing is always AI/Spam.
   - Transactional emails (receipts) go to AI/Billing.
   - Personal relationships override platform context.
   - Work context is determined by professional intent, not just sender.

## Decision Logic Examples
- **LinkedIn Flow:** Job posting = AI/Work; Personal msg = AI/Personal; Digest = AI/Newsletter; Profile view = AI/Other.
- **Payment Flow:** If amount+ID present = AI/Billing; If phishing/scam = AI/Spam; If shipping focus = AI/Shipping.
- **Notification Flow:** Security alert = AI/Security; Payment = AI/Billing; Delivery = AI/Shipping; Personal msg = AI/Personal.</task_description>
Classify the input into one of the available classes, each class has a name in class_name and description in class_description XML block:

<class_name>AI/Promotional</class_name>
<class_description>Marketing and sales communications from businesses, services, or platforms promoting products, services, features, or special offers. INDICATORS: Discount codes, limited-time deals, product launches, 'Upgrade today' calls-to-action, webinar invitations. EXAMPLES: SaaS discount offers, early access invites, Black Friday sales. EDGE CASES: Work-related webinar invites from vendors count as Promotional.</class_description>


<class_name>AI/Travel</class_name>
<class_description>All communications related to travel arrangements, transportation bookings, accommodations, and trip logistics. INDICATORS: Flight/Hotel confirmations, boarding passes, car rental reservations, itineraries. EXAMPLES: Air France confirmations, Airbnb bookings, Eurostar tickets. EDGE CASES: Work conference travel is AI/Travel (logistics focus), not AI/Work.</class_description>


<class_name>AI/Spam</class_name>
<class_description>Unsolicited, fraudulent, or malicious emails including phishing attempts, scams, lottery notifications, and suspicious requests. INDICATORS: Unrealistic promises ('You won!'), urgent threats, requests for passwords/SSN, generic greetings, poor grammar, mismatched sender domains. EXAMPLES: Phishing impersonating Amazon/banks, inheritance scams, crypto schemes. EDGE CASES: Legitimate security alerts go to AI/Security; aggressive but legitimate marketing goes to AI/Promotional.</class_description>


<class_name>AI/Shipping</class_name>
<class_description>Order fulfillment communications including shipping confirmations, tracking updates, delivery notifications, and returns. INDICATORS: Tracking numbers (UPS/FedEx), 'Out for delivery' status, delivered photos, return labels. EXAMPLES: Amazon shipment updates, UPS delivery notifications. EDGE CASES: Order confirmations without shipping info often go to AI/Billing.</class_description>


<class_name>AI/Security</class_name>
<class_description>Account security notifications including login alerts, authentication codes, and password changes. INDICATORS: New device logins, 2FA codes, password reset requests, suspicious activity alerts. EXAMPLES: Google sign-in alerts, Microsoft 2FA codes, GitHub security keys. EDGE CASES: If the email is a scam threat, it is AI/Spam.</class_description>


<class_name>AI/Billing</class_name>
<class_description>Financial transaction communications including invoices, payment receipts, subscription charges, and refunds. INDICATORS: Invoice numbers, transaction IDs, 'Payment successful', subscription renewals, tax receipts. EXAMPLES: Stripe receipts, Netflix renewals, cloud billing statements. EDGE CASES: Order confirmations with payment info are AI/Billing; Expired trial upsells are AI/Promotional.</class_description>


<class_name>AI/Work</class_name>
<class_description>Professional and employment-related communications including job opportunities, project updates, team collaboration, and career development. INDICATORS: Job postings, meeting agendas, sprint planning, pull requests, performance reviews, client emails. EXAMPLES: LinkedIn Recruiter messages, Jira updates, Slack digest, client project requirements. EDGE CASES: Professional conference travel is AI/Travel; Work-related SaaS receipts are AI/Billing.</class_description>


<class_name>AI/Newsletter</class_name>
<class_description>Curated content digests, regular informational updates, or periodic communications from subscribed sources. INDICATORS: Daily/weekly cadence, multiple article links, 'digest', 'roundup', unsubscribe links. EXAMPLES: TechCrunch daily, GitHub trending, Substack newsletters. EDGE CASES: A single dedicated promotional email from a newsletter sender is AI/Promotional.</class_description>


<class_name>AI/Personal</class_name>
<class_description>Direct personal communications from friends, family, or colleagues regarding non-professional matters. INDICATORS: Casual tone, social plans (coffee/dinner), birthday wishes, personal advice. EXAMPLES: Friend asking to hang out, family updates, personal networking. EDGE CASES: Colleagues emailing about work are AI/Work; Platform notifications about messages are AI/Other.</class_description>


<class_name>AI/Other</class_name>
<class_description>Miscellaneous communications including platform notifications, system messages, event registrations, and administrative notices. INDICATORS: Automated system updates, terms of service changes, community moderation, badge awards, meetup confirmations. EXAMPLES: Reddit upvote notifications, Terms of Service updates, event registrations. EDGE CASES: Security notifications must go to AI/Security.</class_description>

Write the name of the predicted class inside output XML block
For example, if the input matches class test_output, write
<output>test_output</output>
"#;

// ── LLM-as-judge ────────────────────────────────────────────────────────────

pub struct Judge {
    api_key: String,
    pub model: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct JudgeVerdict {
    pub correct: Option<bool>,
    pub suggested_label: Option<String>,
    pub rationale: Option<String>,
    pub error: Option<String>,
}

impl Judge {
    fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into()),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
        }
    }

    async fn score(&self, subject: &str, sender: &str, body: &str, predicted: &str) -> JudgeVerdict {
        let body_excerpt = body.chars().take(1000).collect::<String>();
        let labels_csv = LABELS.join(", ");
        let user = format!(
            "Email subject: {subject}\nFrom: {sender}\nBody (excerpt):\n{body_excerpt}\n\n\
             A small classifier predicted the label: {predicted}\n\n\
             Allowed labels: {labels_csv}\n\n\
             Decide whether the predicted label is a reasonable category for this email. \
             Respond ONLY with compact JSON: {{\"correct\": true|false, \"suggested\": \"<one of the allowed labels>\", \"rationale\": \"<one short sentence>\"}}"
        );
        let body = serde_json::json!({
            "model": self.model,
            "temperature": 0.0,
            "max_tokens": 200,
            "messages": [
                {"role": "system", "content": "You are a careful evaluator of an email-classification model. Output JSON only."},
                {"role": "user", "content": user}
            ]
        });
        let resp = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                return JudgeVerdict {
                    correct: None,
                    suggested_label: None,
                    rationale: None,
                    error: Some(format!("network: {}", e)),
                };
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return JudgeVerdict {
                correct: None,
                suggested_label: None,
                rationale: None,
                error: Some(format!("openrouter {}: {}", status, text)),
            };
        }
        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                return JudgeVerdict {
                    correct: None,
                    suggested_label: None,
                    rationale: None,
                    error: Some(format!("parse: {}", e)),
                };
            }
        };
        let content = json
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let cleaned = strip_code_fence(&content);
        match serde_json::from_str::<serde_json::Value>(cleaned) {
            Ok(v) => JudgeVerdict {
                correct: v.get("correct").and_then(|x| x.as_bool()),
                suggested_label: v.get("suggested").and_then(|x| x.as_str()).map(|s| s.to_string()),
                rationale: v.get("rationale").and_then(|x| x.as_str()).map(|s| s.to_string()),
                error: None,
            },
            Err(e) => JudgeVerdict {
                correct: None,
                suggested_label: None,
                rationale: Some(content),
                error: Some(format!("json parse: {}", e)),
            },
        }
    }
}

fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim().trim_end_matches("```").trim();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim().trim_end_matches("```").trim();
    }
    t
}
