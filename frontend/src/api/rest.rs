//! REST API 客户端（基于 gloo-net）：客户端构造与端点子模块接线。
//! 错误类型 `ApiError` 见 `error.rs`，HTTP 传输动词见 `transport.rs`，
//! `Retry-After` 解析见 `retry_after.rs`。

mod arbitrage;
mod auth;
mod dto;
mod encoding;
mod error;
mod exchanges;
mod integrations;
mod portfolio_system;
mod retry_after;
mod timeout;
mod trading;
mod transport;
mod watchlist_alerts;

use crate::api::base::{
    normalize_api_auth_token, normalize_api_base, stored_api_auth_token, stored_or_default_api_base,
};
use encoding::{encode_path_segment, encode_query_component};
use leptos::prelude::{GetUntracked, RwSignal};
use std::sync::Arc;

pub use dto::*;
pub use error::ApiError;
pub(crate) use portfolio_system::{
    portfolio_envelope_degraded_problem, portfolio_envelope_problem,
};
pub(crate) use timeout::with_mutation_timeout;
pub use transport::MutationRequestContext;

const HEADER_REQUEST_ID: &str = "x-request-id";
const HEADER_AUTHORIZATION: &str = "authorization";
const HEADER_IDEMPOTENCY_KEY: &str = "idempotency-key";

#[derive(Clone)]
pub struct ApiClient {
    base_url: Arc<dyn Fn() -> String + Send + Sync>,
    auth_token: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiClient {
    pub fn new() -> Self {
        let base_url = stored_or_default_api_base();
        let auth_token = stored_api_auth_token();
        Self::with_base_and_auth(&base_url, &auth_token)
    }

    pub fn with_base(base_url: &str) -> Self {
        Self::with_base_and_auth(base_url, "")
    }

    pub fn with_base_and_auth(base_url: &str, api_auth_token: &str) -> Self {
        let base_url = normalize_api_base(base_url);
        let auth_token = auth_token_from_raw(api_auth_token);
        Self {
            base_url: Arc::new(move || base_url.clone()),
            auth_token: Arc::new(move || auth_token.clone()),
        }
    }

    pub fn with_base_signal(api_base: RwSignal<String>) -> Self {
        Self {
            base_url: Arc::new(move || normalize_api_base(&api_base.get_untracked())),
            auth_token: Arc::new(|| None),
        }
    }

    pub fn with_base_signal_and_auth(
        api_base: RwSignal<String>,
        api_auth_token: RwSignal<String>,
    ) -> Self {
        Self {
            base_url: Arc::new(move || normalize_api_base(&api_base.get_untracked())),
            auth_token: Arc::new(move || auth_token_from_raw(&api_auth_token.get_untracked())),
        }
    }

    pub fn base_url(&self) -> String {
        (self.base_url)()
    }

    pub async fn health(&self) -> Result<String, ApiError> {
        self.get_text("/health").await
    }

    #[cfg(test)]
    fn authorization_header(&self) -> Option<String> {
        (self.auth_token)().map(|token| format!("Bearer {token}"))
    }
}

fn auth_token_from_raw(token: &str) -> Option<String> {
    let token = normalize_api_auth_token(token);
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::Set;

    #[test]
    fn static_base_is_normalized() {
        let client = ApiClient::with_base(" http://127.0.0.1:8000/// ");

        assert_eq!(client.base_url(), "http://127.0.0.1:8000");
        assert_eq!(client.url("/health"), "http://127.0.0.1:8000/health");
    }

    #[test]
    fn auth_header_is_absent_without_token() {
        let client = ApiClient::with_base("http://127.0.0.1:8000");

        assert_eq!(client.authorization_header(), None);
    }

    #[test]
    fn auth_header_uses_bearer_token_from_signal() {
        let token = RwSignal::new(" secret ".to_owned());
        let client = ApiClient::with_base_signal_and_auth(
            RwSignal::new("http://127.0.0.1:8000".to_owned()),
            token,
        );

        assert_eq!(
            client.authorization_header().as_deref(),
            Some("Bearer secret")
        );
        token.set(String::new());
        assert_eq!(client.authorization_header(), None);
    }

    #[test]
    fn legacy_options_and_simulation_cannot_reenter_the_main_rest_client() {
        let rest_root = include_str!("rest.rs");
        let dto_module = include_str!("rest/dto.rs");
        let forbidden_modules = [
            ["mod", "options;"].join(" "),
            ["mod", "simulation;"].join(" "),
        ];
        let forbidden_aliases = [
            ["pub type", "SimPosition"].join(" "),
            ["pub type", "OpenRequest"].join(" "),
            ["pub type", "CloseResponse"].join(" "),
            ["pub type", "GreeksRequest"].join(" "),
            ["pub type", "OptionGreeks"].join(" "),
            ["pub type", "PriceRequest"].join(" "),
            ["pub type", "PriceResponse"].join(" "),
            ["pub type", "IvRequest"].join(" "),
            ["pub type", "IvResponse"].join(" "),
        ];

        assert!(forbidden_modules
            .iter()
            .all(|module| !rest_root.lines().any(|line| line.trim() == module)));
        assert!(forbidden_aliases
            .iter()
            .all(|alias| !dto_module.contains(alias)));
    }
}
