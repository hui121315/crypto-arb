use rust_decimal::Decimal;
use serde::Deserialize;
use shared_types::stocks::*;
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct Security {
    asset: String,
    name: String,
    cusip: Option<String>,
    sessions: Vec<StockSession>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Market {
    symbol: String,
    base_symbol: String,
    quote_symbol: String,
    market_type: String,
    rwa_market_type: Option<String>,
    order_book_state: String,
    filters: Filters,
}
#[derive(Deserialize)]
struct Filters {
    price: PriceFilter,
    quantity: QuantityFilter,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceFilter {
    tick_size: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuantityFilter {
    min_quantity: String,
    step_size: String,
}

pub(super) fn catalog(securities: &[u8], markets: &[u8], now: i64) -> Result<StockCatalog, String> {
    let securities: Vec<Security> =
        serde_json::from_slice(securities).map_err(|e| format!("Backpack securities: {e}"))?;
    let markets: Vec<Market> =
        serde_json::from_slice(markets).map_err(|e| format!("Backpack markets: {e}"))?;
    let mut identities = BTreeSet::new();
    let mut rows = Vec::with_capacity(securities.len());
    for s in securities {
        let ticker = s
            .asset
            .strip_suffix(".US")
            .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'))
            .ok_or("证券代码不符合官方 .US 身份，不能推导股票订阅")?
            .to_owned();
        if !identities.insert(s.asset.clone()) {
            return Err("证券身份重复".into());
        }
        let order_books = markets
            .iter()
            .filter(|m| {
                m.base_symbol == s.asset
                    && m.market_type == "SPOT"
                    && m.rwa_market_type.as_deref() == Some("STOCK")
            })
            .map(|m| StockOrderBookMarket {
                symbol: m.symbol.clone(),
                quote: m.quote_symbol.clone(),
                state: m.order_book_state.clone(),
                tick_size: m.filters.price.tick_size.clone(),
                min_quantity: m.filters.quantity.min_quantity.clone(),
                step_size: m.filters.quantity.step_size.clone(),
            })
            .collect();
        rows.push(StockSecurity {
            rfq_symbol: format!("{}_USDC_RFQ", s.asset),
            asset: s.asset,
            ticker,
            name: s.name,
            cusip: s.cusip,
            sessions: s.sessions,
            order_books,
        });
    }
    if rows.is_empty() {
        return Err("Backpack 未返回任何证券，不能覆盖已有目录".into());
    }
    rows.sort_by(|a, b| a.asset.cmp(&b.asset));
    Ok(StockCatalog {
        rows,
        observed_at_ms: now,
    })
}

pub(super) fn tokens(bytes: &[u8], security: &str) -> Result<Vec<StockChainToken>, String> {
    asset_context(bytes, security).map(|(tokens, _)| tokens)
}

pub(super) fn asset_context(
    bytes: &[u8],
    security: &str,
) -> Result<(Vec<StockChainToken>, Vec<StockFundingAsset>), String> {
    #[derive(Deserialize)]
    struct Asset {
        symbol: String,
        tokens: Vec<StockChainToken>,
    }
    let assets = serde_json::from_slice::<Vec<Asset>>(bytes)
        .map_err(|e| format!("Backpack assets: {e}"))?
        .into_iter()
        .filter(|a| a.symbol == security || matches!(a.symbol.as_str(), "USDC" | "SOL"));
    let mut seen = BTreeSet::new();
    let mut tokens = vec![];
    let mut funding = vec![];
    for a in assets {
        if !seen.insert(a.symbol.clone()) {
            return Err("Backpack 股票/备款资产映射重复".into());
        }
        if a.symbol == security {
            tokens = a.tokens;
        } else {
            funding.push(StockFundingAsset {
                asset: a.symbol,
                tokens: a.tokens,
            });
        }
    }
    funding.sort_by(|a, b| a.asset.cmp(&b.asset));
    Ok((tokens, funding))
}

pub(super) fn streams(s: &StockSecurity) -> BTreeSet<String> {
    std::iter::once(format!("stockPrice.{}", s.ticker))
        .chain(
            s.order_books
                .iter()
                .map(|m| format!("bookTicker.{}", m.symbol)),
        )
        .collect()
}

#[derive(Deserialize)]
struct Envelope {
    stream: String,
    data: serde_json::Value,
}
#[derive(Deserialize)]
struct Book {
    e: String,
    s: String,
    a: Option<String>,
    #[serde(rename = "A")]
    ask_qty: Option<String>,
    b: Option<String>,
    #[serde(rename = "B")]
    bid_qty: Option<String>,
    u: UpdateId,
    #[serde(rename = "T")]
    time: i64,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum UpdateId {
    Number(u64),
    Text(String),
}
#[derive(Deserialize)]
struct Reference {
    e: String,
    symbol: String,
    bid: Option<String>,
    ask: Option<String>,
    mid: String,
    timestamp: i64,
    session: Option<String>,
}

fn price(value: Option<&str>) -> Result<(), String> {
    if value.is_some_and(|v| !Decimal::from_str_exact(v).is_ok_and(|v| v > Decimal::ZERO)) {
        return Err("股票价格或数量无效".into());
    }
    Ok(())
}

pub(super) fn apply(
    snapshot: &mut StockMarketSnapshot,
    text: &str,
    now: i64,
) -> Result<bool, String> {
    let envelope: Envelope = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            let value: serde_json::Value = serde_json::from_str(text).map_err(|_| {
                snapshot.problem = Some("Backpack WS JSON 无效".into());
                "Backpack WS JSON 无效"
            })?;
            if value.get("error").is_some() || value.get("code").is_some_and(|v| !v.is_null()) {
                snapshot.problem = Some("Backpack 拒绝行情订阅".into());
                return Err("Backpack 拒绝行情订阅".into());
            }
            return Ok(false);
        }
    };
    let Some(security) = &snapshot.security else {
        return Ok(false);
    };
    let conversion = envelope.stream == "bookTicker.USDT_USDC";
    if !conversion && !streams(security).contains(&envelope.stream) {
        return Ok(false);
    }
    let reference = envelope.stream.starts_with("stockPrice.");
    let result = apply_quote(snapshot, envelope, now);
    let problem = if conversion {
        &mut snapshot.conversion_book_problem
    } else if reference {
        &mut snapshot.reference_problem
    } else {
        &mut snapshot.problem
    };
    match &result {
        Ok(true) => *problem = None,
        Err(error) => *problem = Some(error.clone()),
        _ => {}
    }
    result
}

fn apply_quote(
    snapshot: &mut StockMarketSnapshot,
    envelope: Envelope,
    now: i64,
) -> Result<bool, String> {
    let Some(security) = &snapshot.security else {
        return Ok(false);
    };
    if envelope.stream.starts_with("bookTicker.") {
        let b: Book =
            serde_json::from_value(envelope.data).map_err(|e| format!("股票盘口解码：{e}"))?;
        if b.e != "bookTicker" || envelope.stream != format!("bookTicker.{}", b.s) {
            return Err("股票盘口市场身份不符".into());
        }
        for p in [&b.a, &b.b, &b.ask_qty, &b.bid_qty] {
            price(p.as_deref())?;
        }
        if b.a.is_some() != b.ask_qty.is_some() || b.b.is_some() != b.bid_qty.is_some() {
            return Err("股票盘口数量缺失".into());
        }
        if b.b
            .as_deref()
            .zip(b.a.as_deref())
            .is_some_and(|(bid, ask)| {
                Decimal::from_str_exact(bid).unwrap() > Decimal::from_str_exact(ask).unwrap()
            })
        {
            return Err("股票盘口买价高于卖价，等待有效行情".into());
        }
        let source_at_ms = b.time / 1000;
        if source_at_ms <= 0 || source_at_ms > now.saturating_add(2000) {
            return Err("股票盘口时戳无效".into());
        }
        let update_id = match b.u {
            UpdateId::Number(id) => id,
            UpdateId::Text(id) => id.parse::<u64>().map_err(|_| "股票盘口序号无效")?,
        };
        if snapshot
            .books
            .iter()
            .chain(snapshot.conversion_book.iter())
            .any(|q| q.symbol == b.s && (q.update_id >= update_id || q.source_at_ms > source_at_ms))
        {
            return Ok(false);
        }
        let row = StockBookQuote {
            symbol: b.s,
            bid: b.b,
            bid_quantity: b.bid_qty,
            ask: b.a,
            ask_quantity: b.ask_qty,
            update_id,
            source_at_ms,
            received_at_ms: now,
        };
        if row.symbol == STOCK_CONVERSION_SYMBOL {
            snapshot.conversion_book = Some(row);
        } else {
            snapshot.books.retain(|old| old.symbol != row.symbol);
            snapshot.books.push(row);
        }
    } else {
        let q: Reference =
            serde_json::from_value(envelope.data).map_err(|e| format!("股票参考行情解码：{e}"))?;
        if q.e != "stockPrice" || q.symbol != security.ticker {
            return Err("股票参考行情身份不符".into());
        }
        for p in [q.bid.as_deref(), q.ask.as_deref(), Some(q.mid.as_str())] {
            price(p)?;
        }
        if q.timestamp <= 0 || q.timestamp > now.saturating_add(2000) {
            return Err("股票参考行情时戳无效".into());
        }
        if snapshot
            .reference
            .as_ref()
            .is_some_and(|old| old.source_at_ms > q.timestamp)
        {
            return Ok(false);
        }
        snapshot.reference = Some(StockReferenceQuote {
            ticker: q.symbol,
            bid: q.bid,
            ask: q.ask,
            mid: q.mid,
            session: q.session,
            source_at_ms: q.timestamp,
            received_at_ms: now,
        });
    }
    snapshot.observed_at_ms = now.max(snapshot.observed_at_ms.saturating_add(1));
    Ok(true)
}

#[cfg(test)]
pub(super) mod tests;
