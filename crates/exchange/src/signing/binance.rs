//! Binance 签名。
//!
//! ```text
//! 1. 拼 query string（包含 timestamp）
//! 2. signature = HMAC-SHA256(secret, query_string).hex()
//! 3. 追加 &signature=...
//! 4. 头部 X-MBX-APIKEY: {api_key}
//! ```

use common::signing::hmac_sha256_hex;

/// 计算 Binance 风格签名（HMAC-SHA256 → 十六进制小写）。
pub fn sign_query(secret: &[u8], query: &str) -> String {
    hmac_sha256_hex(secret, query.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Binance 官方文档示例：
    /// <https://binance-docs.github.io/apidocs/futures/en/#signed-trade-and-user_data-endpoints>
    #[test]
    fn official_docs_example() {
        let secret = b"NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";
        let query = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        let expected = "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71";
        assert_eq!(sign_query(secret, query), expected);
    }
}
