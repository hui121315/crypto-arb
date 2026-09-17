use super::*;

pub(super) struct OrderbookQuote {
    pub(super) book: Option<OrderBookInfo>,
    pub(super) executable: bool,
    pub(super) quality: MarketQuality,
    pub(super) source: crate::services::market_data::MarketSource,
    pub(super) freshness_ms: Option<i64>,
    pub(super) retry_after_ms: Option<i64>,
    pub(super) last_error: Option<String>,
    pub(super) blockers: Vec<String>,
}

impl OrderbookQuote {
    pub(super) fn display_book(&self) -> Option<&OrderBookInfo> {
        self.book.as_ref()
    }

    pub(super) fn executable_book(&self) -> Option<&OrderBookInfo> {
        self.executable.then_some(self.book.as_ref()).flatten()
    }

    pub(super) fn health(&self, now_ms: i64) -> MarketDataHealth {
        MarketDataHealth {
            quality: crate::services::market_data::envelope::quality(self.quality),
            source: crate::services::market_data::envelope::source(self.source),
            freshness_ms: self.freshness_ms,
            retry_after_ms: self
                .retry_after_ms
                .and_then(|ms| u64::try_from(ms.max(0)).ok()),
            last_error: self.last_error.clone(),
            observed_at_ms: self
                .book
                .as_ref()
                .map(|book| book.timestamp)
                .unwrap_or(now_ms),
            coverage: Some(MarketDataCoverage::new(1, u64::from(self.book.is_some()))),
            problem: None,
        }
    }
}

pub(super) fn orderbook_quote_from_read(
    exchange: &str,
    symbol: &str,
    read: MarketRead<OrderBookInfo>,
) -> OrderbookQuote {
    let executable = read.quality == MarketQuality::Fresh && read.source == MarketSource::WsPush;
    let blockers = orderbook_blockers(exchange, symbol, &read);
    OrderbookQuote {
        book: read.value,
        executable,
        quality: read.quality,
        source: read.source,
        freshness_ms: read.freshness_ms,
        retry_after_ms: read.retry_after_ms,
        last_error: read.last_error,
        blockers,
    }
}

fn orderbook_blockers(
    exchange: &str,
    symbol: &str,
    read: &MarketRead<OrderBookInfo>,
) -> Vec<String> {
    match read.quality {
        MarketQuality::Fresh if read.source == MarketSource::WsPush => Vec::new(),
        MarketQuality::Fresh => vec![format!(
            "{exchange} {symbol} orderbook 仅有 {} 证据，等待按需 WS 深度首帧",
            market_source_label(read.source)
        )],
        MarketQuality::Warming => vec![format!("{exchange} {symbol} orderbook WS 首帧预热中")],
        MarketQuality::StaleAllowed => vec![stale_orderbook_blocker(exchange, symbol, read)],
        MarketQuality::RateLimited => vec![retry_after_blocker(exchange, symbol, read)],
        MarketQuality::CircuitOpen => vec![format!(
            "{exchange} {symbol} orderbook 暂停刷新：交易所熔断中"
        )],
        MarketQuality::Unsupported => vec![format!(
            "{exchange} {symbol} adapter 未注册或不支持 orderbook"
        )],
        MarketQuality::Missing => vec![format!("{exchange} {symbol} orderbook 暂无可用数据")],
    }
}

fn market_source_label(source: crate::services::market_data::MarketSource) -> &'static str {
    match source {
        crate::services::market_data::MarketSource::WsPush => "ws",
        crate::services::market_data::MarketSource::RestColdStart => "rest-cold-start",
        crate::services::market_data::MarketSource::RestBaseline => "rest-baseline",
        crate::services::market_data::MarketSource::LocalCache => "cache",
    }
}

pub(super) fn stale_orderbook_blocker(
    exchange: &str,
    symbol: &str,
    read: &MarketRead<OrderBookInfo>,
) -> String {
    let age = read
        .freshness_ms
        .map(|milliseconds| format!("{}ms", milliseconds.max(0)))
        .unwrap_or_else(|| "未知".to_owned());
    match &read.last_error {
        Some(error) => format!(
            "{exchange} {symbol} orderbook 使用短时缓存({})，盘口年龄 {age}，最新刷新失败: {error}",
            market_source_label(read.source)
        ),
        None => format!(
            "{exchange} {symbol} orderbook 使用短时缓存({})，盘口年龄 {age}",
            market_source_label(read.source)
        ),
    }
}

pub(super) fn retry_after_blocker(
    exchange: &str,
    symbol: &str,
    read: &MarketRead<OrderBookInfo>,
) -> String {
    match read.retry_after_ms {
        Some(milliseconds) => format!(
            "{exchange} {symbol} orderbook 触发限频退避，{}ms 后重试",
            milliseconds.max(0)
        ),
        None => format!("{exchange} {symbol} orderbook 触发限频退避，重试时间未知"),
    }
}
