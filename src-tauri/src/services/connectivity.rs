//! Connectivity probe.
//!
//! Periodically checks whether the device has actual internet connectivity by
//! issuing a cheap HTTP HEAD request to a neutral, high-availability endpoint.
//! The result is published two ways:
//!   * cached in an atomic flag readable via the `is_online` command (used by
//!     the frontend on startup and as a defensive fallback if it misses an
//!     event), and
//!   * emitted as `app-connectivity-changed` Tauri events on every state
//!     transition (online → offline or vice versa).
//!
//! Why a probe instead of relying on `navigator.onLine`: the browser flag only
//! reflects "the OS thinks we have a link layer", which is wrong on captive
//! portals, VPN drops, and Wi-Fi with no internet upstream. A periodic 1-line
//! HEAD request is cheap and authoritative.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::services::app_handle::AppHandle;
use serde::Serialize;
#[cfg(feature = "desktop")]
use tauri::Emitter;

/// Endpoint to probe. Picked because it is globally anycasted, returns small
/// responses, and is unaffected by individual provider outages (e.g. Gmail
/// down does not imply we're offline).
///
/// HTTPS, not plain HTTP. The earlier `http://` saved a handshake but made this
/// the app's only cleartext request — which on iOS means an App Transport
/// Security exception, and an exception is a permanent hole justified by a
/// liveness check. The handshake cost is paid once: the probe reuses one
/// keep-alive connection across the 15-second loop.
const PROBE_URL: &str = "https://1.1.1.1/";
const PROBE_INTERVAL: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityEvent {
    pub online: bool,
}

#[derive(Clone)]
pub struct ConnectivityMonitor {
    /// Latest probe result. Optimistic default (true) so the UI doesn't show
    /// an offline banner before the first probe completes.
    online: Arc<AtomicBool>,
}

impl ConnectivityMonitor {
    /// No-op instance for tests: always reports online, no probe loop.
    pub fn stub() -> Self {
        Self {
            online: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn start(app: AppHandle) -> Self {
        let online = Arc::new(AtomicBool::new(true));
        let monitor = Self { online: online.clone() };

        let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "system",
                    format!("connectivity: failed to build HTTP client (probe disabled): {e}"),
                );
                return monitor;
            }
        };

        crate::runtime::spawn::spawn(async move {
            // Run one probe immediately so the cached state reflects reality
            // before the first PROBE_INTERVAL elapses.
            probe_once(&client, &online, &app).await;
            let mut ticker = tokio::time::interval(PROBE_INTERVAL);
            // Skip the immediate tick that `interval` fires by default — we
            // already ran one probe above.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                probe_once(&client, &online, &app).await;
            }
        });

        monitor
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// Shared handle to the cached online flag, for components that need to
    /// poll it from inside a hot loop without going through the monitor's
    /// `is_online()` method (e.g. the sync scheduler's poll loops, which want
    /// to skip a tick atomically without holding an Arc<ConnectivityMonitor>).
    pub fn online_flag(&self) -> Arc<AtomicBool> {
        self.online.clone()
    }
}

async fn probe_once(client: &reqwest::Client, online: &Arc<AtomicBool>, app: &AppHandle) {
    let now_online = match client.head(PROBE_URL).send().await {
        Ok(resp) => resp.status().is_success() || resp.status().is_redirection(),
        Err(_) => false,
    };
    // Use swap so we only emit on actual transitions; a long stretch of
    // identical results doesn't spam the frontend.
    let prev = online.swap(now_online, Ordering::Relaxed);
    if prev != now_online {
        if let Err(e) = app.emit("app-connectivity-changed", ConnectivityEvent { online: now_online }) {
            crate::services::logger::log("error", "system", format!("connectivity: failed to emit event: {e}"));
        }
    }
}
