//! 期权市场数据与 Greeks 数据。

use serde::{Deserialize, Serialize};

use crate::OptionType;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionMarketQuote {
    pub venue: String,
    pub symbol: String,
    pub underlying: String,
    pub option_type: OptionType,
    pub strike: f64,
    pub expiry_ms: i64,
    pub bid: f64,
    pub ask: f64,
    pub mark: f64,
    pub volume_24h: f64,
    pub timestamp_ms: i64,
    #[serde(default)]
    pub delta: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionGreeks {
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    /// 隐含波动率。
    pub iv: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionGreeksRequest {
    pub spot_price: f64,
    pub strike: f64,
    pub days_to_expiry: f64,
    pub risk_free_rate: f64,
    pub volatility: f64,
    pub option_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionPriceRequest {
    pub spot_price: f64,
    pub strike: f64,
    pub days_to_expiry: f64,
    pub risk_free_rate: f64,
    pub volatility: f64,
    pub option_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionPriceResponse {
    pub price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionIvRequest {
    pub market_price: f64,
    pub spot_price: f64,
    pub strike: f64,
    pub days_to_expiry: f64,
    pub risk_free_rate: f64,
    pub option_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionIvResponse {
    pub iv: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_price_request_uses_camel_case_wire_fields() {
        let request = OptionPriceRequest {
            spot_price: 100.0,
            strike: 105.0,
            days_to_expiry: 30.0,
            risk_free_rate: 0.02,
            volatility: 0.4,
            option_type: "call".to_owned(),
        };

        let text = serde_json::to_string(&request).unwrap_or_default();

        assert!(text.contains("\"spotPrice\""), "json: {text}");
        assert!(text.contains("\"daysToExpiry\""), "json: {text}");
        assert!(text.contains("\"riskFreeRate\""), "json: {text}");
    }

    #[test]
    fn greeks_request_and_response_round_trip_camel_case() {
        let request: OptionGreeksRequest = serde_json::from_str(
            r#"{"spotPrice":100.0,"strike":95.0,"daysToExpiry":14.0,"riskFreeRate":0.03,"volatility":0.5,"optionType":"put"}"#,
        )
        .expect("deserialize greeks request");
        assert_eq!(request.option_type, "put");
        assert_eq!(request.days_to_expiry, 14.0);

        let greeks = OptionGreeks {
            delta: -0.4,
            gamma: 0.02,
            theta: -0.05,
            vega: 0.1,
            rho: -0.01,
            iv: 0.5,
        };
        let json = serde_json::to_value(greeks).expect("serialize greeks");
        assert_eq!(json["delta"], -0.4);
        assert_eq!(json["iv"], 0.5);
    }

    #[test]
    fn iv_request_uses_camel_case_and_response_is_flat() {
        let request: OptionIvRequest = serde_json::from_str(
            r#"{"marketPrice":6.2,"spotPrice":100.0,"strike":100.0,"daysToExpiry":7.0,"riskFreeRate":0.02,"optionType":"call"}"#,
        )
        .expect("deserialize iv request");
        assert_eq!(request.market_price, 6.2);

        let json = serde_json::to_value(OptionIvResponse { iv: 0.42 }).expect("serialize iv");
        assert_eq!(json["iv"], 0.42);
        let json =
            serde_json::to_value(OptionPriceResponse { price: 6.1 }).expect("serialize price");
        assert_eq!(json["price"], 6.1);
    }

    #[test]
    fn market_quote_delta_defaults_when_missing() {
        let quote: OptionMarketQuote = serde_json::from_str(
            r#"{"venue":"deribit","symbol":"BTC-27JUN26-100000-C","underlying":"BTC","optionType":"call","strike":100000.0,"expiryMs":1,"bid":0.01,"ask":0.02,"mark":0.015,"volume24h":12.0,"timestampMs":2}"#,
        )
        .expect("deserialize market quote without delta");

        assert_eq!(quote.delta, 0.0);
        assert_eq!(quote.underlying, "BTC");
    }
}
