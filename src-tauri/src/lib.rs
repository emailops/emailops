// Crate-wide lint gate.
//
// `.unwrap()` / `.expect(...)` on a `Result` or `Option` panics on the
// error path. In a Tauri desktop app, a panic on a worker thread can take
// the whole window with it — terrible UX. The default position is therefore
// "don't panic"; legitimate exceptions (mutex poisoning, hard-coded literals
// that cannot fail, etc.) must be opted-in with `#[allow(clippy::expect_used)]`
// or `#[allow(clippy::unwrap_used)]` and a one-line comment explaining why.
//
// Tests are exempted globally via `allow-unwrap-in-tests = true` and
// `allow-expect-in-tests = true` in `clippy.toml`.
#![deny(clippy::unwrap_used, clippy::expect_used)]

// These are used only by the desktop shell (`run()`, `AppState`, the instance lock);
// the state they used to serve now lives in `runtime::core`.
// Used by the desktop shell (`run()`, `AppState`, the instance lock). The state
// these used to serve now lives in `runtime::core`.
#[cfg(feature = "desktop")]
use std::collections::HashMap;
#[cfg(feature = "desktop")]
use std::path::PathBuf;
#[cfg(feature = "desktop")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "desktop")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "desktop")]
use tauri::webview::PageLoadEvent;
#[cfg(feature = "desktop")]
use tauri::Manager;

pub mod ai;
// The Tauri command layer. Gated so the crate can be built as a headless library
// (`--no-default-features`) for the CLI and for server front ends.
#[cfg(feature = "desktop")]
pub mod commands;
pub mod db;
pub mod models;
pub mod runtime;
pub mod services;
pub mod sync;
pub mod util;

// Rust-native eval harness. Gated behind the `eval` feature so the production
// binary does not carry Tera / tauri::test / YAML parsing code.
#[cfg(feature = "eval")]
pub mod evals;

// `emailops-cli` power-user / agent CLI + interactive REPL. Gated behind the
// `cli` feature so it (and the `reedline` line editor) never compiles into the
// default/release desktop build. The `emailops-cli` bin is a thin wrapper over
// `cli::run()`.
#[cfg(feature = "cli")]
pub mod cli;

pub use db::Database;
pub use models::error::{AppError, Result};
pub use runtime::AppCore;

#[cfg(feature = "desktop")]
fn should_load_dotenv_with(debug_build: bool, env_override: Option<&str>) -> bool {
    debug_build
        || matches!(
            env_override.map(str::trim),
            Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
        )
}

#[cfg(feature = "desktop")]
fn should_load_dotenv() -> bool {
    let env_override = std::env::var("EMAILOPS_LOAD_DOTENV").ok();
    should_load_dotenv_with(cfg!(debug_assertions), env_override.as_deref())
}

/// Tauri's managed state: an [`AppCore`] plus the desktop-only background scheduler.
///
/// `AppState` used to own every field directly. Those fields moved to [`AppCore`]
/// (`runtime/core.rs`) so that the CLI and a server can build the same state without
/// Tauri. `AppState` keeps `Deref<Target = AppCore>`, so the ~200 command handlers that
/// read `state.db`, `state.ai_background`, `state.dispatcher`, or call
/// `state.sync_queue_for(...)` continue to compile unchanged.
#[cfg(feature = "desktop")]
pub struct AppState {
    core: Arc<AppCore>,
    /// Background sync scheduler (Gmail polling + IMAP IDLE). Desktop-only: it owns
    /// long-lived OS threads and `tauri::async_runtime` handles. A server supervises
    /// syncing with its own reconcile loop instead.
    pub scheduler: services::sync_scheduler::SyncScheduler,
}

#[cfg(feature = "desktop")]
impl std::ops::Deref for AppState {
    type Target = AppCore;
    fn deref(&self) -> &AppCore {
        &self.core
    }
}

#[cfg(feature = "desktop")]
impl AppState {
    /// Wrap an already-built core together with the desktop scheduler.
    pub fn new(core: Arc<AppCore>, scheduler: services::sync_scheduler::SyncScheduler) -> Self {
        Self { core, scheduler }
    }

    /// The shared core, for handing to code that must not depend on Tauri.
    pub fn core(&self) -> Arc<AppCore> {
        Arc::clone(&self.core)
    }

    /// Construct an `AppState` suitable for unit/integration tests.
    #[cfg(test)]
    pub fn for_testing(db: Arc<Database>) -> Self {
        Self {
            core: Arc::new(AppCore::for_testing(db)),
            scheduler: services::sync_scheduler::SyncScheduler::stub(),
        }
    }
}

// `run()` is the Tauri entry point. Startup steps that EmailOps cannot
// continue without (keychain init, data-dir resolution, opening the database)
// route their failures through `fatal_startup_error`, which shows the user a
// readable native dialog and exits cleanly — instead of panicking. A panic
// here is especially bad: the setup hook runs inside macOS'
// `applicationDidFinishLaunching`, so the panic cannot unwind across the
// Objective-C boundary and the process `abort()`s with an opaque crash report.
// The only remaining `.expect(...)` is the final `Builder::run()` (the event
// loop itself failing to start has no graceful recovery).
#[allow(clippy::expect_used)]
#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Dotenv is a developer convenience only. In release builds, keep it off
    // unless explicitly requested to avoid accidental secret loading.
    if should_load_dotenv() {
        for path in [".env.local", ".env", "../.env.local", "../.env"] {
            if dotenvy::from_filename(path).is_ok() {
                break;
            }
        }
    }

    // keyring 4 requires explicit backend selection — picks the platform-native
    // credential store (macOS Keychain, Windows Credential Manager, Linux
    // Secret Service). Must run before any keyring_core::Entry::new(...) call.
    if let Err(e) = services::keychain::init_native_store() {
        fatal_startup_error("initialise the OS keychain", &e.to_string(), None);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let setup_start = std::time::Instant::now();

            // Install the production logger so any code path that goes through
            // `services::logger::log(...)` reaches the frontend `app-log` listener.
            // Until this runs, the global is the NoopLogger and emissions are
            // silently dropped — that's fine for the few µs of bootstrap before
            // this line.
            services::logger::install(Arc::new(services::logger::TauriLogger::new(app.handle().clone())));

            // Install the production event sink so chat streaming, phases,
            // sources, and progress events reach the frontend. Sibling of the
            // logger install above; until this runs the global is a NoopEventSink.
            services::events::install(Arc::new(services::events::TauriEventSink::new(app.handle().clone())));

            // Mobile OAuth bridge. On iOS the authorization code arrives as a
            // custom-scheme URL delivered by the OS, on a different call path
            // from the `start_oauth_flow` awaiting it — this is what connects
            // the two. Installed unconditionally (it is inert on desktop, where
            // the loopback listener is used) so the wiring cannot drift out of
            // sync with the platform branch in `sync::oauth`.
            sync::oauth::mobile_callback::install(app.handle().clone());
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().on_open_url(|event| {
                    for url in event.urls() {
                        sync::oauth::mobile_callback::deliver(url.to_string());
                    }
                });
            }

            let t = std::time::Instant::now();
            // `EMAILOPS_DATA_DIR` overrides Tauri's default `app_data_dir`.
            // Makefile dev workflows set it to an ignored repo-local directory;
            // when unset, fall back to the platform default for this app id.
            let app_data_dir = match std::env::var("EMAILOPS_DATA_DIR") {
                Ok(dir) if !dir.trim().is_empty() => {
                    let p = PathBuf::from(dir.trim());
                    eprintln!("[startup] using EMAILOPS_DATA_DIR override: {}", p.display());
                    p
                }
                _ => match app.path().app_data_dir() {
                    Ok(dir) => dir,
                    Err(e) => fatal_startup_error("locate its data directory", &e.to_string(), None),
                },
            };
            eprintln!(
                "[startup] [{:.0}ms] app_data_dir resolved: {}",
                t.elapsed().as_secs_f64() * 1000.0,
                app_data_dir.display()
            );

            // Enforce single-instance: if another emailops process already holds the
            // lock file, log a warning and exit immediately so we don't accumulate zombies.
            let t = std::time::Instant::now();
            match util::instance_lock::acquire(&app_data_dir) {
                Ok(lock) => {
                    // Keep the lock held for the lifetime of the app. Every
                    // supported OS releases it automatically on process exit,
                    // including after a crash.
                    app.manage(lock);
                }
                // Expected and benign: the user launched a second copy.
                Err(util::instance_lock::LockError::AlreadyRunning) => {
                    eprintln!("[startup] Another instance of EmailOps is already running. Exiting.");
                    std::process::exit(0);
                }
                // The data directory is unusable (read-only, full, permissions).
                // Previously this also exited 0 and printed nothing the user
                // would ever see, which made a broken install look like a
                // normal second launch.
                Err(util::instance_lock::LockError::Unavailable(detail)) => {
                    fatal_startup_error("lock its data directory", &detail, Some(&app_data_dir));
                }
            }
            eprintln!(
                "[startup] [{:.0}ms] instance lock acquired",
                t.elapsed().as_secs_f64() * 1000.0
            );

            let t = std::time::Instant::now();
            let db = match Database::new(app_data_dir.clone()) {
                Ok(db) => db,
                Err(e) => fatal_startup_error("open its local database", &e.to_string(), Some(&app_data_dir)),
            };
            eprintln!(
                "[startup] [{:.0}ms] database initialized",
                t.elapsed().as_secs_f64() * 1000.0
            );

            // Persist app_data_dir so services that only have &Database can
            // resolve on-disk paths (e.g. llama.cpp model files) without access
            // to AppState.
            if let Err(e) = db.set_preference("app_data_dir", &app_data_dir.to_string_lossy()) {
                eprintln!("[startup] Warning: failed to save app_data_dir preference: {e}");
            }

            // Copy any catalog model flagged `bundled: true` from the .app
            // resources directory into the user's models dir, if not already
            // present. This is how the Nomic embedding ships pre-installed —
            // so search/chat works on first launch with no download. Failures
            // here are non-fatal: the download flow remains as a fallback.
            match app.path().resource_dir() {
                Ok(resource_dir) => {
                    for entry in ai::model_catalog::CATALOG.iter().filter(|m| m.bundled) {
                        let src = resource_dir
                            .join("models")
                            .join(match entry.kind {
                                ai::model_catalog::ModelKind::Chat => "chat",
                                ai::model_catalog::ModelKind::Embedding => "embed",
                            })
                            .join(format!("{}.gguf", entry.id));
                        match ai::model_manager::seed_bundled_model(&src, &app_data_dir, entry.kind, entry.id) {
                            Ok(true) => eprintln!("[startup] seeded bundled model: {}", entry.id),
                            Ok(false) => {}
                            Err(e) => eprintln!("[startup] Warning: failed to seed bundled model '{}': {e}", entry.id),
                        }
                    }
                }
                Err(e) => eprintln!("[startup] Warning: cannot resolve resource_dir for bundled models: {e}"),
            }

            // Point ggml at the loadable backend modules that ship beside the
            // app, so a GPU is used when one is present and the CPU backend is
            // used when it is not. Only compiled into builds that bundle them.
            #[cfg(all(feature = "llamacpp", feature = "dynamic-backends"))]
            match app.path().resource_dir() {
                Ok(resource_dir) => {
                    ai::llama_cpp::runtime::set_backends_dir(resource_dir.join("backends"));
                }
                Err(e) => eprintln!("[startup] Warning: cannot resolve resource_dir for ggml backends: {e}"),
            }

            // Promote the legacy free-text `ai_output_language` preference into
            // the typed `ai_output_language_v2` code, then drop the legacy key.
            // Idempotent — see `services::i18n::migrate_legacy_ai_output_language`.
            match services::i18n::migrate_legacy_ai_output_language(&db) {
                Ok(true) => eprintln!("[startup] migrated legacy ai_output_language → ai_output_language_v2"),
                Ok(false) => {}
                Err(e) => eprintln!("[startup] Warning: failed to migrate legacy ai_output_language: {e}"),
            }

            // Backfill the `total` column on `lens_runs` for databases
            // created before the column existed. Idempotent.
            match db.ensure_lens_runs_total_column() {
                Ok(true) => eprintln!("[startup] added 'total' column to lens_runs"),
                Ok(false) => {}
                Err(e) => eprintln!("[startup] Warning: failed to add lens_runs.total column: {e}"),
            }

            // Recover any Lens runs left in `running` state from a previous
            // session (crash, force-quit, dev restart). Without this, the UI
            // would show a permanent "running" badge and a no-op Cancel button.
            match db.reset_orphan_lens_runs() {
                Ok(0) => {}
                Ok(n) => {
                    eprintln!("[startup] reset {n} orphan lens run(s) to failed");
                    // Emit a visible log entry so the user understands why the
                    // sidebar shows "last run failed" after a crash or force-quit.
                    services::logger::log(
                        "warn",
                        "lens",
                        format!("{n} lens run(s) were interrupted by a previous app exit and marked as failed"),
                    );
                }
                Err(e) => eprintln!("[startup] Warning: failed to reset orphan lens runs: {e}"),
            }

            // Patch existing lenses created before built-in template schemas
            // gained `is_unique_key` flags (e.g. invoice_number on the
            // invoices_* templates). Idempotent — only updates rows that
            // still lack the flag.
            match db.migrate_template_unique_keys() {
                Ok(0) => {}
                Ok(n) => eprintln!("[startup] patched unique-key schema on {n} lens(es)"),
                Err(e) => eprintln!("[startup] Warning: lens unique-key migration failed: {e}"),
            }

            let db = Arc::new(db);

            // Kick off a non-blocking backup in the background immediately after
            // startup so we always have a recent snapshot before any sync runs.
            {
                let db_clone = db.clone();
                let backup_dir = app_data_dir.join("backups");
                std::thread::spawn(move || match db_clone.backup(&backup_dir, 7) {
                    Ok(path) => {
                        println!("[backup] Startup backup written to {}", path.display());
                    }
                    Err(e) => {
                        eprintln!("[backup] Startup backup failed (non-fatal): {e}");
                    }
                });
            }

            // Pre-load OAuth tokens into memory so the macOS keychain
            // is accessed once at startup instead of per-account sync.
            let t = std::time::Instant::now();
            services::accounts::warm_token_cache(&db);
            eprintln!(
                "[startup] [{:.0}ms] token cache warmed",
                t.elapsed().as_secs_f64() * 1000.0
            );

            // Backfill imap_account_settings for accounts created before the
            // DB-backed settings table existed. Best-effort; never blocks startup.
            let t = std::time::Instant::now();
            services::accounts::backfill_imap_settings(&db);
            eprintln!(
                "[startup] [{:.0}ms] imap settings backfilled",
                t.elapsed().as_secs_f64() * 1000.0
            );

            let t = std::time::Instant::now();
            let ai_queue = services::task_queue::TaskQueue::new(1, "ai");
            let ai_background = services::task_queue::TaskQueue::new(1, "ai_bg");
            let db_queue = services::task_queue::TaskQueue::new(4, "db");
            // Per-account sync queues are created lazily on first manual sync
            // — see `AppState::sync_queue_for`. Storing the map here keeps
            // ownership in AppState so the queues outlive the request that
            // first created them.
            let sync_queues: Arc<Mutex<HashMap<String, services::task_queue::TaskQueue>>> =
                Arc::new(Mutex::new(HashMap::new()));
            eprintln!(
                "[startup] [{:.0}ms] task queues created",
                t.elapsed().as_secs_f64() * 1000.0
            );

            let sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));
            let sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
                Arc::new(Mutex::new(HashMap::new()));

            let dispatcher: Arc<dyn services::background_tasks::TaskDispatcher> =
                Arc::new(services::background_tasks::RealDispatcher {
                    ai_queue: ai_queue.clone(),
                    ai_background: ai_background.clone(),
                    db_queue: db_queue.clone(),
                });

            let t = std::time::Instant::now();
            // Start the connectivity monitor first so the sync scheduler can
            // read its cached online flag and skip poll ticks while we're
            // offline (avoids spamming the output panel with HTTP errors that
            // the offline banner already explains).
            let connectivity = services::connectivity::ConnectivityMonitor::start(app.handle().clone());

            let scheduler = services::sync_scheduler::SyncScheduler::start(
                db.clone(),
                app_data_dir.clone(),
                app.handle().clone(),
                ai_background.clone(),
                sync_abort_flags.clone(),
                sync_locks.clone(),
                connectivity.online_flag(),
            );

            // Same ingredients the scheduler just took, published for the iOS
            // background-refresh entry point — which is a C function invoked by
            // the system and so has no way to be handed state. Installed on
            // every platform (it is a cheap struct) so the wiring cannot rot
            // behind a `cfg` that only one target compiles.
            services::background_refresh::install(services::background_refresh::RefreshContext {
                db: db.clone(),
                app_data_dir: app_data_dir.clone(),
                app: app.handle().clone(),
                ai_background: ai_background.clone(),
                sync_abort_flags: sync_abort_flags.clone(),
                sync_locks: sync_locks.clone(),
            });
            eprintln!(
                "[startup] [{:.0}ms] sync scheduler started",
                t.elapsed().as_secs_f64() * 1000.0
            );

            let core = Arc::new(AppCore {
                db: db.clone(),
                app_data_dir,
                ai_queue,
                ai_background,
                db_queue,
                sync_queues,
                sync_abort_flags,
                sync_locks,
                connectivity,
                dispatcher,
                tool_registry: Arc::new(services::chat::tools::default_registry()),
            });
            app.manage(AppState::new(core, scheduler));

            // Warm up the AI provider in the background so the first chat turn
            // doesn't pay the full cold-load cost of a multi-GB GGUF. Runs on
            // Tauri's async runtime — fire-and-forget; warmup failures must
            // not block startup. Uses a weak reference to the app handle so
            // shutdown during warmup doesn't prevent the process from exiting.
            //
            // Skip warmup on first launch (no `onboarding_completed` pref yet)
            // so we don't load a multi-GB model before the user has even chosen
            // whether they want AI. Also skip if the master AI toggle is off.
            // First chat turn after re-enabling will pay the cold-load cost,
            // which is acceptable.
            let onboarded = db.get_preference("onboarding_completed").ok().flatten().as_deref() == Some("true");
            let ai_enabled = db.is_ai_enabled().unwrap_or(true);
            if onboarded && ai_enabled {
                let db_for_warmup = db.clone();
                tauri::async_runtime::spawn(async move {
                    services::ai::AiService::warmup_from_db(&db_for_warmup).await;
                });
            } else {
                eprintln!("[startup] AI warmup skipped (onboarded={onboarded}, ai_enabled={ai_enabled})");
            }

            eprintln!(
                "[startup] [{:.0}ms] setup complete",
                setup_start.elapsed().as_secs_f64() * 1000.0
            );

            Ok(())
        })
        .on_page_load(|webview, payload| {
            // Show the window once the HTML page finishes loading in the webview.
            // The window starts hidden ("visible": false in tauri.conf.json) to
            // avoid the transparent-blank flash while the webview initialises.
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let _ = webview.window().show();
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // Last window closed — shut down background tasks so the process exits cleanly.
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    state.scheduler.stop();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::add_account,
            commands::accounts::test_imap_connection,
            commands::accounts::add_imap_account,
            commands::accounts::get_imap_settings,
            commands::accounts::update_imap_credentials,
            commands::accounts::list_accounts,
            commands::accounts::remove_account,
            commands::accounts::reauthenticate_account,
            commands::accounts::reorder_accounts,
            commands::accounts::set_account_enabled,
            commands::accounts::update_account_sync_from,
            commands::accounts::get_account_settings,
            commands::accounts::set_account_settings,
            commands::accounts::get_available_categories,
            commands::emails::get_emails,
            commands::emails::get_folders,
            commands::emails::create_folder,
            commands::emails::rename_folder,
            commands::emails::delete_folder,
            commands::emails::move_email,
            commands::emails::get_thread,
            commands::emails::get_email_body,
            commands::emails::mark_as_read,
            commands::emails::delete_email,
            commands::emails::send_reply,
            commands::emails::send_new_email,
            commands::emails::generate_draft,
            commands::emails::generate_new_draft,
            commands::translation::detect_email_language,
            commands::translation::translate_email,
            commands::translation::translate_compose_text,
            commands::emails::redownload_email,
            commands::emails::start_redownload_empty_emails,
            commands::emails::sync_account,
            commands::emails::start_sync_account,
            commands::emails::start_resync_mailbox,
            commands::emails::get_email_inbox_position,
            commands::emails::autocomplete_senders,
            commands::emails::autocomplete_recipients,
            commands::emails::get_email_by_id,
            commands::emails::get_email_count,
            commands::emails::get_sync_status,
            commands::search::search_emails,
            commands::search::generate_embeddings,
            commands::search::start_generate_embeddings,
            commands::search::regenerate_embeddings,
            commands::search::start_regenerate_embeddings,
            commands::search::rebuild_fts_index,
            commands::search::get_pending_embeddings_count,
            commands::search::list_ollama_models,
            commands::search::get_ai_model,
            commands::search::set_ai_model,
            commands::notifications::ensure_notification_permission,
            commands::calendar::get_calendar_events,
            commands::calendar::get_calendars,
            commands::calendar::set_calendar_visible,
            commands::calendar::create_calendar_event,
            commands::calendar::delete_calendar_event,
            commands::calendar::get_calendar_invite,
            commands::calendar::rsvp_calendar_invite,
            commands::calendar::sync_calendar_now,
            commands::preferences::get_pref,
            commands::preferences::set_pref,
            commands::preferences::get_auto_n_ctx,
            commands::preferences::show_main_window,
            commands::preferences::get_system_locale,
            commands::prompts::list_prompts,
            commands::prompts::set_prompt,
            commands::prompts::reset_prompt,
            commands::security::has_main_password,
            commands::security::set_main_password,
            commands::security::verify_main_password,
            commands::security::remove_main_password,
            commands::contacts::get_contacts,
            commands::contacts::list_contacts,
            commands::contacts::get_contact_detail,
            commands::contacts::list_contacts_by_company,
            commands::drafts::list_drafts,
            commands::drafts::get_draft,
            commands::drafts::refresh_drafts,
            commands::drafts::list_draft_attachments,
            commands::drafts::save_draft,
            commands::drafts::send_draft,
            commands::drafts::delete_draft,
            commands::filters::refresh_filter_stats,
            commands::filters::get_saved_suggestions,
            commands::filters::get_filtered_emails,
            commands::filters::get_attachment_ext_stats,
            commands::filters::get_filter_prefs,
            commands::filters::pin_filter,
            commands::filters::remove_filter,
            commands::filters::delete_filter_pref,
            commands::trusted_senders::add_trusted_sender,
            commands::trusted_senders::remove_trusted_sender,
            commands::trusted_senders::list_trusted_senders,
            commands::trusted_senders::is_sender_trusted,
            commands::attachments::create_attachment_rule,
            commands::attachments::update_attachment_rule,
            commands::attachments::delete_attachment_rule,
            commands::attachments::list_attachment_rules,
            commands::attachments::count_attachments_for_rule,
            commands::attachments::get_attachments,
            commands::attachments::get_attachments_for_email,
            commands::attachments::count_attachments,
            commands::attachments::get_attachment,
            commands::attachments::get_attachment_tags,
            commands::attachments::get_attachment_file_path,
            commands::attachments::get_attachment_data,
            commands::attachments::bulk_download_attachments,
            commands::attachments::save_attachment_to_downloads,
            commands::attachments::reveal_in_finder,
            commands::attachments::apply_rule_retroactively,
            commands::attachments::open_attachment_externally,
            commands::attachments::get_email_attachment_metas,
            commands::attachments::reextract_email_attachments,
            commands::attachments::fetch_email_attachment_bytes,
            commands::attachments::open_email_attachment_meta,
            commands::classification::get_classification_config,
            commands::classification::set_classification_config,
            commands::classification::classify_previous_emails,
            commands::classification::reclassify_all_emails,
            commands::classification::get_email_tags,
            commands::classification::get_email_tags_batch,
            commands::classification::count_unclassified_emails,
            commands::classification::get_tag_priorities,
            commands::classification::list_classification_rules,
            commands::classification::create_classification_rule,
            commands::classification::update_classification_rule,
            commands::classification::delete_classification_rule,
            commands::junk::get_junk_verdicts,
            commands::junk::set_junk_feedback,
            commands::junk::backfill_junk_scores,
            commands::junk::report_junk_to_provider,
            commands::junk::get_junk_config,
            commands::junk::set_junk_config,
            commands::junk::get_junk_stats,
            commands::ai_config::get_ai_config,
            commands::ai_config::set_ai_config,
            commands::ai_config::get_ai_usage,
            commands::ai_config::reset_ai_usage,
            commands::ai_config::list_ai_models,
            commands::ai_config::list_ai_embedding_models,
            commands::ai_config::get_embeddings_config,
            commands::ai_config::set_embeddings_config,
            commands::ai_config::check_ai_available,
            commands::ai_config::test_ai_provider,
            commands::ai_models::list_catalog_models,
            commands::ai_models::list_local_models,
            commands::ai_models::delete_local_model,
            commands::ai_models::start_model_download,
            commands::ai_models::link_local_model,
            commands::ai_models::cancel_model_download,
            commands::chat::list_chat_conversations,
            commands::chat::create_chat_conversation,
            commands::chat::create_chat_conversation_with_thread,
            commands::chat::rename_chat_conversation,
            commands::chat::delete_chat_conversation,
            commands::chat::get_chat_messages,
            commands::chat::send_chat_message,
            commands::chat::prewarm_chat,
            commands::memory::list_pending_tasks,
            commands::memory::get_task_counts,
            commands::memory::create_pending_task,
            commands::memory::update_pending_task_status,
            commands::memory::list_open_threads,
            commands::memory::list_memory_facts,
            commands::memory::promote_memory_fact,
            commands::memory::retire_memory_fact,
            commands::memory::update_memory_fact,
            commands::memory::delete_memory_fact,
            commands::memory::get_memory_counts,
            commands::memory::get_memory_config,
            commands::memory::set_memory_config,
            commands::memory::get_task_config,
            commands::memory::set_task_config,
            commands::memory::get_memory_backfill_status,
            commands::memory::start_memory_backfill,
            commands::memory::cancel_memory_backfill,
            commands::memory::reset_memory_extraction,
            commands::memory::get_task_backfill_status,
            commands::memory::start_task_backfill,
            commands::memory::cancel_task_backfill,
            commands::memory::reset_task_extraction,
            commands::memory::run_memory_consolidation,
            commands::dashboard::get_dashboard_stats,
            commands::dashboard::refresh_server_total,
            commands::dashboard::get_queue_state,
            commands::dashboard::get_storage_stats,
            commands::system::detect_ai_capability,
            commands::system::is_rosetta_translated,
            commands::system::get_available_update,
            commands::system::get_build_info,
            commands::connectivity::is_online,
            commands::lenses::list_lenses,
            commands::lenses::get_lens,
            commands::lenses::create_lens,
            commands::lenses::update_lens,
            commands::lenses::delete_lens,
            commands::lenses::duplicate_lens,
            commands::lenses::list_lens_templates,
            commands::lenses::create_lens_from_template,
            commands::lenses::get_lens_rows,
            commands::lenses::update_lens_row_override,
            commands::lenses::exclude_lens_row,
            commands::lenses::include_lens_row,
            commands::lenses::run_lens,
            commands::lenses::cancel_lens_run,
            commands::lenses::get_lens_status,
            commands::lenses::list_lens_runs,
            commands::lenses::reextract_lens_row,
            commands::lenses::preview_lens_extraction,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app_handle, event| {
            // Built + run with a callback (rather than plain `Builder::run`)
            // purely to reach `RunEvent::Exit`. On macOS `-[NSApplication
            // terminate:]` calls `exit()` itself, so the event loop never
            // returns and any cleanup written after `run(...)` is dead code —
            // this callback is the last point we control.
            if matches!(event, tauri::RunEvent::Exit) {
                on_exit();
            }
        });
}

/// Last-chance cleanup, run from `RunEvent::Exit`.
///
/// Releases the embedded AI runtime, then leaves the process without running
/// C++ static destructors.
///
/// Both halves matter. ggml registers every Metal buffer in a residency set
/// and its device-list destructor asserts the set is empty at `exit()`
/// (`ggml_metal_rsets_free`), aborting the process otherwise — which is why a
/// plain quit produced a `SIGABRT` crash report for anyone using the embedded
/// provider. `shutdown_local_ai` removes the cause by dropping the model and
/// waiting for the inference thread; `_exit` is the backstop, because the
/// bundled *embedding* runtime and any future vendored at-exit hook can abort
/// the same way and a crash on quit is never worth the destructors we skip.
///
/// Skipping them is safe here: SQLite is in WAL mode (a WAL left behind is
/// recovered on next open, and the backup thread writes to a temp file that is
/// only renamed on success), and no other shutdown work is registered via
/// `atexit`.
#[cfg(feature = "desktop")]
fn on_exit() {
    services::ai::shutdown_and_exit(0)
}

// ── Fatal startup errors ──────────────────────────────────────────────────────

/// What the user should actually do about a fatal startup failure.
#[cfg(feature = "desktop")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupRemedy {
    /// Default assumption: a full startup disk or a file-permission problem.
    Environment,
    /// The data directory was written by a newer build than this one, so the
    /// remedy is to change the *app*, not the disk.
    DataFromNewerVersion,
}

/// Classify a fatal startup detail so the copy names a remedy that can work.
///
/// Refinery reports a binary/schema mismatch two ways: `MissingVersion` when
/// the database records a migration this binary does not embed, and
/// `DivergentVersion` when the same version has different content. Both mean
/// this install is older than its data — typically because an uninstall left
/// the data directory behind and an older build was then installed over it.
/// Refinery is right to refuse: older code cannot read a newer schema.
#[cfg(feature = "desktop")]
fn classify_startup_failure(detail: &str) -> StartupRemedy {
    let detail = detail.to_lowercase();
    if detail.contains("is missing from the filesystem") || detail.contains("is different than filesystem one") {
        StartupRemedy::DataFromNewerVersion
    } else {
        StartupRemedy::Environment
    }
}

/// Build the user-facing message for a fatal startup failure. Kept pure so it
/// can be unit-tested without exiting the process.
///
/// `app_data_dir` is optional because the earliest call sites run before it has
/// been resolved; when known it is named so "start fresh" is actionable.
#[cfg(feature = "desktop")]
fn format_startup_error(stage: &str, detail: &str, app_data_dir: Option<&std::path::Path>) -> String {
    let remedy = match classify_startup_failure(detail) {
        StartupRemedy::Environment => "This is often caused by a full startup disk or a file-permission \
             problem. Free up disk space and try opening EmailOps again."
            .to_string(),
        StartupRemedy::DataFromNewerVersion => {
            // Only offer the destructive escape hatch when we can say exactly
            // what to delete.
            let start_fresh = match app_data_dir {
                Some(dir) => format!(
                    "\n\nTo start fresh instead, delete this folder — that removes locally stored \
                     mail and settings, and accounts have to be added again:\n{}",
                    dir.display()
                ),
                None => String::new(),
            };
            format!(
                "Its local data was created by a newer version of EmailOps, which this version \
                 cannot read. Installing the latest version of EmailOps will fix this.{start_fresh}"
            )
        }
    };
    format!("EmailOps could not start because it failed to {stage}.\n\n{remedy}\n\nDetails: {detail}")
}

/// Abort startup gracefully: log the reason, show the user a readable dialog,
/// then exit with a non-zero status. Unlike a panic, `std::process::exit` does
/// not trip the `panic_cannot_unwind` abort path when called from inside the
/// macOS launch callback.
///
/// `app_data_dir` is `None` at the earliest call sites (resolving it is itself
/// one of the steps that can fail); the crash report then falls back to the
/// system temp directory — see `util::startup_dialog`.
#[cfg(feature = "desktop")]
fn fatal_startup_error(stage: &str, detail: &str, app_data_dir: Option<&std::path::Path>) -> ! {
    let message = format_startup_error(stage, detail, app_data_dir);
    eprintln!("[startup][fatal] {message}");
    // The frontend window is never shown on this path, so the logger emission
    // is mostly for parity; the dialog and crash report are what the user
    // actually sees.
    services::logger::log("error", "system", message.clone());
    util::startup_dialog::show(app_data_dir, &message);
    std::process::exit(1);
}

#[cfg(test)]
#[cfg(feature = "desktop")]
mod startup_error_tests {
    use super::{classify_startup_failure, format_startup_error, StartupRemedy};
    use std::path::Path;

    #[test]
    fn message_names_the_stage_and_includes_details() {
        let msg = format_startup_error("open its local database", "disk I/O error (SQLITE_FULL)", None);
        assert!(
            msg.contains("open its local database"),
            "stage must appear in the message"
        );
        assert!(
            msg.contains("disk I/O error (SQLITE_FULL)"),
            "underlying detail must appear"
        );
    }

    #[test]
    fn message_hints_at_disk_space() {
        // The most common real cause is a full disk — the user-facing copy must
        // point them at it so they can self-recover. The phrase spans a `\`
        // line-continuation, so it also pins that the copy renders with the
        // right spacing.
        let msg = format_startup_error("locate its data directory", "permission denied", None);
        assert!(
            msg.contains("a file-permission problem"),
            "disk/permission copy must render contiguously: {msg}"
        );
    }

    #[test]
    fn classifies_refinery_mismatches_as_stale_install() {
        // Refinery reports a binary/schema mismatch two ways. Both mean the
        // data directory outlived a newer install.
        for detail in [
            "DB migration failed: migration V18__email_headers is missing from the filesystem",
            "DB migration failed: applied migration V18__email_headers is different than filesystem one V18__other",
        ] {
            assert_eq!(
                classify_startup_failure(detail),
                StartupRemedy::DataFromNewerVersion,
                "{detail} must be classified as a version mismatch"
            );
        }
    }

    #[test]
    fn classifies_everything_else_as_environmental() {
        for detail in ["disk I/O error (SQLITE_FULL)", "permission denied", "unable to open"] {
            assert_eq!(
                classify_startup_failure(detail),
                StartupRemedy::Environment,
                "{detail} must keep the disk/permission copy"
            );
        }
    }

    #[test]
    fn stale_install_blames_the_app_version_not_the_disk() {
        // Regression: installing a build that predates V018 over a data
        // directory a newer build had already migrated produced the generic
        // "free up disk space" copy, sending the user after a cause that
        // cannot possibly be it.
        let msg = format_startup_error(
            "open its local database",
            "DB migration failed: migration V18__email_headers is missing from the filesystem",
            Some(Path::new("/data/EmailOps")),
        );
        // The phrase spans a `\` line-continuation, pinning the rendered
        // spacing as well as the wording.
        assert!(
            msg.contains("newer version of EmailOps, which this version cannot read"),
            "must say the data came from a newer version: {msg}"
        );
        assert!(
            !msg.to_lowercase().contains("free up disk space"),
            "must not blame disk space: {msg}"
        );
    }

    #[test]
    fn stale_install_names_the_data_folder_to_remove() {
        // "Start fresh" is only actionable if the user is told what to delete.
        let msg = format_startup_error(
            "open its local database",
            "DB migration failed: migration V18__email_headers is missing from the filesystem",
            Some(Path::new("/data/EmailOps")),
        );
        assert!(msg.contains("/data/EmailOps"), "must name the data folder: {msg}");
        assert!(
            msg.contains("removes locally stored mail and settings"),
            "the destructive warning must render contiguously: {msg}"
        );
    }

    #[test]
    fn stale_install_copy_survives_an_unknown_data_folder() {
        // The data directory is unknown at the earliest call sites; the advice
        // must still make sense without a path to name.
        let msg = format_startup_error(
            "open its local database",
            "DB migration failed: migration V18__email_headers is missing from the filesystem",
            None,
        );
        assert!(msg.to_lowercase().contains("newer version"), "{msg}");
        assert!(
            !msg.contains("Data folder:"),
            "must not print an empty data folder line: {msg}"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "desktop")]
mod env_loading_tests {
    use super::should_load_dotenv_with;

    #[test]
    fn debug_builds_load_dotenv_without_override() {
        assert!(should_load_dotenv_with(true, None));
    }

    #[test]
    fn release_builds_skip_dotenv_by_default() {
        assert!(!should_load_dotenv_with(false, None));
    }

    #[test]
    fn release_builds_allow_explicit_override() {
        assert!(should_load_dotenv_with(false, Some("1")));
        assert!(should_load_dotenv_with(false, Some("true")));
        assert!(should_load_dotenv_with(false, Some("on")));
    }

    #[test]
    fn release_builds_reject_falsey_override() {
        assert!(!should_load_dotenv_with(false, Some("0")));
        assert!(!should_load_dotenv_with(false, Some("false")));
        assert!(!should_load_dotenv_with(false, Some("")));
    }
}

#[cfg(test)]
#[cfg(feature = "desktop")]
mod appstate_tests {
    use std::sync::Arc;

    use super::*;

    fn test_db() -> Arc<Database> {
        Arc::new(Database::new_for_testing().expect("in-memory test DB"))
    }

    #[test]
    fn for_testing_constructs_without_panic() {
        let db = test_db();
        let state = AppState::for_testing(db.clone());
        // Can still access the DB through AppState.
        let accounts = state.db.list_accounts().expect("list_accounts");
        assert!(accounts.is_empty());
    }

    #[test]
    fn for_testing_uses_fake_dispatcher() {
        let db = test_db();
        let state = AppState::for_testing(db);
        // FakeDispatcher.recorded() starts empty.
        let recorded = state.dispatcher.recorded();
        assert!(recorded.is_empty());
    }

    #[tokio::test]
    async fn for_testing_dispatcher_records_enqueued_tasks() {
        let db = test_db();
        let state = AppState::for_testing(db);

        state
            .dispatcher
            .dispatch(
                services::background_tasks::BackgroundTask::GenerateDraft {
                    email_id: "e-1".into(),
                    request_id: "r-1".into(),
                },
                Box::new(|| Box::pin(async { panic!("must not run") })),
            )
            .await;

        let recorded = state.dispatcher.recorded();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(
            &recorded[0],
            services::background_tasks::BackgroundTask::GenerateDraft { email_id, .. }
                if email_id == "e-1"
        ));
    }
}
