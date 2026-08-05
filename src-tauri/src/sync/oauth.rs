use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, ClientSecret, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, RefreshToken, TokenResponse, TokenUrl,
};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use crate::models::error::{AppError, Result};
use crate::models::OAuthTokens;

// Gmail OAuth configuration
// Credentials can be supplied at runtime for development, or bundled into
// release builds at compile time for installed desktop app usage.
const GMAIL_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GMAIL_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const BUNDLED_GMAIL_CLIENT_ID: Option<&str> = option_env!("EMAILOPS_GMAIL_CLIENT_ID");
const BUNDLED_GMAIL_CLIENT_SECRET: Option<&str> = option_env!("EMAILOPS_GMAIL_CLIENT_SECRET");
// Separate, iOS-type Google client. Public (PKCE, no secret) with a
// reversed-client-ID redirect — see `mobile_redirect_uri`.
const BUNDLED_GMAIL_IOS_CLIENT_ID: Option<&str> = option_env!("EMAILOPS_GMAIL_IOS_CLIENT_ID");

// NOTE: `gmail.modify` includes full read access, so `gmail.readonly` must NOT
// be added here — Google's restricted-scope verification requires the narrowest
// scope set, and the declared scopes must match this list exactly.
const GMAIL_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.send",
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/calendar.events",
    // Enumerating the account's other and shared calendars (and their colours)
    // needs `calendarList.list`, which `calendar.events` does not cover.
    // This is the narrowest scope that grants it — do NOT widen to
    // `calendar.readonly`, which also grants reading every calendar's contents.
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
];

// Microsoft Graph (Outlook / Office 365) OAuth configuration.
// Uses the `common` tenant endpoint so both personal (outlook.com, hotmail.com,
// live.com) and work/school (Microsoft 365) accounts can sign in with the same
// Azure AD app registration. `offline_access` is required to receive a refresh
// token; without it Graph hands out 1-hour access tokens with no way to renew.
const OUTLOOK_AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const OUTLOOK_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const BUNDLED_OUTLOOK_CLIENT_ID: Option<&str> = option_env!("EMAILOPS_OUTLOOK_CLIENT_ID");
const BUNDLED_OUTLOOK_CLIENT_SECRET: Option<&str> = option_env!("EMAILOPS_OUTLOOK_CLIENT_SECRET");

const OUTLOOK_SCOPES: &[&str] = &[
    "offline_access",
    "User.Read",
    "Mail.ReadWrite",
    "Mail.Send",
    "Calendars.ReadWrite",
];

/// Suffix every Google OAuth client ID carries.
const GOOGLE_CLIENT_ID_SUFFIX: &str = ".apps.googleusercontent.com";

/// Scheme prefix Google expects for an iOS client's custom-scheme redirect.
const GOOGLE_REVERSED_SCHEME_PREFIX: &str = "com.googleusercontent.apps.";

/// Turn a Google iOS client ID into the custom URI scheme it must redirect to.
///
/// `123-abc.apps.googleusercontent.com` → `com.googleusercontent.apps.123-abc`
///
/// This is the "reversed client ID" Google's iOS guidance refers to. It is the
/// value that must appear in the app's `CFBundleURLTypes` *and* in the
/// `redirect_uri` sent with the authorization request; a mismatch surfaces only
/// as `redirect_uri_mismatch` after a full trip through the consent screen,
/// which is why this is a pure function with its own tests rather than a
/// `format!` inlined at the call site.
///
/// Returns `None` for anything that is not a Google client ID — a desktop
/// client, an Azure GUID, an empty string, or an already-reversed scheme —
/// so a misconfiguration fails loudly at startup instead of producing a
/// redirect URI that can never match.
pub fn reversed_client_id(client_id: &str) -> Option<String> {
    if client_id.starts_with(GOOGLE_REVERSED_SCHEME_PREFIX) {
        return None;
    }
    let body = client_id.strip_suffix(GOOGLE_CLIENT_ID_SUFFIX)?;
    if body.is_empty() {
        return None;
    }
    Some(format!("{GOOGLE_REVERSED_SCHEME_PREFIX}{body}"))
}

/// The app's own custom URI scheme, registered in `CFBundleURLTypes`. Matches
/// `identifier` in tauri.conf.json.
const APP_URI_SCHEME: &str = "com.emailops.app";

/// Path component appended to every mobile redirect URI. Arbitrary, but it must
/// match byte-for-byte between the provider registration and the auth request.
const MOBILE_REDIRECT_PATH: &str = "oauth2redirect";

/// Redirect URI to use on a mobile build, where there is no loopback listener.
///
/// The two providers differ in kind, not just in value:
///
/// * **Google** derives the scheme from the client ID itself (the reversed
///   client ID), so a wrong or desktop client ID cannot produce a usable
///   redirect — hence the `Option`. Note the single slash: Google's convention
///   is `scheme:/path`, and `scheme://path` is a different URI that will be
///   rejected as a mismatch.
/// * **Azure** lets a public client register any custom scheme, and the client
///   ID plays no part, so the app's own scheme is used and this cannot fail.
///
/// Returns `None` only when Google's client ID is unusable, which callers
/// surface as a configuration error rather than attempting a doomed round trip.
pub fn mobile_redirect_uri(provider: &str, client_id: &str) -> Option<String> {
    match provider {
        "outlook" => Some(format!("{APP_URI_SCHEME}://{MOBILE_REDIRECT_PATH}")),
        // Gmail, and the same fallback `for_provider` uses for unknown values.
        _ => reversed_client_id(client_id).map(|scheme| format!("{scheme}:/{MOBILE_REDIRECT_PATH}")),
    }
}

pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    /// Host used in the loopback redirect URI. Google accepts both "127.0.0.1"
    /// and "localhost" for desktop apps, but Microsoft's consumer endpoint
    /// (login.live.com) only special-cases `http://localhost` with dynamic ports.
    pub redirect_host: &'static str,
    /// Canonical provider key ("gmail" / "outlook"). Needed because the mobile
    /// redirect URI is derived per provider and `display_name` is prose meant
    /// for error messages, not a stable identifier to match on.
    pub provider_key: &'static str,
    /// Human-readable provider identifier used in error messages (e.g. "Gmail",
    /// "Outlook"). Avoids surfacing "Gmail OAuth credentials missing" when the
    /// user is actually trying to add an Outlook account.
    pub display_name: &'static str,
    /// Env var names reported in the missing-credentials error so operators
    /// know which variables to set for this provider.
    pub env_var_hint: &'static str,
}

impl OAuthConfig {
    /// Build OAuth config for the given provider string.
    /// Supported: "gmail", "outlook". Unknown values fall back to Gmail.
    pub fn for_provider(provider: &str) -> Self {
        match provider {
            "gmail" => Self::gmail(),
            "outlook" => Self::outlook(),
            _ => Self::gmail(), // fallback
        }
    }

    pub fn gmail() -> Self {
        // Google issues a distinct client per platform and they are not
        // interchangeable: the desktop client is confidential (carries a
        // secret) and registers a loopback redirect that iOS cannot use, while
        // the iOS client is public and derives its redirect from the client ID.
        // Handing the desktop secret to an App Store binary would also ship an
        // extractable secret. Leaving the secret empty here is what flips
        // `is_public_client` on and enables PKCE further down.
        let (client_id, client_secret) = if is_mobile_target() {
            (
                runtime_or_bundled_env("EMAILOPS_GMAIL_IOS_CLIENT_ID", BUNDLED_GMAIL_IOS_CLIENT_ID),
                String::new(),
            )
        } else {
            (
                runtime_or_bundled_env("EMAILOPS_GMAIL_CLIENT_ID", BUNDLED_GMAIL_CLIENT_ID),
                runtime_or_bundled_env("EMAILOPS_GMAIL_CLIENT_SECRET", BUNDLED_GMAIL_CLIENT_SECRET),
            )
        };

        Self {
            client_id,
            client_secret,
            auth_url: GMAIL_AUTH_URL.to_string(),
            token_url: GMAIL_TOKEN_URL.to_string(),
            scopes: GMAIL_SCOPES.iter().map(|s| s.to_string()).collect(),
            redirect_host: "127.0.0.1",
            provider_key: "gmail",
            display_name: "Gmail",
            env_var_hint: "EMAILOPS_GMAIL_CLIENT_ID and EMAILOPS_GMAIL_CLIENT_SECRET",
        }
    }

    pub fn outlook() -> Self {
        let client_id = runtime_or_bundled_env("EMAILOPS_OUTLOOK_CLIENT_ID", BUNDLED_OUTLOOK_CLIENT_ID);
        let client_secret = runtime_or_bundled_env("EMAILOPS_OUTLOOK_CLIENT_SECRET", BUNDLED_OUTLOOK_CLIENT_SECRET);

        Self {
            client_id,
            client_secret,
            auth_url: OUTLOOK_AUTH_URL.to_string(),
            token_url: OUTLOOK_TOKEN_URL.to_string(),
            scopes: OUTLOOK_SCOPES.iter().map(|s| s.to_string()).collect(),
            // Microsoft special-cases `http://localhost` for native-client apps to permit
            // dynamic ports. `http://127.0.0.1:<port>` must be registered exactly and is
            // rejected by login.live.com when it isn't.
            redirect_host: "localhost",
            provider_key: "outlook",
            display_name: "Outlook",
            // Secret is optional: Azure AD public-client / native-app
            // registrations use PKCE and reject any client_secret (AADSTS90023).
            // Only the client ID is strictly required.
            env_var_hint:
                "EMAILOPS_OUTLOOK_CLIENT_ID (EMAILOPS_OUTLOOK_CLIENT_SECRET is optional, only for confidential clients)",
        }
    }
}

fn runtime_or_bundled_env(key: &str, bundled: Option<&str>) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| bundled.map(str::to_string).filter(|value| !value.trim().is_empty()))
        .unwrap_or_default()
}

/// True when this build targets a mobile OS, where the loopback redirect and
/// the `open` crate are both unavailable.
///
/// A `const fn` over `cfg!` rather than `#[cfg]` blocks at each branch: both
/// arms then type-check on every host, so a desktop `cargo check` still
/// compiles the mobile path instead of silently rotting it.
pub const fn is_mobile_target() -> bool {
    cfg!(any(target_os = "ios", target_os = "android"))
}

/// Bridge for the mobile OAuth callback.
///
/// On mobile the authorization code does not come back over a socket we own —
/// the OS hands the app a `com.googleusercontent.apps.…:/oauth2redirect` URL
/// through the deep-link plugin, on a completely different call path from the
/// one awaiting it. This module parks a oneshot sender so the deep-link
/// listener installed in `lib.rs` can wake `start_oauth_flow`.
///
/// It is an installed global for the same reason `logger`, `events` and
/// `keychain` are: the alternative is threading an `AppHandle` through
/// `services::accounts::{add_account, reauthenticate_account}` and their
/// command wrappers, purely so one leaf function can reach the OS.
pub mod mobile_callback {
    use std::sync::{Mutex, PoisonError};

    use tokio::sync::oneshot;

    use crate::services::app_handle::AppHandle;

    static APP: Mutex<Option<AppHandle>> = Mutex::new(None);
    static PENDING: Mutex<Option<oneshot::Sender<String>>> = Mutex::new(None);

    /// Record the handle used to open the system browser. Called once at startup.
    pub fn install(app: AppHandle) {
        *APP.lock().unwrap_or_else(PoisonError::into_inner) = Some(app);
    }

    pub(super) fn app() -> Option<AppHandle> {
        APP.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Arm the bridge before the browser is opened, so a callback that arrives
    /// quickly cannot race ahead of the receiver.
    pub(super) fn arm() -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        // Replacing any previous sender deliberately cancels an abandoned
        // sign-in attempt rather than leaving it to receive this one's code.
        *PENDING.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        rx
    }

    /// Deliver a deep-link callback URL. Called from the `on_open_url` listener.
    /// A URL arriving with nothing armed is dropped — that is a cold-start
    /// launch from a stale link, not an in-flight sign-in.
    pub fn deliver(url: String) {
        if let Some(tx) = PENDING.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = tx.send(url);
        }
    }
}

/// How long to wait for a mobile sign-in to come back before giving up.
///
/// Longer than the desktop loopback deadline because the mobile flow leaves the
/// app entirely: the user is switched to Safari, may hit a password manager,
/// 2FA, or an account chooser, and the app is suspended in the background the
/// whole time. Five minutes is generous enough not to punish a slow 2FA and
/// short enough that an abandoned attempt eventually releases the bridge.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// Open a URL in the system browser on a mobile target.
///
/// Deliberately not the `open` crate: it has no iOS backend and fails with
/// `ENOENT`. The shell plugin does implement iOS, so it is the one path that
/// actually reaches Safari from a Tauri iOS build.
#[cfg(feature = "desktop")]
fn open_url_on_mobile(app: &crate::services::app_handle::AppHandle, url: &str) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::OAuthError(format!("Failed to open browser: {e}")))
}

/// Headless builds (`--no-default-features`) have no shell plugin and no
/// webview; nothing can open a browser, so this is a hard error rather than a
/// silent no-op that would hang on the callback.
#[cfg(not(feature = "desktop"))]
fn open_url_on_mobile(_app: &crate::services::app_handle::AppHandle, _url: &str) -> Result<()> {
    Err(AppError::OAuthError(
        "Cannot open a browser in a headless build.".to_string(),
    ))
}

/// Pull `code` out of a full redirect URL, verifying `state` first.
///
/// The desktop path parses a raw HTTP request line; on mobile the OS hands us
/// the URL directly, so only the query string needs unpicking. State is checked
/// before the code is returned — a callback whose state does not match is a
/// CSRF attempt or a crossed-over stale sign-in, never something to exchange.
fn extract_code_from_redirect_url(redirect_url: &str, expected_state: &str) -> Result<String> {
    let parsed =
        url::Url::parse(redirect_url).map_err(|e| AppError::OAuthError(format!("Malformed callback URL: {e}")))?;

    let mut code = None;
    let mut state = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => {
                return Err(AppError::OAuthError(format!("Authorization denied: {value}")));
            }
            _ => {}
        }
    }

    if state.as_deref() != Some(expected_state) {
        return Err(AppError::OAuthError(
            "OAuth state mismatch — ignoring callback.".to_string(),
        ));
    }

    code.ok_or_else(|| AppError::OAuthError("Callback URL carried no authorization code.".to_string()))
}

pub async fn start_oauth_flow(config: &OAuthConfig) -> Result<OAuthTokens> {
    if config.client_id.is_empty() {
        return Err(AppError::OAuthError(format!(
            "Missing {} OAuth client ID. Set {}.",
            config.display_name, config.env_var_hint
        )));
    }

    // Public client = no client secret. Azure AD public-client apps reject any
    // client_secret (AADSTS90023) and require PKCE. If a secret is provided we
    // treat this as a confidential client (Google's desktop-app flow).
    let is_public_client = config.client_secret.is_empty();

    // Two ways the authorization code can come back:
    //
    //  * desktop — we bind an ephemeral loopback port and read the callback
    //    off the socket ourselves;
    //  * mobile — there is no loopback redirect target and no browser-launcher
    //    binary, so the OS delivers a custom-scheme URL to the deep-link
    //    plugin and `mobile_callback` hands it over.
    let (listener, deep_link_rx, redirect_url) = if is_mobile_target() {
        let redirect_url = mobile_redirect_uri(config.provider_key, &config.client_id).ok_or_else(|| {
            AppError::OAuthError(format!(
                "{} is not configured for mobile sign-in: its client ID does not yield a redirect scheme. \
                 An iOS build needs the iOS-type OAuth client, not the desktop one.",
                config.display_name
            ))
        })?;
        // Arm before opening the browser so a fast callback cannot outrun us.
        (None, Some(mobile_callback::arm()), redirect_url)
    } else {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| AppError::OAuthError(format!("Failed to bind port: {}", e)))?;
        let port = listener
            .local_addr()
            .map_err(|e| AppError::OAuthError(format!("Failed to read bound port: {}", e)))?
            .port();
        let redirect_url = format!("http://{}:{}", config.redirect_host, port);
        (Some(listener), None, redirect_url)
    };

    // Create OAuth client
    let mut client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(AuthUrl::new(config.auth_url.clone()).map_err(|e| AppError::OAuthError(e.to_string()))?)
        .set_token_uri(TokenUrl::new(config.token_url.clone()).map_err(|e| AppError::OAuthError(e.to_string()))?)
        .set_redirect_uri(RedirectUrl::new(redirect_url).map_err(|e| AppError::OAuthError(e.to_string()))?);
    if !is_public_client {
        client = client.set_client_secret(ClientSecret::new(config.client_secret.clone()));
    }

    // Generate authorization URL (with PKCE for public clients — Azure AD
    // enforces PKCE when the app registration has no client secret).
    let (pkce_challenge, pkce_verifier) = if is_public_client {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        (Some(challenge), Some(verifier))
    } else {
        (None, None)
    };

    let mut auth_request = client.authorize_url(oauth2::CsrfToken::new_random);
    for scope in &config.scopes {
        auth_request = auth_request.add_scope(oauth2::Scope::new(scope.clone()));
    }
    if let Some(challenge) = pkce_challenge {
        auth_request = auth_request.set_pkce_challenge(challenge);
    }
    let (auth_url, csrf_token) = auth_request.url();

    // Open browser for user authorization. `that_detached` (not `that`) is
    // required here: `open::that` blocks the calling thread until the
    // launched application *exits* — on macOS `open -a` hands off via
    // LaunchServices and returns immediately, masking this, but on Linux
    // `open::that` execs the browser directly and waits on it, so this
    // Tauri command would never resolve until the user closed every window
    // of their browser. `that_detached` launches and returns immediately.
    let code = match (listener, deep_link_rx) {
        // Desktop: launch via the `open` crate, then read the loopback socket.
        (Some(listener), _) => {
            open::that_detached(auth_url.as_str())
                .map_err(|e| AppError::OAuthError(format!("Failed to open browser: {}", e)))?;
            wait_for_callback(listener, csrf_token.secret())?
        }
        // Mobile: the `open` crate has no backend here — it fails with ENOENT
        // ("Failed to open browser: No such file or directory"), which is
        // exactly what an iOS sign-in used to die on. Route through the shell
        // plugin, which does have an iOS implementation, and then await the
        // deep-link callback instead of a socket.
        (None, Some(rx)) => {
            let app = mobile_callback::app().ok_or_else(|| {
                AppError::OAuthError(
                    "Mobile OAuth bridge was never installed — sign-in cannot open a browser.".to_string(),
                )
            })?;
            open_url_on_mobile(&app, auth_url.as_str())?;

            let redirect_url = tokio::time::timeout(CALLBACK_TIMEOUT, rx)
                .await
                .map_err(|_| AppError::OAuthError("Timed out waiting for sign-in to complete.".to_string()))?
                .map_err(|_| AppError::OAuthError("Sign-in was cancelled.".to_string()))?;

            extract_code_from_redirect_url(&redirect_url, csrf_token.secret())?
        }
        // Unreachable: the tuple is constructed as exactly one of the two arms.
        (None, None) => {
            return Err(AppError::OAuthError(
                "No OAuth callback channel was set up — this is a bug.".to_string(),
            ))
        }
    };

    // Exchange code for tokens
    let http_client = reqwest::Client::new();
    let mut exchange = client.exchange_code(AuthorizationCode::new(code));
    if let Some(verifier) = pkce_verifier {
        // PkceCodeVerifier is move-only; re-wrap its secret so we can hand it in.
        exchange = exchange.set_pkce_verifier(PkceCodeVerifier::new(verifier.secret().clone()));
    }
    let token_result = exchange
        .request_async(&http_client)
        .await
        .map_err(|e| AppError::OAuthError(format!("Token exchange failed: {}", e)))?;

    let expires_at = token_result
        .expires_in()
        .map(|d: Duration| chrono::Utc::now().timestamp() + d.as_secs() as i64);

    Ok(OAuthTokens {
        access_token: token_result.access_token().secret().clone(),
        refresh_token: token_result
            .refresh_token()
            .map(|t: &oauth2::RefreshToken| t.secret().clone()),
        expires_at,
    })
}

pub async fn refresh_oauth_token(config: &OAuthConfig, refresh_token: &str) -> Result<OAuthTokens> {
    if config.client_id.is_empty() {
        return Err(AppError::OAuthError(format!(
            "Missing {} OAuth client ID. Set {}.",
            config.display_name, config.env_var_hint
        )));
    }

    let is_public_client = config.client_secret.is_empty();

    let mut client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(AuthUrl::new(config.auth_url.clone()).map_err(|e| AppError::OAuthError(e.to_string()))?)
        .set_token_uri(TokenUrl::new(config.token_url.clone()).map_err(|e| AppError::OAuthError(e.to_string()))?);
    if !is_public_client {
        client = client.set_client_secret(ClientSecret::new(config.client_secret.clone()));
    }

    let http_client = reqwest::Client::new();
    let token_result = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
        .request_async(&http_client)
        .await
        .map_err(|e| AppError::OAuthError(format!("Token refresh failed: {}", e)))?;

    let expires_at = token_result
        .expires_in()
        .map(|d: Duration| chrono::Utc::now().timestamp() + d.as_secs() as i64);

    Ok(OAuthTokens {
        access_token: token_result.access_token().secret().clone(),
        refresh_token: token_result
            .refresh_token()
            .map(|t: &oauth2::RefreshToken| t.secret().clone())
            .or_else(|| Some(refresh_token.to_string())),
        expires_at,
    })
}

fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::OAuthError(format!("Failed to configure callback listener: {}", e)))?;

    let deadline = std::time::Instant::now() + Duration::from_secs(180);

    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(15)))
                    .map_err(|e| AppError::OAuthError(format!("Failed to configure callback stream: {}", e)))?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(15)))
                    .map_err(|e| AppError::OAuthError(format!("Failed to configure callback stream: {}", e)))?;

                match read_callback_request(&stream, expected_state) {
                    Ok(code) => {
                        write_callback_response(&mut stream, success_response())?;
                        return Ok(code);
                    }
                    Err(error) => {
                        let _ = write_callback_response(&mut stream, &error_response(&error.to_string()));
                    }
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                return Err(AppError::OAuthError(format!("Failed to accept connection: {}", err)));
            }
        }
    }

    Err(AppError::OAuthError(
        "Timed out waiting for OAuth callback. Please try again.".to_string(),
    ))
}

fn read_callback_request(stream: &TcpStream, expected_state: &str) -> Result<String> {
    let mut request_line = String::new();
    let mut reader = BufReader::new(stream);
    reader
        .read_line(&mut request_line)
        .map_err(|e| AppError::OAuthError(format!("Failed to read callback request: {}", e)))?;

    let url_path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AppError::OAuthError("Invalid callback request".to_string()))?;

    extract_callback_code(url_path, expected_state)
}

fn extract_callback_code(url_path: &str, expected_state: &str) -> Result<String> {
    let full_url = format!("http://localhost{}", url_path);
    let parsed =
        url::Url::parse(&full_url).map_err(|e| AppError::OAuthError(format!("Failed to parse callback URL: {}", e)))?;

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;

    for (key, value) in parsed.query_pairs() {
        if key == "code" {
            code = Some(value.to_string());
        } else if key == "state" {
            state = Some(value.to_string());
        }
    }

    let code = code.ok_or_else(|| AppError::OAuthError("Missing authorization code in callback".to_string()))?;
    let state = state.ok_or_else(|| AppError::OAuthError("Missing OAuth state in callback".to_string()))?;

    if state != expected_state {
        return Err(AppError::OAuthError("CSRF token mismatch".to_string()));
    }

    Ok(code)
}

fn success_response() -> &'static str {
    r#"HTTP/1.1 200 OK
Content-Type: text/html

<!DOCTYPE html>
<html>
<head><title>EmailOps</title></head>
<body style="font-family: sans-serif; text-align: center; padding-top: 50px;">
    <h1>Authorization Successful!</h1>
    <p>You can close this window and return to EmailOps.</p>
</body>
</html>
"#
}

fn error_response(message: &str) -> String {
    format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<!DOCTYPE html><html><head><title>EmailOps</title></head><body style=\"font-family: sans-serif; text-align: center; padding-top: 50px;\"><h1>Authorization Failed</h1><p>{}</p><p>You can close this window and return to EmailOps.</p></body></html>",
        message
    )
}

fn write_callback_response(stream: &mut TcpStream, response: &str) -> Result<()> {
    stream
        .write_all(response.as_bytes())
        .map_err(|e| AppError::OAuthError(format!("Failed to send callback response: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::extract_callback_code;
    use super::reversed_client_id;
    use super::OAuthConfig;

    #[test]
    fn reverses_a_google_ios_client_id_into_its_uri_scheme() {
        // Google iOS clients redirect to the "reversed client ID" as a custom
        // scheme. Getting this wrong yields a redirect_uri_mismatch that is
        // only visible after a full round trip through the consent screen, so
        // it is worth pinning exactly.
        assert_eq!(
            reversed_client_id("123456789-abcdefg.apps.googleusercontent.com").as_deref(),
            Some("com.googleusercontent.apps.123456789-abcdefg")
        );
    }

    #[test]
    fn rejects_a_client_id_that_is_not_a_google_client_id() {
        // A desktop client ID pasted into the iOS slot, an Azure GUID, or a
        // typo must not silently produce a scheme that cannot ever match.
        assert_eq!(reversed_client_id(""), None);
        assert_eq!(reversed_client_id("not-a-client-id"), None);
        assert_eq!(reversed_client_id("11111111-2222-3333-4444-555555555555"), None);
    }

    #[test]
    fn rejects_an_already_reversed_client_id() {
        // Defensive: if someone pastes the scheme back into the client-ID
        // field, reversing again would produce nonsense rather than failing.
        assert_eq!(reversed_client_id("com.googleusercontent.apps.123456789-abcdefg"), None);
    }

    #[test]
    fn rejects_a_google_suffix_with_no_client_body() {
        assert_eq!(reversed_client_id(".apps.googleusercontent.com"), None);
    }

    #[test]
    fn gmail_mobile_redirect_is_the_reversed_client_id_scheme() {
        // Google's own convention uses a single slash after the scheme
        // (`scheme:/path`), not `scheme://path`. Registering one and sending
        // the other is a redirect_uri_mismatch.
        assert_eq!(
            super::mobile_redirect_uri("gmail", "123-abc.apps.googleusercontent.com").as_deref(),
            Some("com.googleusercontent.apps.123-abc:/oauth2redirect")
        );
    }

    #[test]
    fn outlook_mobile_redirect_uses_the_app_bundle_scheme() {
        // Azure public clients accept an arbitrary custom scheme for the
        // "Mobile and desktop applications" platform. The client ID does not
        // participate, unlike Google's reversed-client-ID scheme.
        assert_eq!(
            super::mobile_redirect_uri("outlook", "any-azure-guid").as_deref(),
            Some("com.emailops.app://oauth2redirect")
        );
    }

    #[test]
    fn gmail_mobile_redirect_fails_loudly_on_a_bad_client_id() {
        // A desktop client ID left in the iOS slot must not yield a redirect
        // URI that can never match — callers surface this as a config error.
        assert_eq!(super::mobile_redirect_uri("gmail", "not-a-google-client-id"), None);
        assert_eq!(super::mobile_redirect_uri("gmail", ""), None);
    }

    #[test]
    fn gmail_scopes_include_calendar_events_read_write() {
        // `calendar.events` covers read + create/update of events (the calendar
        // view syncs events and creates them from the new-event dialog).
        let config = OAuthConfig::gmail();
        assert!(
            config
                .scopes
                .iter()
                .any(|s| s == "https://www.googleapis.com/auth/calendar.events"),
            "Gmail OAuth must request event read/write access to Google Calendar, got: {:?}",
            config.scopes
        );
    }

    #[test]
    fn gmail_scopes_include_calendar_list_enumeration() {
        // `calendar.events` grants event access on a calendar you can already
        // name, but not `calendarList.list` — without this scope the app can
        // never discover the user's other or shared calendars.
        let config = OAuthConfig::gmail();
        assert!(
            config
                .scopes
                .iter()
                .any(|s| s == "https://www.googleapis.com/auth/calendar.calendarlist.readonly"),
            "Gmail OAuth must be able to enumerate the account's calendars, got: {:?}",
            config.scopes
        );
    }

    #[test]
    fn gmail_scopes_omit_broad_calendar_readonly() {
        // `calendar.calendarlist.readonly` lists calendars; `calendar.readonly`
        // would additionally grant reading every calendar's full contents.
        // Google's verification requires the narrowest scope that works.
        let config = OAuthConfig::gmail();
        assert!(
            !config
                .scopes
                .iter()
                .any(|s| s == "https://www.googleapis.com/auth/calendar.readonly"
                    || s == "https://www.googleapis.com/auth/calendar"),
            "a broader calendar scope than needed was requested, got: {:?}",
            config.scopes
        );
    }

    #[test]
    fn gmail_scopes_omit_redundant_readonly() {
        // `gmail.modify` already grants full read access; requesting
        // `gmail.readonly` on top is redundant and complicates Google's
        // restricted-scope verification (narrowest-scope requirement).
        let config = OAuthConfig::gmail();
        assert!(
            !config
                .scopes
                .iter()
                .any(|s| s == "https://www.googleapis.com/auth/gmail.readonly"),
            "gmail.readonly is redundant alongside gmail.modify, got: {:?}",
            config.scopes
        );
    }

    #[test]
    fn outlook_scopes_include_calendar_read_write() {
        let config = OAuthConfig::outlook();
        assert!(
            config.scopes.iter().any(|s| s == "Calendars.ReadWrite"),
            "Outlook OAuth must request read/write access to Graph calendars, got: {:?}",
            config.scopes
        );
    }

    #[test]
    fn extracts_code_from_valid_callback() {
        let code = extract_callback_code("/?code=abc123&state=expected", "expected").unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn rejects_callback_with_wrong_state() {
        let error = extract_callback_code("/?code=abc123&state=wrong", "expected").unwrap_err();
        assert!(error.to_string().contains("CSRF token mismatch"));
    }

    #[test]
    fn rejects_callback_without_code() {
        let error = extract_callback_code("/?state=expected", "expected").unwrap_err();
        assert!(error.to_string().contains("Missing authorization code"));
    }
}
