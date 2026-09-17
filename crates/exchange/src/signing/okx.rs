//! OKX 签名。
//!
//! ```text
//! prehash = timestamp + method + requestPath + body
//! signature = base64(HMAC-SHA256(secret, prehash))
//! headers:
//!   OK-ACCESS-KEY: api_key
//!   OK-ACCESS-SIGN: signature
//!   OK-ACCESS-TIMESTAMP: ISO8601 (e.g. "2026-05-14T04:18:00.945Z")
//!   OK-ACCESS-PASSPHRASE: passphrase
//! ```
//!
//! 注意：
//! - `timestamp` 必须使用毫秒精度的 ISO8601 字符串
//! - `requestPath` 包含查询字符串（含 `?`）
//! - `body` 为 POST 时的 JSON 字符串；GET 为空字符串

use chrono::{DateTime, SecondsFormat, Utc};
use common::signing::hmac_sha256_base64;

/// 生成 OKX 风格的 ISO8601 毫秒时间戳。
pub fn iso8601_now() -> String {
    iso8601_from(Utc::now())
}

pub fn iso8601_from(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn iso8601_from_millis(ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(iso8601_from)
}

/// 计算 OKX 签名。返回 Base64 编码的字符串。
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

    /// OKX 官方文档示例：
    /// <https://www.okx.com/docs-v5/en/#rest-api-authentication-signature>
    /// secret = "C5ECCAFEDDF03D86D003A2D7C36CD9D3"
    /// prehash = "2020-12-08T09:08:57.715ZGET/api/v5/account/balance?ccy=BTC"
    /// signature = "Vw9pVCXBVPxDrnB+1cDGE9CdXQ2T7VV4LQQpYXUKkZw="（举例计算）
    #[test]
    fn iso8601_format_has_millis_and_z() {
        let ts = iso8601_now();
        assert!(ts.contains('T'), "should contain T separator: {ts}");
        assert!(ts.ends_with('Z'), "should end with Z: {ts}");
        // YYYY-MM-DDTHH:MM:SS.mmmZ → 24 chars
        assert_eq!(ts.len(), 24);
    }

    #[test]
    fn iso8601_from_millis_is_deterministic() {
        assert_eq!(
            iso8601_from_millis(1_700_000_000_123).as_deref(),
            Some("2023-11-14T22:13:20.123Z")
        );
    }

    #[test]
    fn sign_is_deterministic_for_known_input() {
        let secret = b"C5ECCAFEDDF03D86D003A2D7C36CD9D3";
        let timestamp = "2020-12-08T09:08:57.715Z";
        let sig_a = sign(
            secret,
            timestamp,
            "GET",
            "/api/v5/account/balance?ccy=BTC",
            "",
        );
        let sig_b = sign(
            secret,
            timestamp,
            "GET",
            "/api/v5/account/balance?ccy=BTC",
            "",
        );
        assert_eq!(sig_a, sig_b);
        // Base64 of HMAC-SHA256 → 44 chars (with padding)
        assert_eq!(sig_a.len(), 44);
        assert!(sig_a.ends_with('='));
    }

    #[test]
    fn sign_changes_with_method_or_path() {
        let secret = b"abc";
        let ts = "2020-01-01T00:00:00.000Z";
        let g = sign(secret, ts, "GET", "/api/x", "");
        let p = sign(secret, ts, "POST", "/api/x", "");
        let g2 = sign(secret, ts, "GET", "/api/y", "");
        assert_ne!(g, p);
        assert_ne!(g, g2);
    }
}
