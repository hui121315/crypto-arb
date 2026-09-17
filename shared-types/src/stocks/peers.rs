use super::{comparison::*, *};
use crate::{InstrumentListingStatus, VenueInstrument};
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPeerProduct {
    Spot,
    Perpetual,
}

impl StockPeerProduct {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Perpetual => "perpetual",
        }
    }
    pub fn matches(self, value: Option<&str>) -> bool {
        match self {
            Self::Spot => value == Some("spot"),
            Self::Perpetual => matches!(value, Some("perp" | "perpetual")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerSelection {
    pub venue: String,
    pub product: StockPeerProduct,
    pub native_symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerWatchRequest {
    pub asset: String,
    pub selection: Option<StockPeerSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerCatalogRequest {
    pub venue: String,
    pub product: StockPeerProduct,
    #[serde(default)]
    pub search: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerCatalog {
    pub request: StockPeerCatalogRequest,
    pub rows: Vec<VenueInstrument>,
    pub matched: usize,
    pub registry_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerIdentity {
    pub underlying_verified: bool,
    pub underlying_isin: Option<String>,
    pub product_isin: Option<String>,
    pub issuer: Option<String>,
    pub sources: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerQuote {
    pub symbol: String,
    pub bid: String,
    pub ask: String,
    pub bid_quantity: Option<String>,
    pub ask_quantity: Option<String>,
    pub source: String,
    pub source_at_ms: Option<i64>,
    pub received_at_ms: i64,
}

impl StockPeerQuote {
    pub fn fresh_ws(&self, now: i64) -> bool {
        self.source == "ws_push"
            && now >= self.received_at_ms
            && now - self.received_at_ms <= 3_000
            && self
                .source_at_ms
                .is_some_and(|t| now >= t && now - t <= 3_000)
            && matches!((positive(&self.bid),positive(&self.ask)),(Some(b),Some(a)) if b<=a)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerComparison {
    pub selection: StockPeerSelection,
    pub instrument: Option<VenueInstrument>,
    pub identity: StockPeerIdentity,
    /// Server-verified share basis for price/BBO/lot comparisons, not transfer
    /// token units or proof of the corporate-action-aware execution compiler.
    #[serde(default)]
    pub share_unit_verified: bool,
    pub quote: Option<StockPeerQuote>,
    /// Actual USDC/native-quote BBO on the selected venue; never a fixed peg.
    pub quote_conversion: Option<StockPeerQuote>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockPeerEstimate {
    pub chain_buy: bool,
    pub shares: Option<String>,
    pub gross_usdc: Option<String>,
    pub cex_notional_usdc: Option<String>,
    pub remainder_shares: Option<String>,
    pub blockers: Vec<String>,
}

pub fn evaluate_peer(snapshot: &StockMarketSnapshot, now: i64) -> Vec<StockPeerEstimate> {
    let Some(peer) = snapshot.peer.as_ref() else {
        return vec![];
    };
    [true, false]
        .into_iter()
        .map(|chain_buy| {
            let mut row = StockPeerEstimate {
                chain_buy,
                shares: None,
                gross_usdc: None,
                cex_notional_usdc: None,
                remainder_shares: None,
                blockers: vec!["尚未核齐账户费率、Gas、双边库存与执行通道；差额不是净利润".into()],
            };
            if let Err(reason) = estimate(snapshot, peer, chain_buy, now, &mut row) {
                row.blockers.push(reason.into());
            }
            row
        })
        .collect()
}

fn estimate(
    snapshot: &StockMarketSnapshot,
    peer: &StockPeerComparison,
    chain_buy: bool,
    now: i64,
    row: &mut StockPeerEstimate,
) -> Result<(), &'static str> {
    if !peer.identity.underlying_verified {
        return Err("对应证券尚未核实，保留原币报价，不计算收益");
    }
    if peer.selection.product != StockPeerProduct::Spot {
        return Err("股票永续还需合约乘数、Funding 与持仓成本核验");
    }
    if !peer.share_unit_verified {
        return Err("报价数量、下单步长与实际股数的换算尚未核实，暂不计算差额");
    }
    let instrument = peer.instrument.as_ref().ok_or("官方市场规格尚未读取")?;
    if peer.problem.is_some()
        || instrument.venue != peer.selection.venue
        || instrument.native_symbol != peer.selection.native_symbol
        || !peer
            .selection
            .product
            .matches(instrument.product_type.as_deref())
        || !instrument.has_official_provenance()
        || !instrument.is_fresh_at(
            now,
            crate::instrument_registry::INSTRUMENT_SPEC_FRESHNESS_MS,
        )
        || instrument.listing_status != InstrumentListingStatus::Trading
    {
        return Err("官方市场规格已陈旧或市场未开放");
    }
    let c = snapshot
        .comparison
        .as_ref()
        .filter(|c| {
            snapshot
                .security
                .as_ref()
                .is_some_and(|s| s.asset == c.asset)
        })
        .ok_or("先读取当前股票的链上双向询价")?;
    if now < c.mint.checked_at_ms
        || now - c.mint.checked_at_ms > 60_000
        || c.mint.next_change_at_ms.is_some_and(|t| now >= t)
    {
        return Err("链上股数倍率已陈旧");
    }
    let q = if chain_buy {
        Some(&c.buy)
    } else {
        c.sell.as_ref()
    }
    .filter(|q| quote_current(q, now))
    .ok_or("链上对应方向报价缺失或已陈旧")?;
    if (chain_buy && (q.input_mint != SOLANA_USDC || q.output_mint != c.mint.address))
        || (!chain_buy && (q.input_mint != c.mint.address || q.output_mint != SOLANA_USDC))
    {
        return Err("链上报价合约与当前证券不一致");
    }
    let b = peer
        .quote
        .as_ref()
        .filter(|b| b.fresh_ws(now) && b.symbol.eq_ignore_ascii_case(&peer.selection.native_symbol))
        .ok_or("交易所 WS 买卖价缺失或已陈旧")?;
    let amount = shares(
        if chain_buy {
            &q.minimum_output_raw
        } else {
            &q.input_raw
        },
        &c.mint,
    )
    .filter(|q| *q > Decimal::ZERO)
    .ok_or("无法核算链上股数")?;
    let step = instrument
        .qty_step
        .and_then(|v| positive(&v.to_string()))
        .ok_or("缺少交易所股数步长")?;
    let lots = amount.checked_div(step).ok_or("股数计算超出范围")?;
    let qty = (if chain_buy { lots.floor() } else { lots.ceil() })
        .checked_mul(step)
        .ok_or("股数计算超出范围")?;
    row.shares = Some(qty.normalize().to_string());
    row.remainder_shares = Some((qty - amount).abs().normalize().to_string());
    let min = instrument
        .min_qty
        .and_then(|v| positive(&v.to_string()))
        .ok_or("缺少交易所最小股数")?;
    if qty < min {
        return Err("本次股数低于交易所最小数量");
    }
    let price = positive(if chain_buy { &b.bid } else { &b.ask }).ok_or("缺少对应方向价格")?;
    let available = if chain_buy {
        b.bid_quantity.as_deref()
    } else {
        b.ask_quantity.as_deref()
    }
    .and_then(positive)
    .ok_or("一档数量未知，构建前需读取深度")?;
    if available < qty {
        return Err("一档数量不足，构建前需读取深度");
    }
    let notional = qty.checked_mul(price).ok_or("金额计算超出范围")?;
    let min_notional = instrument
        .min_notional
        .filter(|v| v.is_finite() && *v >= 0.0)
        .and_then(|v| v.to_string().parse::<Decimal>().ok())
        .ok_or("缺少交易所最小金额")?;
    if notional < min_notional {
        return Err("本次金额低于交易所最小金额");
    }
    let cex_usdc = match instrument.quote_asset.as_deref() {
        Some("USDC") => notional,
        Some("USD" | "USDT") => {
            let fx = peer
                .quote_conversion
                .as_ref()
                .filter(|f| {
                    f.fresh_ws(now)
                        && f.symbol
                            == format!(
                                "USDC/{}",
                                instrument.quote_asset.as_deref().unwrap_or_default()
                            )
                })
                .ok_or("缺少新鲜的 USDC 兑换盘口，不默认一比一")?;
            // Sell stock -> buy USDC at the ask. Buy stock -> sell USDC at the bid.
            let rate = positive(if chain_buy { &fx.ask } else { &fx.bid })
                .ok_or("稳定币兑换方向价格缺失")?;
            let converted = notional.checked_div(rate).ok_or("换汇金额超出范围")?;
            let depth = if chain_buy {
                fx.ask_quantity.as_deref()
            } else {
                fx.bid_quantity.as_deref()
            }
            .and_then(positive)
            .ok_or("稳定币兑换一档数量未知")?;
            if depth < converted {
                return Err("稳定币兑换一档数量不足");
            }
            row.blockers
                .push("包含额外换汇腿，换汇手续费及非原子风险尚未核齐".into());
            converted
        }
        _ => return Err("所选计价币尚未接入显式换汇"),
    };
    let raw = if chain_buy {
        &q.input_raw
    } else {
        &q.minimum_output_raw
    };
    let chain_usdc = Decimal::from(raw.parse::<u64>().map_err(|_| "链上 USDC 金额无效")?)
        / Decimal::from(1_000_000);
    let gross = if chain_buy {
        cex_usdc.checked_sub(chain_usdc)
    } else {
        chain_usdc.checked_sub(cex_usdc)
    }
    .ok_or("差额超出范围")?;
    row.gross_usdc = Some(gross.normalize().to_string());
    row.cex_notional_usdc = Some(cex_usdc.normalize().to_string());
    row.blockers
        .push("不同发行方的产品不能直接互转；需预置库存，价差可能扩大".into());
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
