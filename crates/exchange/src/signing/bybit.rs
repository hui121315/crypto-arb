//! Bybit v5 签名。
//!
//! ```text
//! sign_str  = timestamp + api_key + recv_window + queryString_or_body
//! signature = HMAC-SHA256(secret, sign_str).hex()
//! headers:
//!   X-BAPI-API-KEY: api_key
//!   X-BAPI-TIMESTAMP: ms timestamp (string)
//!   X-BAPI-RECV-WINDOW: 5000
//!   X-BAPI-SIGN: signature
//! ```
//!
//! 注意：
//! - GET 请求 `query_or_body` 是排序后的 query string（不含 `?`）
//! - POST 请求 `query_or_body` 是 JSON body 字符串

use common::signing::hmac_sha256_hex;

pub const DEFAULT_RECV_WINDOW: &str = "5000";

pub fn sign(
    secret: &[u8],
    timestamp: &str,
    api_key: &str,
    recv_window: &str,
    query_or_body: &str,
) -> String {
    let prehash = format!("{timestamp}{api_key}{recv_window}{query_or_body}");
    hmac_sha256_hex(secret, prehash.as_bytes())
}

pub fn ws_auth_sign(secret: &[u8], expires: &str) -> String {
    let prehash = format!("GET/realtime{expires}");
    hmac_sha256_hex(secret, prehash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn deterministic_for_same_input() {
        let s1 = sign(b"secret", "1700000000000", "key", "5000", "category=linear");
        let s2 = sign(b"secret", "1700000000000", "key", "5000", "category=linear");
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 64); // hex of 32 bytes
    }

    #[test]
    fn changes_with_any_field() {
        let base = sign(b"s", "1", "k", "5000", "q");
        assert_ne!(base, sign(b"s", "2", "k", "5000", "q"));
        assert_ne!(base, sign(b"s", "1", "kk", "5000", "q"));
        assert_ne!(base, sign(b"s", "1", "k", "5001", "q"));
        assert_ne!(base, sign(b"s", "1", "k", "5000", "qq"));
    }

    #[test]
    fn ws_auth_signature_is_hex_hmac() {
        let signature = ws_auth_sign(b"secret", "1700000000000");

        assert_eq!(signature.len(), 64);
        assert_ne!(signature, ws_auth_sign(b"secret", "1700000000001"));
    }
}
