use common::time::now_ms;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use shared_types::SpotTick;

pub(super) const EXCHANGE: &str = "kucoin";
const SNAPSHOT_TOPIC_PREFIX: &str = "/market/snapshot";
const TICKER_TOPIC_PREFIX: &str = "/market/ticker";
const ALL_TICKERS_TOPIC: &str = "/market/ticker:all";

#[derive(Debug, Clone)]
pub(super) struct CachedSpotTicker {
    pub(super) bid: String,
    pub(super) ask: String,
    pub(super) last: String,
    pub(super) bid_size: String,
    pub(super) ask_size: String,
    pub(super) volume_24h: Option<String>,
    pub(super) data_timestamp_ms: i64,
    pub(super) cached_at_ms: i64,
}

#[derive(Debug)]
pub(super) struct ParsedSpotTicker {
    pub(super) symbol: String,
    pub(super) cached: CachedSpotTicker,
}

pub(super) fn command_payload(kind: &str, symbol: &str) -> String {
    json!({
        "id": format!("spot-ticker-{symbol}-{}", now_ms()),
        "type": kind,
        "topic": snapshot_topic(symbol),
        "response": true,
    })
    .to_string()
}

pub(super) fn all_tickers_command_payload(kind: &str) -> String {
    json!({
        "id": format!("spot-all-bbo-{}", now_ms()),
        "type": kind,
        "topic": ALL_TICKERS_TOPIC,
        "response": true,
    })
    .to_string()
}

pub(super) fn parse_spot_ticker(text: &str) -> Option<ParsedSpotTicker> {
    let envelope: TickerEnvelope = serde_json::from_str(text).ok()?;
    let symbol = if envelope.topic == ALL_TICKERS_TOPIC {
        envelope.subject
    } else {
        envelope.topic.split_once(':')?.1.to_owned()
    };
    if symbol.is_empty() {
        return None;
    }
    let data = envelope.data?;
    if envelope.topic.starts_with(SNAPSHOT_TOPIC_PREFIX) {
        return parse_snapshot(symbol, data);
    }
    if envelope.topic.starts_with(TICKER_TOPIC_PREFIX) {
        return parse_ticker(symbol, data);
    }
    None
}

fn parse_snapshot(symbol: String, data: serde_json::Value) -> Option<ParsedSpotTicker> {
    let data: SnapshotOuter = serde_json::from_value(data).ok()?;
    let data = data.data?;
    let now = now_ms();
    let data_timestamp_ms = data.datetime.filter(|value| *value > 0)?;
    let bid = data.buy?.into_string();
    let ask = data.sell?.into_string();
    let last = data.last_traded_price?.into_string();
    let bid_size = data.bid_size?.into_string();
    let ask_size = data.ask_size?.into_string();
    let volume_24h = data
        .vol_value
        .or_else(|| data.market_change_24h.and_then(|window| window.vol_value))
        .map(DecimalText::into_string);
    positive_decimal(&bid)?;
    positive_decimal(&ask)?;
    positive_decimal(&last)?;
    nonnegative_decimal(&bid_size)?;
    nonnegative_decimal(&ask_size)?;
    if let Some(volume) = volume_24h.as_deref() {
        nonnegative_decimal(volume)?;
    }
    Some(ParsedSpotTicker {
        symbol,
        cached: CachedSpotTicker {
            bid,
            ask,
            last,
            bid_size,
            ask_size,
            volume_24h,
            data_timestamp_ms,
            cached_at_ms: now,
        },
    })
}

fn parse_ticker(symbol: String, data: serde_json::Value) -> Option<ParsedSpotTicker> {
    let data: TickerData = serde_json::from_value(data).ok()?;
    let now = now_ms();
    let data_timestamp_ms = data.time.filter(|value| *value > 0)?;
    let bid = data.best_bid?.into_string();
    let ask = data.best_ask?.into_string();
    let last = data.price?.into_string();
    let bid_size = data.best_bid_size?.into_string();
    let ask_size = data.best_ask_size?.into_string();
    positive_decimal(&bid)?;
    positive_decimal(&ask)?;
    positive_decimal(&last)?;
    nonnegative_decimal(&bid_size)?;
    nonnegative_decimal(&ask_size)?;
    Some(ParsedSpotTicker {
        symbol,
        cached: CachedSpotTicker {
            bid,
            ask,
            last,
            bid_size,
            ask_size,
            volume_24h: None,
            data_timestamp_ms,
            cached_at_ms: now,
        },
    })
}

pub(super) fn merge_cached_spot_ticker(current: &mut CachedSpotTicker, incoming: CachedSpotTicker) {
    if incoming.data_timestamp_ms >= current.data_timestamp_ms {
        let volume_24h = incoming
            .volume_24h
            .clone()
            .or_else(|| current.volume_24h.clone());
        *current = incoming;
        current.volume_24h = volume_24h;
    } else if current.volume_24h.is_none() && incoming.volume_24h.is_some() {
        current.volume_24h = incoming.volume_24h;
    }
}

pub(super) fn spot_tick_from_cached(symbol: &str, row: &CachedSpotTicker) -> Option<SpotTick> {
    let (base, quote) = crate::spot::delimited_pair(symbol, '-')?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: EXCHANGE,
        base: &base,
        quote: &quote,
        bid: &row.bid,
        ask: &row.ask,
        last: &row.last,
        bid_size: Some(&row.bid_size),
        ask_size: Some(&row.ask_size),
        volume_24h: row.volume_24h.as_deref().unwrap_or("0"),
        exchange_ts_ms: (row.data_timestamp_ms > 0).then_some(row.data_timestamp_ms),
    })
}

fn snapshot_topic(symbol: &str) -> String {
    format!("{SNAPSHOT_TOPIC_PREFIX}:{symbol}")
}

fn positive_decimal(value: &str) -> Option<Decimal> {
    value
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value > Decimal::ZERO)
}

fn nonnegative_decimal(value: &str) -> Option<Decimal> {
    value
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value >= Decimal::ZERO)
}

#[derive(Debug, Deserialize)]
struct TickerEnvelope {
    #[serde(default)]
    topic: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SnapshotOuter {
    #[serde(default)]
    data: Option<SnapshotData>,
}

#[derive(Debug, Deserialize)]
struct SnapshotData {
    #[serde(default)]
    buy: Option<DecimalText>,
    #[serde(default)]
    sell: Option<DecimalText>,
    #[serde(default, rename = "lastTradedPrice")]
    last_traded_price: Option<DecimalText>,
    #[serde(default, rename = "bidSize")]
    bid_size: Option<DecimalText>,
    #[serde(default, rename = "askSize")]
    ask_size: Option<DecimalText>,
    #[serde(default)]
    datetime: Option<i64>,
    #[serde(default, rename = "volValue")]
    vol_value: Option<DecimalText>,
    #[serde(default, rename = "marketChange24h")]
    market_change_24h: Option<MarketWindow>,
}

#[derive(Debug, Deserialize)]
struct TickerData {
    #[serde(default)]
    price: Option<DecimalText>,
    #[serde(default, rename = "bestBid")]
    best_bid: Option<DecimalText>,
    #[serde(default, rename = "bestAsk")]
    best_ask: Option<DecimalText>,
    #[serde(default, rename = "bestBidSize")]
    best_bid_size: Option<DecimalText>,
    #[serde(default, rename = "bestAskSize")]
    best_ask_size: Option<DecimalText>,
    #[serde(default)]
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MarketWindow {
    #[serde(default, rename = "volValue")]
    vol_value: Option<DecimalText>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DecimalText {
    Text(String),
    Number(serde_json::Number),
}

impl DecimalText {
    fn into_string(self) -> String {
        match self {
            Self::Text(value) => value,
            Self::Number(value) => value.to_string(),
        }
    }
}
