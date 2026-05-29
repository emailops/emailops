pub mod gmail;
pub mod imap;
pub mod mime_builder;
#[cfg(any(test, debug_assertions))]
pub mod mock;
pub mod oauth;
pub mod outlook;
pub mod outlook_payload;
pub mod provider;
