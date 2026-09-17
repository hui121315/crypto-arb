//! WebSocket / Pub-Sub 频道名解析。
//!
//! 与 Python 旧版 channel 约定保持一致：
//! - `arbitrage`：套利机会快照
//! - `watchlist`：自选列表更新
//! - `portfolio`：持仓 / 风控聚合快照
//! - `market:{currency}`：单币种行情，例 `market:BTC`
//! - `ticker:{symbol}:{exchange}`：单家某币种行情，例 `ticker:BTC:binance`
//! - `funding:{symbol}`：单币种各家费率，例 `funding:BTC`

pub const ARBITRAGE: &str = "arbitrage";
pub const WATCHLIST: &str = "watchlist";
pub const FUNDING_RATES: &str = "funding-rates";
pub const ALERTS: &str = "alerts";
pub const ORDERS: &str = "orders";
pub const EXECUTION: &str = "execution";
pub const PORTFOLIO: &str = "portfolio";
pub const RISK_ALERTS: &str = "risk-alerts";
pub const SYSTEM: &str = "system";
pub const AUTOMATION: &str = "automation";
pub const ONCHAIN: &str = "onchain";
pub const STOCKS: &str = "stocks";
pub const REVIEW: &str = "review";
pub const WEBHOOK: &str = "webhook";
/// Backend-only wake signal. It is not an `AppWS` product channel.
pub const ACTION_RUN_ACTIVITY: &str = "internal:action-runs";
/// Backend-only wake signal. It is not an `AppWS` product channel.
pub const CLOSE_RUN_ACTIVITY: &str = "internal:close-runs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsChannelDelivery {
    Immediate,
    Batched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsChannelReplaySource {
    OpportunitySnapshot,
    WatchlistSnapshot,
    FundingRatesSnapshot,
    AlertRulesSnapshot,
    OrderSnapshot,
    ExecutionRunSnapshot,
    PortfolioSnapshot,
    RiskSnapshot,
    SystemHealthSnapshot,
    AutomationStatusSnapshot,
    OnchainComparisonSnapshot,
    StockMarketSnapshot,
    ReviewRuntimeSnapshot,
    WebhookRuntimeStatusSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsChannelFeature {
    Core,
    WatchlistAlerts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WsChannelSpec {
    pub name: &'static str,
    pub delivery: WsChannelDelivery,
    pub replay_source: WsChannelReplaySource,
    pub feature: WsChannelFeature,
    pub authenticated: bool,
}

pub const WS_CHANNEL_SPECS: [WsChannelSpec; 14] = [
    WsChannelSpec {
        name: STOCKS,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::StockMarketSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: ARBITRAGE,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::OpportunitySnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: WATCHLIST,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::WatchlistSnapshot,
        feature: WsChannelFeature::WatchlistAlerts,
        authenticated: true,
    },
    WsChannelSpec {
        name: FUNDING_RATES,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::FundingRatesSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: ALERTS,
        delivery: WsChannelDelivery::Immediate,
        replay_source: WsChannelReplaySource::AlertRulesSnapshot,
        feature: WsChannelFeature::WatchlistAlerts,
        authenticated: true,
    },
    WsChannelSpec {
        name: ORDERS,
        delivery: WsChannelDelivery::Immediate,
        replay_source: WsChannelReplaySource::OrderSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: EXECUTION,
        delivery: WsChannelDelivery::Immediate,
        replay_source: WsChannelReplaySource::ExecutionRunSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: PORTFOLIO,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::PortfolioSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: RISK_ALERTS,
        delivery: WsChannelDelivery::Immediate,
        replay_source: WsChannelReplaySource::RiskSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: SYSTEM,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::SystemHealthSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: AUTOMATION,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::AutomationStatusSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: ONCHAIN,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::OnchainComparisonSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: REVIEW,
        delivery: WsChannelDelivery::Batched,
        replay_source: WsChannelReplaySource::ReviewRuntimeSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
    WsChannelSpec {
        name: WEBHOOK,
        delivery: WsChannelDelivery::Immediate,
        replay_source: WsChannelReplaySource::WebhookRuntimeStatusSnapshot,
        feature: WsChannelFeature::Core,
        authenticated: true,
    },
];

pub fn ws_channel_spec(channel: &str) -> Option<&'static WsChannelSpec> {
    WS_CHANNEL_SPECS.iter().find(|spec| spec.name == channel)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    Arbitrage,
    Watchlist,
    FundingRates,
    Alerts,
    Orders,
    Execution,
    Portfolio,
    RiskAlerts,
    System,
    Automation,
    Onchain,
    Stocks,
    Review,
    Webhook,
    Market { currency: String },
    Ticker { symbol: String, exchange: String },
    Funding { symbol: String },
    Custom(String),
}

impl Topic {
    /// 将 Topic 渲染回 `channel:arg1:arg2` 字符串。
    pub fn as_channel(&self) -> String {
        match self {
            Topic::Arbitrage => ARBITRAGE.to_owned(),
            Topic::Watchlist => WATCHLIST.to_owned(),
            Topic::FundingRates => FUNDING_RATES.to_owned(),
            Topic::Alerts => ALERTS.to_owned(),
            Topic::Orders => ORDERS.to_owned(),
            Topic::Execution => EXECUTION.to_owned(),
            Topic::Portfolio => PORTFOLIO.to_owned(),
            Topic::RiskAlerts => RISK_ALERTS.to_owned(),
            Topic::System => SYSTEM.to_owned(),
            Topic::Automation => AUTOMATION.to_owned(),
            Topic::Onchain => ONCHAIN.to_owned(),
            Topic::Stocks => STOCKS.to_owned(),
            Topic::Review => REVIEW.to_owned(),
            Topic::Webhook => WEBHOOK.to_owned(),
            Topic::Market { currency } => format!("market:{}", currency.to_ascii_uppercase()),
            Topic::Ticker { symbol, exchange } => format!(
                "ticker:{}:{}",
                symbol.to_ascii_uppercase(),
                exchange.to_ascii_lowercase()
            ),
            Topic::Funding { symbol } => format!("funding:{}", symbol.to_ascii_uppercase()),
            Topic::Custom(s) => s.clone(),
        }
    }
}

/// 解析频道字符串为结构化 [`Topic`]。无效格式返回 `Topic::Custom`。
pub fn parse(channel: &str) -> Topic {
    let parts: Vec<&str> = channel.split(':').collect();
    match parts.as_slice() {
        [ARBITRAGE] => Topic::Arbitrage,
        [WATCHLIST] => Topic::Watchlist,
        [FUNDING_RATES] => Topic::FundingRates,
        [ALERTS] => Topic::Alerts,
        [ORDERS] => Topic::Orders,
        [EXECUTION] => Topic::Execution,
        [PORTFOLIO] => Topic::Portfolio,
        [RISK_ALERTS] => Topic::RiskAlerts,
        [SYSTEM] => Topic::System,
        [AUTOMATION] => Topic::Automation,
        [ONCHAIN] => Topic::Onchain,
        [STOCKS] => Topic::Stocks,
        [REVIEW] => Topic::Review,
        [WEBHOOK] => Topic::Webhook,
        ["market", currency] if !currency.is_empty() => Topic::Market {
            currency: currency.to_ascii_uppercase(),
        },
        ["ticker", symbol, exchange] if !symbol.is_empty() && !exchange.is_empty() => {
            Topic::Ticker {
                symbol: symbol.to_ascii_uppercase(),
                exchange: exchange.to_ascii_lowercase(),
            }
        }
        ["funding", symbol] if !symbol.is_empty() => Topic::Funding {
            symbol: symbol.to_ascii_uppercase(),
        },
        _ => Topic::Custom(channel.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_arbitrage() {
        assert_eq!(parse("arbitrage"), Topic::Arbitrage);
    }

    #[test]
    fn parses_watchlist() {
        assert_eq!(parse("watchlist"), Topic::Watchlist);
    }

    #[test]
    fn parses_named_runtime_channels() {
        assert_eq!(parse(FUNDING_RATES), Topic::FundingRates);
        assert_eq!(parse(ALERTS), Topic::Alerts);
        assert_eq!(parse(ORDERS), Topic::Orders);
        assert_eq!(parse(EXECUTION), Topic::Execution);
        assert_eq!(parse(PORTFOLIO), Topic::Portfolio);
        assert_eq!(parse(RISK_ALERTS), Topic::RiskAlerts);
        assert_eq!(parse(SYSTEM), Topic::System);
        assert_eq!(parse(AUTOMATION), Topic::Automation);
        assert_eq!(parse(ONCHAIN), Topic::Onchain);
        assert_eq!(parse(STOCKS), Topic::Stocks);
        assert_eq!(parse(REVIEW), Topic::Review);
        assert_eq!(parse(WEBHOOK), Topic::Webhook);
    }

    #[test]
    fn parses_market_uppercase_currency() {
        assert_eq!(
            parse("market:btc"),
            Topic::Market {
                currency: "BTC".into()
            }
        );
        assert_eq!(
            parse("market:ETH"),
            Topic::Market {
                currency: "ETH".into()
            }
        );
    }

    #[test]
    fn parses_ticker_normalization() {
        assert_eq!(
            parse("ticker:btc:BINANCE"),
            Topic::Ticker {
                symbol: "BTC".into(),
                exchange: "binance".into()
            }
        );
    }

    #[test]
    fn parses_funding() {
        assert_eq!(
            parse("funding:eth"),
            Topic::Funding {
                symbol: "ETH".into()
            }
        );
    }

    #[test]
    fn unknown_channel_becomes_custom() {
        assert_eq!(
            parse("foo:bar:baz:extra"),
            Topic::Custom("foo:bar:baz:extra".into())
        );
        assert_eq!(parse(""), Topic::Custom("".into()));
    }

    #[test]
    fn empty_argument_falls_back_to_custom() {
        assert_eq!(parse("market:"), Topic::Custom("market:".into()));
        assert_eq!(parse("funding:"), Topic::Custom("funding:".into()));
    }

    #[test]
    fn as_channel_round_trip() {
        let cases = [
            Topic::Arbitrage,
            Topic::Watchlist,
            Topic::FundingRates,
            Topic::Alerts,
            Topic::Orders,
            Topic::Execution,
            Topic::Portfolio,
            Topic::RiskAlerts,
            Topic::System,
            Topic::Automation,
            Topic::Onchain,
            Topic::Stocks,
            Topic::Review,
            Topic::Webhook,
            Topic::Market {
                currency: "BTC".into(),
            },
            Topic::Ticker {
                symbol: "BTC".into(),
                exchange: "binance".into(),
            },
            Topic::Funding {
                symbol: "ETH".into(),
            },
        ];
        for t in cases {
            let s = t.as_channel();
            let parsed = parse(&s);
            assert_eq!(parsed, t, "round-trip broken: {s}");
        }
    }

    #[test]
    fn product_channel_registry_is_unique_authenticated_and_replayable() {
        let mut names = WS_CHANNEL_SPECS
            .iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), WS_CHANNEL_SPECS.len());
        assert!(WS_CHANNEL_SPECS.iter().all(|spec| spec.authenticated));
        for spec in WS_CHANNEL_SPECS {
            assert_eq!(ws_channel_spec(spec.name), Some(&spec));
        }
    }

    #[test]
    fn legacy_dynamic_topics_are_not_product_channels_without_live_publishers() {
        for channel in ["market:BTC", "ticker:BTC:binance", "funding:BTC"] {
            assert!(!matches!(parse(channel), Topic::Custom(_)));
            assert!(ws_channel_spec(channel).is_none());
        }
    }
}
