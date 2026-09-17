//! 现货与跨现货套利 DTO。

use crate::instrument_coverage::InstrumentCoverageEntry;
use crate::list::ListPage;
use crate::problem::ApiProblem;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Default-off spot v1 诊断读取的请求契约。
///
/// `symbol` 保留旧客户端的“完整 symbol 或 base”查询语义；`base` /
/// `quote` 用于强制精确分解。`fresh_only` 只返回有 Fresh row evidence
/// 的观察数据，不把旧缓存伪装为当前行情。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotTicksQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, rename = "fresh_only", alias = "freshOnly")]
    pub fresh_only: bool,
}

impl SpotTicksQuery {
    #[must_use]
    pub fn for_symbol(symbol: impl AsRef<str>) -> Self {
        let symbol = symbol.as_ref().trim();
        Self {
            symbol: (!symbol.is_empty()).then(|| symbol.to_owned()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotTick {
    pub venue: String,
    pub symbol: String,
    pub bid: Decimal,
    pub ask: Decimal,
    pub last: Decimal,
    /// 最优买量。官方载荷未提供该字段时为 `None`，绝不伪造成 0。
    pub bid_size: Option<Decimal>,
    /// 最优卖量。官方载荷未提供该字段时为 `None`，绝不伪造成 0。
    pub ask_size: Option<Decimal>,
    pub volume_24h: Decimal,
    /// 交易所在官方载荷中给出的撮合/事件时间戳；载荷未提供时为 `None`，
    /// 不与本地落地时间合并。
    pub exchange_ts_ms: Option<i64>,
    /// 本地实际收到并解析该 tick 的时间，始终由 ingest 侧写入。
    pub received_at_ms: i64,
}

impl SpotTick {
    /// 估值/风控读取时间戳的统一入口：优先用交易所时间戳，缺失时回落本地落地时间。
    pub fn best_timestamp_ms(&self) -> i64 {
        self.exchange_ts_ms.unwrap_or(self.received_at_ms)
    }
}

/// Bounded spot v1 诊断响应主体。
///
/// `base_listing_coverage` 是 official instrument registry 对 canonical base 的挂牌
/// 证据，与 `row_evidence` 的行情新鲜度及 `fanout` 的运行结果分开传达。
/// 它不单独授予现货价格行任何可执行资格。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotTicksPage {
    #[serde(default)]
    pub ticks: Vec<SpotTick>,
    pub page: ListPage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_listing_coverage: Vec<InstrumentCoverageEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_problems: Vec<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossSpotSpread {
    pub symbol: String,
    pub buy_venue: String,
    pub sell_venue: String,
    pub buy_price: Decimal,
    pub sell_price: Decimal,
    pub gross_spread_bps: f64,
    pub net_spread_bps: f64,
    pub max_size_usd: Decimal,
    pub captured_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_ticks_query_keeps_snake_case_fresh_only_and_trims_legacy_symbol() {
        let query = SpotTicksQuery {
            base: Some("BTC".into()),
            quote: Some("USDT".into()),
            venue: Some("binance".into()),
            limit: Some(64),
            cursor: Some("32".into()),
            fresh_only: true,
            ..SpotTicksQuery::for_symbol(" BTCUSDT ")
        };

        let text = serde_json::to_string(&query).expect("serialize spot query");

        assert!(text.contains("\"symbol\":\"BTCUSDT\""));
        assert!(text.contains("\"fresh_only\":true"));
        assert!(!text.contains("freshOnly"));
    }

    #[test]
    fn spot_ticks_page_serializes_page_listing_problems_and_request_id() {
        let page = SpotTicksPage {
            page: ListPage {
                limit: 64,
                max_limit: 128,
                start_offset: 0,
                returned_count: 0,
                total_rows: 0,
                has_more: false,
                next_cursor: None,
                ..ListPage::default()
            },
            query_problems: vec![ApiProblem::new("LIST_CURSOR_INVALID", "bad cursor")],
            request_id: Some("req-spot-page".into()),
            ..SpotTicksPage::default()
        };

        let text = serde_json::to_string(&page).expect("serialize spot page");

        assert!(text.contains("\"queryProblems\""));
        assert!(text.contains("\"requestId\":\"req-spot-page\""));
        assert!(text.contains("\"maxLimit\":128"));
    }
}
