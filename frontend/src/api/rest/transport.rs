//! `ApiClient` 的底层 HTTP 传输：请求头注入、request-id 生成、GET/POST/PATCH 动词与失败日志。
//! 错误解析见 `error.rs`，客户端构造与端点方法见各端点子模块。

use gloo_net::http::{Request, RequestBuilder};
use serde::{Deserialize, Serialize};

use crate::api::request_id::next_request_id;

use super::error::{error_from_response, ApiError};
use super::{ApiClient, HEADER_AUTHORIZATION, HEADER_IDEMPOTENCY_KEY, HEADER_REQUEST_ID};

mod probe;
use probe::{wasm_decode_probe_finish, wasm_decode_probe_start};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationRequestContext {
    request_id: String,
    idempotency_key: Option<String>,
}

impl Default for MutationRequestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationRequestContext {
    #[must_use]
    pub fn new() -> Self {
        Self {
            request_id: next_request_id(),
            idempotency_key: None,
        }
    }

    #[must_use]
    pub fn with_idempotency_key(idempotency_key: impl Into<String>) -> Self {
        let idempotency_key = idempotency_key.into();
        Self {
            request_id: next_request_id(),
            idempotency_key: (!idempotency_key.trim().is_empty()).then_some(idempotency_key),
        }
    }

    #[must_use]
    pub fn new_idempotent_attempt(scope: impl AsRef<str>) -> Self {
        let request_id = next_request_id();
        let scope = scope.as_ref().trim();
        let idempotency_key = if scope.is_empty() {
            request_id.clone()
        } else {
            format!("{scope}:{request_id}")
        };
        Self {
            request_id,
            idempotency_key: Some(idempotency_key),
        }
    }

    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    #[must_use]
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    #[must_use]
    pub fn evidence(&self) -> shared_types::ActionEvidence {
        shared_types::ActionEvidence::client_request(
            self.request_id.clone(),
            self.idempotency_key.clone(),
        )
    }
}

impl ApiClient {
    pub(in crate::api::rest) async fn get_text(&self, path: &str) -> Result<String, ApiError> {
        let result = async {
            let url = self.url(path);
            let request_id = next_request_id();
            let resp = self
                .request_headers(Request::get(&url), &request_id)
                .send()
                .await
                .map_err(|error| ApiError::network(error, Some(request_id.clone())))?;
            if !resp.ok() {
                return Err(error_from_response(resp, &request_id).await);
            }
            resp.text()
                .await
                .map_err(|error| ApiError::parse(error, Some(request_id)))
        }
        .await;
        log_failure("GET", path, &result);
        result
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        let result = self.get_json_quiet(path).await;
        log_failure("GET", path, &result);
        result
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn get_json_quiet<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let request_id = next_request_id();
        let resp = self
            .request_headers(Request::get(&url), &request_id)
            .send()
            .await
            .map_err(|error| ApiError::network(error, Some(request_id.clone())))?;
        if !resp.ok() {
            return Err(error_from_response(resp, &request_id).await);
        }
        let decode_started_at_ms = wasm_decode_probe_start(path);
        let decoded = resp.json::<T>().await;
        wasm_decode_probe_finish(decode_started_at_ms, decoded.is_ok());
        decoded.map_err(|error| ApiError::parse(error, Some(request_id)))
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.post_json_with_context(path, body, &MutationRequestContext::new())
            .await
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn post_json_quiet<
        B: Serialize,
        T: for<'de> Deserialize<'de>,
    >(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.post_json_with_context_quiet(path, body, &MutationRequestContext::new())
            .await
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn post_json_with_context<
        B: Serialize,
        T: for<'de> Deserialize<'de>,
    >(
        &self,
        path: &str,
        body: &B,
        context: &MutationRequestContext,
    ) -> Result<T, ApiError> {
        let result = self.post_json_with_context_quiet(path, body, context).await;
        log_failure("POST", path, &result);
        result
    }

    async fn post_json_with_context_quiet<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
        context: &MutationRequestContext,
    ) -> Result<T, ApiError> {
        let result = async {
            let url = self.url(path);
            let request_id = context.request_id();
            let mut request = self.request_headers(Request::post(&url), request_id);
            if let Some(idempotency_key) = context.idempotency_key() {
                request = request.header(HEADER_IDEMPOTENCY_KEY, idempotency_key);
            }
            let resp = request
                .json(body)
                .map_err(|error| ApiError::encode(error, Some(request_id.to_owned())))?
                .send()
                .await
                .map_err(|error| ApiError::network(error, Some(request_id.to_owned())))?;
            if !resp.ok() {
                return Err(error_from_response(resp, request_id).await);
            }
            resp.json::<T>()
                .await
                .map_err(|error| ApiError::parse(error, Some(request_id.to_owned())))
        }
        .await;
        result
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn post_json_with_idempotency_key<
        B: Serialize,
        T: for<'de> Deserialize<'de>,
    >(
        &self,
        path: &str,
        body: &B,
        idempotency_key: &str,
    ) -> Result<T, ApiError> {
        self.post_json_with_context(
            path,
            body,
            &MutationRequestContext::with_idempotency_key(idempotency_key),
        )
        .await
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn patch_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.patch_json_with_context(path, body, &MutationRequestContext::new())
            .await
    }

    #[inline(never)]
    pub(in crate::api::rest) async fn patch_json_with_context<
        B: Serialize,
        T: for<'de> Deserialize<'de>,
    >(
        &self,
        path: &str,
        body: &B,
        context: &MutationRequestContext,
    ) -> Result<T, ApiError> {
        let result = async {
            let url = self.url(path);
            let request_id = context.request_id();
            let mut request = self.request_headers(Request::patch(&url), request_id);
            if let Some(idempotency_key) = context.idempotency_key() {
                request = request.header(HEADER_IDEMPOTENCY_KEY, idempotency_key);
            }
            let resp = request
                .json(body)
                .map_err(|error| ApiError::encode(error, Some(request_id.to_owned())))?
                .send()
                .await
                .map_err(|error| ApiError::network(error, Some(request_id.to_owned())))?;
            if !resp.ok() {
                return Err(error_from_response(resp, request_id).await);
            }
            resp.json::<T>()
                .await
                .map_err(|error| ApiError::parse(error, Some(request_id.to_owned())))
        }
        .await;
        log_failure("PATCH", path, &result);
        result
    }

    pub(in crate::api::rest) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }

    fn request_headers(&self, request: RequestBuilder, request_id: &str) -> RequestBuilder {
        let request = request.header(HEADER_REQUEST_ID, request_id);
        match (self.auth_token)() {
            Some(token) => request.header(HEADER_AUTHORIZATION, &format!("Bearer {token}")),
            None => request,
        }
    }
}

fn log_failure<T>(method: &str, path: &str, result: &Result<T, ApiError>) {
    if let Err(error) = result {
        leptos::logging::error!("api {method} {path} failed: {error}");
    }
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod tests;
