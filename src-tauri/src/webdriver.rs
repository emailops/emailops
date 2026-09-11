//! Embedded W3C WebDriver server for UI verification runs
//! (`.claude/skills/verify-emailops`).
//!
//! The server is unauthenticated automation over HTTP on 127.0.0.1, so it is
//! opt-in twice: the `webdriver` cargo feature (dev builds only, see the
//! `compile_error!` below) and `TAURI_WEBDRIVER_PORT` at launch. A plain
//! `make dev` — which may be holding the real mailbox — never listens.

use tauri::{Builder, Runtime};

/// Environment variable the launcher sets to request the server.
pub const PORT_ENV_VAR: &str = "TAURI_WEBDRIVER_PORT";

/// Port requested through [`PORT_ENV_VAR`].
///
/// `None` (unset, blank, non-numeric or zero) means "do not start the server":
/// a typo must not fall back to the plugin's default port.
pub fn port_from_env(value: Option<&str>) -> Option<u16> {
    value?.trim().parse::<u16>().ok().filter(|port| *port != 0)
}

/// Register the WebDriver plugin on `builder` when this launch asked for it.
pub fn register<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    let Some(port) = port_from_env(std::env::var(PORT_ENV_VAR).ok().as_deref()) else {
        return builder;
    };
    #[cfg(feature = "webdriver")]
    {
        eprintln!("[startup] embedded WebDriver server enabled on 127.0.0.1:{port}");
        builder.plugin(tauri_plugin_wdio_webdriver::init_with_port(port))
    }
    #[cfg(not(feature = "webdriver"))]
    {
        eprintln!("[startup] {PORT_ENV_VAR}={port} ignored: this build has no `webdriver` feature");
        builder
    }
}

#[cfg(all(feature = "webdriver", not(debug_assertions)))]
compile_error!("the `webdriver` feature exposes unauthenticated automation over HTTP; it is dev-only and must not be built in release");

#[cfg(test)]
mod tests {
    use super::port_from_env;

    #[test]
    fn unset_means_no_webdriver_server() {
        assert_eq!(port_from_env(None), None);
    }

    #[test]
    fn a_valid_port_enables_the_server_on_that_port() {
        assert_eq!(port_from_env(Some("4445")), Some(4445));
        assert_eq!(port_from_env(Some(" 9515 ")), Some(9515));
    }

    #[test]
    fn an_unparseable_or_zero_value_keeps_the_server_off() {
        // A typo must not silently open the default port on a dev app that may
        // be holding the real mailbox.
        for bad in ["", "abc", "0", "70000", "-1"] {
            assert_eq!(port_from_env(Some(bad)), None, "value {bad:?}");
        }
    }
}
