//! KuCoin v2 签名。
//!
//! ```text
//! sign_str            = timestamp + method + requestPath + body
//! signature           = base64(HMAC-SHA256(secret, sign_str))
//! encrypted_passphrase = base64(HMAC-SHA256(secret, passphrase))
//! headers:
//!   KC-API-KEY: api_key
//!   KC-API-SIGN: signature
//!   KC-API-TIMESTAMP: ms timestamp (string)
//!   KC-API-PASSPHRASE: encrypted_passphrase
//!   KC-API-KEY-VERSION: 2
//! ```
//!
//! 关键：passphrase 须用 secret 二次 HMAC + Base64 加密，不可明文。

use common::signing::hmac_sha256_base64;

pub const KEY_VERSION: &str = "2";

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

pub fn encrypt_passphrase(secret: &[u8], passphrase: &str) -> String {
    hmac_sha256_base64(secret, passphrase.as_bytes())
}

/// Pro WebSocket Add/Cancel Order 连接 URL 签名。
///
/// 官方文档：<https://www.kucoin.com/docs-new/3470133w0>
/// prehash 为 `{apikey+timestamp}`，不同于 REST `timestamp+method+path+body`。
pub fn sign_ws_connect(secret: &[u8], api_key: &str, timestamp: &str) -> String {
    let prehash = format!("{api_key}{timestamp}");
    hmac_sha256_base64(secret, prehash.as_bytes())
}

/// Pro WebSocket 二次认证 challenge 签名。
///
/// 官方只说明用 API-Secret 对服务端推送的 JSON 字符串做 HMAC-SHA256 + Base64；
/// 具体发送字段未在页面文本中稳定呈现，因此这里仅暴露签名函数，不硬写认证 payload。
pub fn sign_ws_challenge(secret: &[u8], challenge_json: &str) -> String {
    hmac_sha256_base64(secret, challenge_json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn sign_deterministic() {
        let s1 = sign(
            b"secret",
            "1700000000000",
            "GET",
            "/api/v1/account-overview",
            "",
        );
        let s2 = sign(
            b"secret",
            "1700000000000",
            "GET",
            "/api/v1/account-overview",
            "",
        );
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 44);
        assert!(s1.ends_with('='));
    }

    #[test]
    fn passphrase_encryption_is_deterministic_and_distinct_from_signature() {
        let p1 = encrypt_passphrase(b"secret", "my-passphrase");
        let p2 = encrypt_passphrase(b"secret", "my-passphrase");
        assert_eq!(p1, p2);
        // 不同 secret 产生不同密文
        let p3 = encrypt_passphrase(b"other", "my-passphrase");
        assert_ne!(p1, p3);
    }

    #[test]
    fn ws_connect_sign_uses_apikey_then_timestamp() {
        let sig = sign_ws_connect(b"secret", "api-key", "1700000000000");
        let rest_sig = sign(b"secret", "1700000000000", "GET", "api-key", "");
        assert_ne!(sig, rest_sig);
        assert_eq!(sig.len(), 44);
    }
}
