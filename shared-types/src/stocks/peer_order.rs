use super::{comparison::positive, *};
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerOrderCheckRequest {
    pub asset: String,
    pub selection: StockPeerSelection,
    pub direction: StockChainDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerOrderDraft {
    #[serde(default, skip_serializing_if = "StockPeerOrderPurpose::is_equity")]
    pub purpose: StockPeerOrderPurpose,
    pub request: StockPeerOrderCheckRequest,
    pub quantity: String,
    pub limit_price: String,
    pub quote_asset: String,
    pub prepared_at_ms: i64,
    pub source_at_ms: i64,
    pub metadata_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPeerOrderPurpose {
    #[default]
    Equity,
    CashConversion,
}
impl StockPeerOrderPurpose {
    pub fn is_equity(&self) -> bool {
        *self == Self::Equity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPeerOrderCheckStatus {
    Passed,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerOrderCheck {
    pub draft: StockPeerOrderDraft,
    pub completed_at_ms: Option<i64>,
    pub status: StockPeerOrderCheckStatus,
    pub message: String,
}

pub fn prepare_peer_order_check(
    s: &StockMarketSnapshot,
    request: StockPeerOrderCheckRequest,
    now: i64,
) -> Result<StockPeerOrderDraft, String> {
    let issuer = identity::backpack_token_identity(s)?;
    if s.comparison.as_ref().is_none_or(|c| {
        c.asset != issuer.asset
            || c.mint.address != issuer.solana_mint
            || c.mint.decimals != issuer.decimals
    }) {
        return Err("链上报价合约或精度与官方股票映射不一致".into());
    }
    let peer = s
        .peer
        .as_ref()
        .filter(|p| p.selection == request.selection && p.share_unit_verified)
        .ok_or("所选股票行情或份额口径尚未核实")?;
    let xstock = issuer
        .kraken
        .as_ref()
        .ok_or("该证券没有核实的 Kraken 股票对应关系")?;
    let spec = peer.instrument.as_ref().ok_or("缺少官方股票规格")?;
    let quote = spec
        .quote_asset
        .as_deref()
        .filter(|q| matches!(*q, "USDC" | "USDT" | "USD"))
        .ok_or("股票计价币未核实")?;
    if request.asset != issuer.asset
        || request.selection.venue != "kraken"
        || request.selection.product != StockPeerProduct::Spot
        || request.selection.native_symbol != format!("{}/{quote}", xstock.base)
        || spec.asset_class != crate::InstrumentAssetClass::Equity
        || spec.canonical_symbol != xstock.base.to_ascii_uppercase()
        || spec.source_url.as_deref()
            != Some("https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument")
        || spec.schema_version.as_deref() != Some("kraken-spot-ws-v2-instrument-2026-08-06")
    {
        return Err("股票原生市场与已核实的 WS 规格不一致".into());
    }
    let row = evaluate_peer(s, now)
        .into_iter()
        .find(|r| r.chain_buy == (request.direction == StockChainDirection::Buy))
        .ok_or("缺少对应方向报价")?;
    if row.gross_usdc.is_none() {
        return Err(row
            .blockers
            .last()
            .cloned()
            .unwrap_or_else(|| "报价条件未满足".into()));
    }
    let b = peer.quote.as_ref().ok_or("缺少股票 WS 盘口")?;
    let price = positive(if row.chain_buy { &b.bid } else { &b.ask }).ok_or("股票限价无效")?;
    let tick = spec
        .price_tick
        .and_then(|n| positive(&n.to_string()))
        .ok_or("缺少官方价格步长")?;
    if price.checked_rem(tick).is_none_or(|n| !n.is_zero()) {
        return Err("股票限价不符合官方步长".into());
    }
    Ok(StockPeerOrderDraft {
        purpose: StockPeerOrderPurpose::Equity,
        request,
        quantity: row.shares.ok_or("缺少精确股数")?,
        limit_price: price.normalize().to_string(),
        quote_asset: quote.into(),
        prepared_at_ms: now,
        source_at_ms: b.source_at_ms.ok_or("缺少源行情时间")?,
        metadata_at_ms: spec.checked_at_ms,
    })
}

impl StockPeerOrderDraft {
    pub fn metadata_max_age_ms(&self) -> i64 {
        if self.purpose.is_equity() {
            60_000
        } else {
            crate::instrument_registry::INSTRUMENT_SPEC_FRESHNESS_MS
        }
    }
    // Only a validation request exists here. No caller-controlled switch can turn
    // it into an executable order, and no arbitrary JSON is accepted by the API.
    pub fn kraken_validation(
        &self,
        token: &str,
        request_id: u64,
        now: i64,
    ) -> Result<serde_json::Value, &'static str> {
        let (base, quote) = self
            .request
            .selection
            .native_symbol
            .split_once('/')
            .ok_or("股票市场无效")?;
        let profile =
            identity::backpack_issuer_profile(&self.request.asset).and_then(|p| p.kraken.as_ref());
        let valid_identity = match self.purpose {
            StockPeerOrderPurpose::Equity => profile.is_some_and(|p| p.base == base),
            StockPeerOrderPurpose::CashConversion => {
                profile.is_some() && base == "USDC" && matches!(quote, "USD" | "USDT")
            }
        };
        let qty = positive(&self.quantity)
            .filter(|n| *n <= Decimal::from(1_000_000))
            .ok_or("股票数量无效")?;
        let price = positive(&self.limit_price)
            .filter(|n| *n <= Decimal::from(10_000_000))
            .ok_or("股票限价无效")?;
        if !valid_identity
            || self.request.selection.venue != "kraken"
            || self.request.selection.product != StockPeerProduct::Spot
            || quote != self.quote_asset
            || !matches!(quote, "USD" | "USDC" | "USDT")
            || token.is_empty()
            || now < self.prepared_at_ms
            || now - self.prepared_at_ms > 10_000
        {
            return Err("股票验证参数已过期或原生市场不匹配");
        }
        let number = |n: Decimal| {
            n.normalize()
                .to_string()
                .parse::<serde_json::Number>()
                .map(serde_json::Value::Number)
                .map_err(|_| "精确数量编码失败")
        };
        let deadline =
            chrono::DateTime::from_timestamp_millis(now.checked_add(2000).ok_or("验证时间无效")?)
                .ok_or("验证时间无效")?
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        Ok(
            serde_json::json!({"method":"add_order","req_id":request_id,"params":{
                "symbol":self.request.selection.native_symbol,"side":if self.request.direction==StockChainDirection::Buy{"sell"}else{"buy"},
                "order_type":"limit","order_qty":number(qty)?,"limit_price":number(price)?,"time_in_force":"fok",
                "fee_preference":"quote","margin":false,"post_only":false,"stp_type":"cancel_newest","validate":true,
                "cl_ord_id":format!("sv{request_id:016x}"),"deadline":deadline,"token":token
            }}),
        )
    }
}

#[cfg(test)]
mod tests;
