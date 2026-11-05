//! How an `ImapClient` reaches its server: the IMAP byte stream and SMTP
//! submission.
//!
//! Production goes through [`NativeTlsTransport`] (TCP + implicit TLS or
//! STARTTLS, lettre for SMTP). Tests swap in
//! [`crate::sync::imap_fake::FakeImapServer`], an in-memory IMAP server, so
//! every `ImapClient` operation runs end to end without a network.
//!
//! IDLE stays on the TLS stream itself (it needs `SetReadTimeout`), so it is
//! not part of the trait.

use std::io::{Read, Write};

use async_trait::async_trait;

use crate::models::error::{AppError, Result};
use crate::sync::imap::ImapCredentials;

/// A bidirectional IMAP byte stream.
pub trait ImapStream: Read + Write + Send {}
impl<T: Read + Write + Send> ImapStream for T {}

/// Opens IMAP streams and submits mail over SMTP for one account.
#[async_trait]
pub trait MailTransport: Send + Sync {
    /// Open a stream that is ready for `LOGIN` (TLS already negotiated).
    fn open_imap(&self, creds: &ImapCredentials) -> std::result::Result<Box<dyn ImapStream>, imap::Error>;

    /// Submit `email` for delivery.
    async fn send_smtp(&self, creds: &ImapCredentials, email: lettre::Message) -> Result<()>;

    /// Connect to the SMTP server and authenticate, sending nothing.
    async fn test_smtp(&self, creds: &ImapCredentials) -> Result<()>;
}

/// Path of a PEM CA certificate trusted in addition to the system roots. Read
/// only by debug builds, so the self-signed test servers of `make imap-e2e`
/// can be reached; a release build ignores it.
pub const TEST_CA_ENV: &str = "EMAILOPS_IMAP_TEST_CA";

/// The production transport: native TLS over TCP.
#[derive(Default)]
pub struct NativeTlsTransport {
    extra_root_pem: Option<Vec<u8>>,
}

impl NativeTlsTransport {
    /// The transport every `ImapClient` starts with: system roots, plus the
    /// [`TEST_CA_ENV`] certificate in a debug build.
    pub fn from_env() -> Self {
        Self {
            extra_root_pem: test_ca_from_env(),
        }
    }

    /// Trust `pem` in addition to the system roots.
    pub fn with_extra_root(pem: Vec<u8>) -> Self {
        Self {
            extra_root_pem: Some(pem),
        }
    }

    fn tls_connector(&self) -> std::result::Result<native_tls::TlsConnector, imap::Error> {
        let mut builder = native_tls::TlsConnector::builder();
        if let Some(pem) = &self.extra_root_pem {
            let cert = native_tls::Certificate::from_pem(pem)
                .map_err(|e| imap::Error::Bad(format!("TLS init failed: extra root certificate: {e}")))?;
            builder.add_root_certificate(cert);
        }
        builder
            .build()
            .map_err(|e| imap::Error::Bad(format!("TLS init failed: {e}")))
    }

    /// TCP connect (with timeout) + TLS handshake, with the IO timeout set on
    /// the socket. Shared by [`MailTransport::open_imap`] and the IDLE session.
    pub fn open_tls(
        &self,
        creds: &ImapCredentials,
    ) -> std::result::Result<native_tls::TlsStream<std::net::TcpStream>, imap::Error> {
        use std::io;
        use std::net::{TcpStream, ToSocketAddrs};
        use std::time::Duration;

        const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

        let tls = self.tls_connector()?;

        // Resolve the first address and connect with a timeout.
        let addr = (creds.host.as_str(), creds.port)
            .to_socket_addrs()
            .map_err(imap::Error::Io)?
            .next()
            .ok_or_else(|| {
                imap::Error::Io(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    format!("No addresses resolved for {}:{}", creds.host, creds.port),
                ))
            })?;

        let mut tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(imap::Error::Io)?;
        tcp.set_read_timeout(Some(crate::sync::imap::IO_TIMEOUT))
            .map_err(imap::Error::Io)?;
        tcp.set_write_timeout(Some(crate::sync::imap::IO_TIMEOUT))
            .map_err(imap::Error::Io)?;

        if crate::sync::imap::ImapTlsMode::for_port(creds.port) == crate::sync::imap::ImapTlsMode::StartTls {
            crate::sync::imap::negotiate_starttls(&mut tcp).map_err(imap::Error::Io)?;
        }

        tls.connect(creds.host.as_str(), tcp)
            .map_err(|e| imap::Error::Bad(format!("TLS handshake failed: {e}")))
    }

    fn smtp_mailer(&self, creds: &ImapCredentials) -> Result<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
        use lettre::{AsyncSmtpTransport, Tokio1Executor};

        let login = Credentials::new(creds.username.clone(), creds.password.clone());
        // Port 465 = SMTPS (TLS on connect); anything else (e.g. 587) = STARTTLS.
        // Using relay() on a STARTTLS port causes an immediate TLS handshake against
        // a server that's still in plain-text mode, which hangs until timeout.
        let implicit = creds.smtp_port == 465;
        let mut builder = if implicit {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&creds.smtp_host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&creds.smtp_host)
        }
        .map_err(|e| AppError::SyncError(format!("SMTP relay init failed: {e}")))?;

        if let Some(pem) = &self.extra_root_pem {
            let cert = Certificate::from_pem(pem)
                .map_err(|e| AppError::SyncError(format!("SMTP TLS init failed: extra root certificate: {e}")))?;
            let params = TlsParameters::builder(creds.smtp_host.clone())
                .add_root_certificate(cert)
                .build_native()
                .map_err(|e| AppError::SyncError(format!("SMTP TLS init failed: {e}")))?;
            builder = builder.tls(if implicit {
                Tls::Wrapper(params)
            } else {
                Tls::Required(params)
            });
        }

        Ok(builder.credentials(login).port(creds.smtp_port).build())
    }
}

#[async_trait]
impl MailTransport for NativeTlsTransport {
    fn open_imap(&self, creds: &ImapCredentials) -> std::result::Result<Box<dyn ImapStream>, imap::Error> {
        Ok(Box::new(self.open_tls(creds)?))
    }

    async fn send_smtp(&self, creds: &ImapCredentials, email: lettre::Message) -> Result<()> {
        use lettre::AsyncTransport;
        self.smtp_mailer(creds)?
            .send(email)
            .await
            .map_err(|e| AppError::SyncError(format!("SMTP send failed: {e}")))?;
        Ok(())
    }

    async fn test_smtp(&self, creds: &ImapCredentials) -> Result<()> {
        self.smtp_mailer(creds)?
            .test_connection()
            .await
            .map_err(|e| AppError::SyncError(format!("SMTP connection test failed: {e}")))?;
        Ok(())
    }
}

#[cfg(debug_assertions)]
fn test_ca_from_env() -> Option<Vec<u8>> {
    let path = std::env::var_os(TEST_CA_ENV)?;
    match std::fs::read(&path) {
        Ok(pem) => Some(pem),
        Err(e) => {
            crate::services::logger::log(
                "error",
                "sync",
                format!(
                    "{TEST_CA_ENV} is set but {} cannot be read: {e}",
                    path.to_string_lossy()
                ),
            );
            None
        }
    }
}

#[cfg(not(debug_assertions))]
fn test_ca_from_env() -> Option<Vec<u8>> {
    None
}
