use crate::services::app_handle::AppHandle;

use crate::models::error::{AppError, Result};
use crate::models::Account;
use crate::services::accounts;
use crate::sync::gmail::GmailClient;
use crate::sync::imap::ImapClient;
use crate::sync::oauth::{self, OAuthConfig};
use crate::sync::outlook::OutlookClient;
use crate::sync::provider::EmailProvider;

/// Public alias used by other modules (e.g. attachment commands) that need a one-shot provider.
pub async fn build_provider(
    account: &Account,
    app: Option<AppHandle>,
) -> Result<Box<dyn crate::sync::provider::EmailProvider>> {
    build_provider_for_account(account, app).await
}

/// Build the email provider for an account, refreshing OAuth tokens proactively if needed.
/// For OAuth providers (Gmail, Outlook), the returned client also holds the refresh
/// token so it can recover transparently from mid-sync 401s without surfacing them as errors.
pub(super) async fn build_provider_for_account(
    account: &Account,
    app: Option<AppHandle>,
) -> Result<Box<dyn EmailProvider>> {
    match account.provider.as_str() {
        "gmail" => {
            let tokens = refresh_oauth_tokens_if_needed(account).await?;
            Ok(Box::new(GmailClient::new(
                tokens.access_token,
                tokens.refresh_token,
                app,
                Some(account.id.clone()),
            )))
        }
        "outlook" => {
            let tokens = refresh_oauth_tokens_if_needed(account).await?;
            Ok(Box::new(OutlookClient::new(
                tokens.access_token,
                tokens.refresh_token,
                app,
                Some(account.id.clone()),
            )))
        }
        "imap" => {
            let creds = accounts::get_imap_credentials(&account.id)?;
            Ok(Box::new(ImapClient::new(
                creds,
                account.email.clone(),
                account.name.clone(),
                account.id.clone(),
            )))
        }
        other => Err(AppError::SyncError(format!("Unsupported email provider: {other}"))),
    }
}

/// Proactively refresh the OAuth token for an OAuth account (Gmail/Outlook) when close to expiry.
/// Returns the full `OAuthTokens` (with the refresh token) so the caller can pass it
/// to the provider for mid-sync transparent refresh.
async fn refresh_oauth_tokens_if_needed(account: &Account) -> Result<crate::models::OAuthTokens> {
    let mut tokens = accounts::get_tokens(&account.id)?;

    // Refresh if: expiry is unknown (treat as potentially expired), or expires within 5 minutes.
    let needs_refresh = tokens
        .expires_at
        .map(|exp| exp < chrono::Utc::now().timestamp() + 300)
        .unwrap_or(true);

    if needs_refresh {
        let provider_label = match account.provider.as_str() {
            "outlook" => "Outlook",
            _ => "Gmail",
        };
        match tokens.refresh_token.clone() {
            Some(refresh_token) => {
                let config = OAuthConfig::for_provider(&account.provider);
                match oauth::refresh_oauth_token(&config, &refresh_token).await {
                    Ok(new_tokens) => {
                        accounts::store_tokens(&account.id, &new_tokens)?;
                        tokens = new_tokens;
                    }
                    Err(e) => {
                        return Err(AppError::AuthError(format!(
                            "{} session expired and could not be refreshed automatically. \
                             Please re-authenticate the account {} \
                             (Settings → Accounts → Re-authenticate). Details: {}",
                            provider_label, account.email, e
                        )));
                    }
                }
            }
            None => {
                return Err(AppError::AuthError(format!(
                    "{} session expired for {}. \
                     Please remove and re-add the account.",
                    provider_label, account.email
                )));
            }
        }
    }

    Ok(tokens)
}

/// Wrap a provider operation error, replacing raw OAuth error messages with
/// an actionable re-authentication prompt.
pub(super) fn map_send_error(err: AppError, account_email: &str) -> AppError {
    let msg = err.to_string();
    if msg.contains("invalid authentication credentials")
        || msg.contains("Invalid Credentials")
        || msg.contains("auth") && (msg.contains("401") || msg.contains("expired") || msg.contains("revoked"))
    {
        AppError::AuthError(format!(
            "Authentication expired for {}. \
             Please re-authenticate the account (Settings → Accounts → Re-authenticate).",
            account_email
        ))
    } else {
        err
    }
}
