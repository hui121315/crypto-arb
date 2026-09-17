//! Bitget UTA V3 REST 签名。
//!
//! ```text
//! prehash   = timestamp + method + requestPath + body
//! signature = base64(HMAC-SHA256(secret, prehash))
//! headers:
//!   ACCESS-KEY: api_key
//!   ACCESS-SIGN: signature
//!   ACCESS-TIMESTAMP: ms timestamp (string)
//!   ACCESS-PASSPHRASE: passphrase
//!   Content-Type: application/json
//! ```
//!
//! 与 OKX 极相似（仅 header 名称不同；OKX 用 ISO8601，Bitget 用 ms 数字字符串）。

use common::signing::hmac_sha256_base64;

pub fn sign(
    secret: &[u8],
    timestamp: &str,
    method: &str,
    request_path: &str,
    body: &str,
) -> String {
    let prehash = format!("{timestamp}{method}{request_path}{body}");
    hmac_sha256_base64(secret, prehash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn deterministic() {
        let s1 = sign(
            b"secret",
            "1700000000000",
            "GET",
            "/api/v3/account/assets",
            "",
        );
        let s2 = sign(
            b"secret",
            "1700000000000",
            "GET",
            "/api/v3/account/assets",
            "",
        );
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 44); // base64 of 32 bytes
        assert!(s1.ends_with('='));
    }

    #[test]
    fn changes_with_input() {
        let base = sign(b"s", "1", "GET", "/p", "");
        assert_ne!(base, sign(b"s", "1", "POST", "/p", ""));
        assert_ne!(base, sign(b"s", "1", "GET", "/q", ""));
        assert_ne!(base, sign(b"s", "2", "GET", "/p", ""));
        assert_ne!(base, sign(b"s", "1", "GET", "/p", "{}"));
    }
}
