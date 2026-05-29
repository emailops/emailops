//! Boots a `wiremock` server from an `emailops_lib::sync::mock::Cassette`
//! and exposes its `base_url()` so integration tests can construct a
//! `GmailClient` / `OutlookClient` via `with_base_url(server.base_url())`.
//!
//! Lives under `tests/` rather than in `src/sync/mock/` because `wiremock`
//! is a dev-dependency and the lib can't link it.

use std::path::Path;

use emailops_lib::sync::mock::Cassette;
use wiremock::matchers::{method as method_matcher, path as path_matcher, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub struct MockProviderServer {
    server: MockServer,
    cassette: Cassette,
}

impl MockProviderServer {
    /// Load a cassette from disk, start a `wiremock` server on a random
    /// localhost port, and register every interaction as a stub. Stubs
    /// match on `(method, path, every query_param)` — the same axes the
    /// production clients vary on per call.
    pub async fn from_cassette_path(path: &Path) -> Self {
        let cassette = Cassette::load_from(path).expect("load cassette");
        Self::from_cassette(cassette).await
    }

    pub async fn from_cassette(cassette: Cassette) -> Self {
        let server = MockServer::start().await;
        for interaction in &cassette.interactions {
            let req = &interaction.request;
            let resp = &interaction.response;
            let mut mock = Mock::given(method_matcher(req.method.as_str())).and(path_matcher(req.url_path.as_str()));
            for (k, v) in &req.query_params {
                mock = mock.and(query_param(k.as_str(), v.as_str()));
            }
            let mut response = ResponseTemplate::new(resp.status);
            for (h_name, h_value) in &resp.headers {
                response = response.insert_header(h_name.as_str(), h_value.as_str());
            }
            if let Some(body) = &resp.body_json {
                response = response.set_body_json(body);
            }
            server.register(mock.respond_with(response)).await;
        }
        Self { server, cassette }
    }

    /// Base URL for `with_base_url(...)` on the production clients. Includes
    /// scheme + host + port, no trailing slash, no path.
    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    pub fn cassette(&self) -> &Cassette {
        &self.cassette
    }
}
