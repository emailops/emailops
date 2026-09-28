use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, RefreshToken, TokenResponse, TokenUrl,
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
pub const GMAIL_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const BUNDLED_GMAIL_CLIENT_ID: Option<&str> = option_env!("EMAILOPS_GMAIL_CLIENT_ID");
const BUNDLED_GMAIL_CLIENT_SECRET: Option<&str> = option_env!("EMAILOPS_GMAIL_CLIENT_SECRET");

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
        let client_id = runtime_or_bundled_env("EMAILOPS_GMAIL_CLIENT_ID", BUNDLED_GMAIL_CLIENT_ID);
        let client_secret = runtime_or_bundled_env("EMAILOPS_GMAIL_CLIENT_SECRET", BUNDLED_GMAIL_CLIENT_SECRET);

        Self {
            client_id,
            client_secret,
            auth_url: GMAIL_AUTH_URL.to_string(),
            token_url: GMAIL_TOKEN_URL.to_string(),
            scopes: GMAIL_SCOPES.iter().map(|s| s.to_string()).collect(),
            redirect_host: "127.0.0.1",
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

pub async fn start_oauth_flow(config: &OAuthConfig) -> Result<OAuthTokens> {
    if config.client_id.is_empty() {
        return Err(AppError::OAuthError(format!(
            "Missing {} OAuth client ID. Set {}.",
            config.display_name, config.env_var_hint
        )));
    }

    // Find an available port for the callback
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| AppError::OAuthError(format!("Failed to bind port: {}", e)))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::OAuthError(format!("Failed to read bound port: {}", e)))?
        .port();
    let redirect_url = format!("http://{}:{}", config.redirect_host, port);

    let client = oauth_client(config, Some(&redirect_url))?;
    let (auth_url, csrf_token, pkce_verifier) = authorization_url(&client, &config.scopes);

    // Open browser for user authorization. `that_detached` (not `that`) is
    // required here: `open::that` blocks the calling thread until the
    // launched application *exits* — on macOS `open -a` hands off via
    // LaunchServices and returns immediately, masking this, but on Linux
    // `open::that` execs the browser directly and waits on it, so this
    // Tauri command would never resolve until the user closed every window
    // of their browser. `that_detached` launches and returns immediately.
    open::that_detached(auth_url.as_str())
        .map_err(|e| AppError::OAuthError(format!("Failed to open browser: {}", e)))?;

    // Wait for the callback
    let code = wait_for_callback(listener, csrf_token.secret())?;

    // Exchange code for tokens
    let http_client = reqwest::Client::new();
    let token_result = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
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

type ConfiguredClient = oauth2::basic::BasicClient<
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

/// Public client = no client secret. Azure AD public-client apps reject any
/// client_secret (AADSTS90023); Google desktop clients ship a secret that is
/// not confidential (it is embedded in the binary).
fn oauth_client(config: &OAuthConfig, redirect_url: Option<&str>) -> Result<ConfiguredClient> {
    let mut client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(AuthUrl::new(config.auth_url.clone()).map_err(|e| AppError::OAuthError(e.to_string()))?)
        .set_token_uri(TokenUrl::new(config.token_url.clone()).map_err(|e| AppError::OAuthError(e.to_string()))?);
    if let Some(redirect_url) = redirect_url {
        client = client.set_redirect_uri(
            RedirectUrl::new(redirect_url.to_string()).map_err(|e| AppError::OAuthError(e.to_string()))?,
        );
    }
    if !config.client_secret.is_empty() {
        client = client.set_client_secret(ClientSecret::new(config.client_secret.clone()));
    }
    Ok(client)
}

/// Authorization URL with a random `state` and a PKCE S256 challenge.
fn authorization_url(client: &ConfiguredClient, scopes: &[String]) -> (oauth2::url::Url, CsrfToken, PkceCodeVerifier) {
    let mut auth_request = client.authorize_url(CsrfToken::new_random);
    for scope in scopes {
        auth_request = auth_request.add_scope(oauth2::Scope::new(scope.clone()));
    }
    // PKCE for every client, not only public ones: a desktop app's client
    // secret is extractable, so only PKCE binds the code to this instance
    // (RFC 8252 §6). Azure AD requires it; Google accepts it with the secret.
    let (challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state) = auth_request.set_pkce_challenge(challenge).url();
    (url, state, pkce_verifier)
}

pub async fn refresh_oauth_token(config: &OAuthConfig, refresh_token: &str) -> Result<OAuthTokens> {
    if config.client_id.is_empty() {
        return Err(AppError::OAuthError(format!(
            "Missing {} OAuth client ID. Set {}.",
            config.display_name, config.env_var_hint
        )));
    }

    let client = oauth_client(config, None)?;

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

/// Revoke a token at an RFC 7009 endpoint (Google's `/revoke`). Revoking a
/// refresh token ends the whole grant, so the app loses access immediately
/// instead of when the user finds it in their Google Account settings.
pub async fn revoke_token(revoke_url: &str, token: &str) -> Result<()> {
    let response = reqwest::Client::new()
        .post(revoke_url)
        .form(&[("token", token)])
        .send()
        .await
        .map_err(|e| AppError::OAuthError(format!("Token revocation failed: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::OAuthError(format!("Token revocation failed: HTTP {status}")));
    }
    Ok(())
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
    use super::{authorization_url, oauth_client, revoke_token, OAuthConfig};

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

    fn confidential_config() -> OAuthConfig {
        OAuthConfig {
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            ..OAuthConfig::gmail()
        }
    }

    #[test]
    fn authorization_url_sends_pkce_s256_for_confidential_client() {
        // Google desktop clients ship a (non-confidential) client secret; PKCE
        // must still bind the code to this app instance (RFC 8252 §6).
        let config = confidential_config();
        let client = oauth_client(&config, Some("http://127.0.0.1:1234")).unwrap();
        let (url, _state, _verifier) = authorization_url(&client, &config.scopes);
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert!(
            query.iter().any(|(k, v)| k == "code_challenge_method" && v == "S256"),
            "{url}"
        );
        assert!(query.iter().any(|(k, _)| k == "code_challenge"), "{url}");
    }

    #[test]
    fn authorization_url_sends_pkce_s256_for_public_client() {
        let config = OAuthConfig {
            client_id: "client-id".to_string(),
            client_secret: String::new(),
            ..OAuthConfig::outlook()
        };
        let client = oauth_client(&config, Some("http://localhost:1234")).unwrap();
        let (url, _state, _verifier) = authorization_url(&client, &config.scopes);
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "code_challenge_method" && v == "S256"),
            "{url}"
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

    /// One-shot HTTP server: answers the first request with `status` and
    /// returns the raw request text it received.
    fn one_shot_server(status: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            // Give up after 5 s so a client that never connects fails the
            // test instead of hanging it.
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20))
                    }
                    Err(_) => return String::new(),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut buf = vec![0u8; 8192];
            let mut request = String::new();
            // Read until the form body (after the blank line) has arrived.
            while !request.contains("token=") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (url, handle)
    }

    #[tokio::test]
    async fn revoke_token_posts_the_token_as_a_form() {
        let (url, server) = one_shot_server("200 OK");
        revoke_token(&url, "refresh-123").await.unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /revoke "), "{request}");
        assert!(request.contains("token=refresh-123"), "{request}");
    }

    #[tokio::test]
    async fn revoke_token_fails_on_a_non_success_status() {
        let (url, server) = one_shot_server("400 Bad Request");
        let result = revoke_token(&url, "already-revoked").await;
        server.join().unwrap();
        assert!(result.is_err(), "{result:?}");
    }
}
