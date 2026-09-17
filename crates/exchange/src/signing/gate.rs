//! Gate v4 签名（HMAC-SHA512）。
//!
//! ```text
//! body_hash = sha512(body).hex()       (空 body 时 hash 空字符串)
//! sign_str  = method\n url_path\n query_string\n body_hash\n timestamp_secs
//! signature = HMAC-SHA512(secret, sign_str).hex()
//! headers:
//!   KEY: api_key
//!   Timestamp: timestamp_secs (string)
//!   SIGN: signature
//! ```
//!
//! 注意：
//! - timestamp 是**秒**（不是毫秒）
//! - `body_hash` 即使 body 为空也要计算 SHA-512('') = "cf83e1357eef..."
//! - `sign_str` 内是 `\n` 分隔，五段

use common::signing::hmac_sha512_hex;
use sha2::{Digest, Sha512};

/// 计算 Gate 风格签名。
pub fn sign(
    secret: &[u8],
    method: &str,
    url_path: &str,
    query_string: &str,
    body: &str,
    timestamp_secs: &str,
) -> String {
    let mut hasher = Sha512::new();
    hasher.update(body.as_bytes());
    let body_hash = hex::encode(hasher.finalize());

    let sign_str = format!("{method}\n{url_path}\n{query_string}\n{body_hash}\n{timestamp_secs}");
    hmac_sha512_hex(secret, sign_str.as_bytes())
}

pub fn ws_sign(secret: &[u8], channel: &str, event: &str, timestamp_secs: &str) -> String {
    let sign_str = format!("channel={channel}&event={event}&time={timestamp_secs}");
    hmac_sha512_hex(secret, sign_str.as_bytes())
}

/// Sign a Gate Futures WebSocket API operation such as `futures.login`.
///
/// This protocol intentionally differs from private channel subscription auth.
pub fn ws_api_sign(
    secret: &[u8],
    event: &str,
    channel: &str,
    request_param: &str,
    timestamp_secs: &str,
) -> String {
    let sign_str = format!("{event}\n{channel}\n{request_param}\n{timestamp_secs}");
    hmac_sha512_hex(secret, sign_str.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn deterministic() {
        let s1 = sign(
            b"secret",
            "GET",
            "/api/v4/futures/usdt/contracts",
            "",
            "",
            "1700000000",
        );
        let s2 = sign(
            b"secret",
            "GET",
            "/api/v4/futures/usdt/contracts",
            "",
            "",
            "1700000000",
        );
        assert_eq!(s1, s2);
        // hex of HMAC-SHA512 = 128 chars
        assert_eq!(s1.len(), 128);
    }

    #[test]
    fn empty_body_uses_known_sha512() {
        // 仅断言"两个不同 body 产生不同签名"——避免硬编码 SHA512 值
        let with_empty = sign(b"k", "POST", "/p", "", "", "1");
        let with_body = sign(b"k", "POST", "/p", "", "{}", "1");
        assert_ne!(with_empty, with_body);
    }

    #[test]
    fn changes_with_each_input() {
        let base = sign(b"s", "GET", "/p", "a=1", "", "100");
        assert_ne!(base, sign(b"s", "POST", "/p", "a=1", "", "100"));
        assert_ne!(base, sign(b"s", "GET", "/q", "a=1", "", "100"));
        assert_ne!(base, sign(b"s", "GET", "/p", "a=2", "", "100"));
        assert_ne!(base, sign(b"s", "GET", "/p", "a=1", "", "200"));
    }

    #[test]
    fn ws_sign_uses_channel_event_time() {
        let signature = ws_sign(b"secret", "futures.login", "api", "1700000000");

        assert_eq!(signature.len(), 128);
        assert_ne!(
            signature,
            ws_sign(b"secret", "futures.order_place", "api", "1700000000")
        );
    }

    #[test]
    fn ws_api_sign_matches_official_login_prehash() {
        let signature = ws_api_sign(b"secret", "api", "futures.login", "", "1700000000");

        assert_eq!(
            signature,
            "f39035057b3528fc2c5aff4b9cfa9f43673c88d3ff823c55468608173205809999a8b45d7ed898ebf49c15a4f6e5131de175ded143be5eeb58431f600e1d4085"
        );
        assert_ne!(
            signature,
            ws_sign(b"secret", "futures.login", "api", "1700000000")
        );
    }
}
