//! API Base 单一来源与 URL 转换。

use gloo_storage::Storage;

pub const API_BASE_STORAGE_KEY: &str = "api_base";
pub const API_AUTH_TOKEN_STORAGE_KEY: &str = "api_auth_token";
pub const DEFAULT_API_BASE: &str = "http://127.0.0.1:8000";

pub fn stored_or_default_api_base() -> String {
    gloo_storage::LocalStorage::get::<String>(API_BASE_STORAGE_KEY)
        .ok()
        .map(|base| normalize_api_base(&base))
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

pub fn store_api_base(base_url: &str) -> String {
    let normalized = normalize_api_base(base_url);
    let value = if normalized.is_empty() {
        DEFAULT_API_BASE.to_string()
    } else {
        normalized
    };
    let _ = gloo_storage::LocalStorage::set(API_BASE_STORAGE_KEY, &value);
    value
}

pub fn stored_api_auth_token() -> String {
    gloo_storage::LocalStorage::get::<String>(API_AUTH_TOKEN_STORAGE_KEY)
        .ok()
        .map(|token| normalize_api_auth_token(&token))
        .unwrap_or_default()
}

pub fn store_api_auth_token(token: &str) -> String {
    let normalized = normalize_api_auth_token(token);
    if normalized.is_empty() {
        gloo_storage::LocalStorage::delete(API_AUTH_TOKEN_STORAGE_KEY);
    } else {
        let _ = gloo_storage::LocalStorage::set(API_AUTH_TOKEN_STORAGE_KEY, &normalized);
    }
    normalized
}

pub fn normalize_api_base(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

pub fn normalize_api_auth_token(token: &str) -> String {
    token.trim().to_string()
}

pub fn ws_url_from_api_base(base_url: &str) -> String {
    let trimmed = normalize_api_base(base_url);
    let base = if trimmed.is_empty() {
        DEFAULT_API_BASE
    } else {
        trimmed.as_str()
    };
    let scheme_swapped = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{base}")
    };
    format!("{scheme_swapped}/ws?encoding=zlib-json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_spaces_and_trailing_slash() {
        assert_eq!(
            normalize_api_base(" http://127.0.0.1:8000/// "),
            "http://127.0.0.1:8000"
        );
    }

    #[test]
    fn websocket_url_uses_matching_scheme() {
        assert_eq!(
            ws_url_from_api_base("https://api.crossline.local/"),
            "wss://api.crossline.local/ws?encoding=zlib-json"
        );
        assert_eq!(
            ws_url_from_api_base("http://127.0.0.1:8000"),
            "ws://127.0.0.1:8000/ws?encoding=zlib-json"
        );
        assert_eq!(
            ws_url_from_api_base("127.0.0.1:8000"),
            "ws://127.0.0.1:8000/ws?encoding=zlib-json"
        );
    }

    #[test]
    fn auth_token_normalization_trims_and_allows_empty() {
        assert_eq!(normalize_api_auth_token(" secret "), "secret");
        assert_eq!(normalize_api_auth_token("   "), "");
    }
}
