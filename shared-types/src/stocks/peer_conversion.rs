use super::*;
use crate::VenueInstrument;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

pub const MAX_STOCK_PEER_CONVERSIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerConversionRequest {
    pub plan_id: String,
    pub revision: u64,
    // Surplus native quote: minimum USDC received. Deficit: maximum USDC sold.
    pub usdc_limit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerConversion {
    pub request: StockPeerConversionRequest,
    pub native_gap: String,
    pub market: VenueInstrument,
    pub quote: StockPeerQuote,
    pub account: StockPeerAccount,
    pub draft: StockPeerOrderDraft,
    pub fee_quote: String,
    pub quote_change: String,
    pub valid_until_ms: i64,
    pub cancelled_at_ms: Option<i64>,
    pub order: Option<StockPeerOrderReceipt>,
    pub history: StockPeerHistoryCheck,
}
fn d(s: &str) -> Result<Decimal, String> {
    stock_exact_decimal(s).map_err(str::to_owned)
}
fn text(n: Decimal) -> String {
    n.normalize().to_string()
}

impl StockPeerPlan {
    pub fn peer_conversion_gap(&self) -> Result<Decimal, String> {
        let a = self.accounting();
        if self.phase != StockPeerPlanPhase::SubmissionUnknown
            || a.status != StockAccountingStatus::LegsReconciled
            || a.recovery_target.is_some()
            || !matches!(a.quote_asset.as_str(), "USD" | "USDT")
        {
            return Err("先核齐原交易、费用和股票差额，再处理原币换汇".into());
        }
        let n = d(a
            .cash_totals
            .get(&a.quote_asset)
            .ok_or("原币现金收支未知")?)?;
        if n.is_zero() {
            return Err("原币现金差额已经为零".into());
        }
        Ok(n)
    }
    pub fn peer_conversion_available(&self, now: i64) -> bool {
        self.phase == StockPeerPlanPhase::SubmissionUnknown
            && self.conversions.len() < MAX_STOCK_PEER_CONVERSIONS
            && self.peer_inventory_idle(now)
            && !self.recoveries.iter().any(|r| {
                r.submission.as_ref().is_some_and(|s| s.receipt.is_none())
                    || (r.submission.is_none()
                        && r.cancelled_at_ms.is_none()
                        && now < r.cost.valid_until_ms)
            })
            && !self.native_topups.iter().any(|r| {
                r.current(now)
                    || r.terms
                        .submission
                        .as_ref()
                        .is_some_and(|s| s.receipt.is_none())
            })
            && !self.conversions.last().is_some_and(|r| {
                r.order.as_ref().is_some_and(|_| r.cash_changes().is_err())
                    || (r.order.is_none() && r.cancelled_at_ms.is_none() && now < r.valid_until_ms)
            })
    }
}

impl StockPeerConversion {
    pub fn compile(
        p: &StockPeerPlan,
        r: StockPeerConversionRequest,
        m: VenueInstrument,
        q: StockPeerQuote,
        a: StockPeerAccount,
        now: i64,
    ) -> Result<Self, String> {
        let gap = p.peer_conversion_gap()?;
        let buying = gap > Decimal::ZERO;
        let quote = p.terms.draft.quote_asset.clone();
        let native = format!("USDC/{quote}");
        let limit =
            stock_peer_recovery_limit(&r.usdc_limit).ok_or("USDC 限额必须大于零，最多六位小数")?;
        if r.plan_id != p.plan_id
            || r.revision != p.revision
            || !p.peer_conversion_available(now)
            || m.venue != "kraken"
            || m.native_symbol != native
            || m.canonical_symbol != "USDC"
            || m.quote_asset.as_deref() != Some(&quote)
            || m.product_type.as_deref() != Some("spot")
            || !matches!(
                m.asset_class,
                crate::InstrumentAssetClass::Crypto | crate::InstrumentAssetClass::Forex
            )
            || !m.is_hedge_constructible()
            || !m.is_fresh_at(
                now,
                crate::instrument_registry::INSTRUMENT_SPEC_FRESHNESS_MS,
            )
            || m.source_url.as_deref()
                != Some(
                    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument",
                )
            || m.schema_version.as_deref() != Some("kraken-spot-ws-v2-instrument-2026-08-06")
            || q.symbol != native
            || !q.fresh_ws(now)
            || a.venue != "kraken"
            || a.native_symbol != p.request.selection.native_symbol
            || a.quote_asset != quote
            || !a.problems.is_empty()
            || now < a.observed_at_ms
            || now - a.observed_at_ms > 30_000
        {
            return Err("换汇原计划、官方市场、WS 盘口或原账户未就绪".into());
        }
        let fee = d(a.fx_taker_pct.as_deref().ok_or("缺少本账户换汇费率")?)? / Decimal::from(100);
        if fee < Decimal::ZERO || fee >= Decimal::ONE {
            return Err("换汇费率无效".into());
        }
        let step = d(&m.qty_step.ok_or("换汇数量步长未知")?.to_string())?;
        let tick = d(&m.price_tick.ok_or("换汇价格步长未知")?.to_string())?;
        let price = d(if buying { &q.ask } else { &q.bid })?;
        let depth = d(if buying {
            q.ask_quantity.as_deref()
        } else {
            q.bid_quantity.as_deref()
        }
        .ok_or("换汇 WS 买卖量未知")?)?;
        if step <= Decimal::ZERO
            || tick <= Decimal::ZERO
            || price <= Decimal::ZERO
            || price.checked_rem(tick) != Some(Decimal::ZERO)
        {
            return Err("换汇官方规格无效".into());
        }
        let unit = price
            .checked_mul(if buying {
                Decimal::ONE + fee
            } else {
                Decimal::ONE - fee
            })
            .ok_or("换汇成本溢出")?;
        let lots = gap
            .abs()
            .checked_div(unit)
            .and_then(|n| n.checked_div(step))
            .ok_or("换汇数量溢出")?;
        let qty = (if buying { lots.floor() } else { lots.ceil() })
            .checked_mul(step)
            .ok_or("换汇数量溢出")?;
        let gross = qty.checked_mul(price).ok_or("换汇金额溢出")?;
        let fees = gross.checked_mul(fee).ok_or("换汇费用溢出")?;
        let change = (if buying { -gross } else { gross })
            .checked_sub(fees)
            .ok_or("换汇净额溢出")?;
        if qty <= Decimal::ZERO
            || qty > Decimal::from(100_000)
            || qty > depth
            || m.min_qty
                .is_some_and(|n| d(&n.to_string()).map_or(true, |n| qty < n))
            || m.min_notional
                .is_some_and(|n| d(&n.to_string()).map_or(true, |n| gross < n))
            || (buying && (qty < limit || -change > gap))
            || (!buying && (qty > limit || change < -gap))
        {
            return Err("按官方步长和费用计算的换汇数量不满足限额、最小额或盘口量".into());
        }
        if buying && d(a.quote_available.as_deref().ok_or("原币可用余额未知")?)? < -change
            || !buying && d(a.usdc_available.as_deref().ok_or("USDC 可用余额未知")?)? < qty
        {
            return Err("Kraken 换汇可用余额不足；不会自动借款".into());
        }
        let draft = StockPeerOrderDraft {
            purpose: StockPeerOrderPurpose::CashConversion,
            request: StockPeerOrderCheckRequest {
                asset: p.request.asset.clone(),
                selection: StockPeerSelection {
                    venue: "kraken".into(),
                    product: StockPeerProduct::Spot,
                    native_symbol: native,
                },
                direction: if buying {
                    StockChainDirection::Sell
                } else {
                    StockChainDirection::Buy
                },
            },
            quantity: text(qty),
            limit_price: text(price),
            quote_asset: quote,
            prepared_at_ms: now,
            source_at_ms: q.source_at_ms.ok_or("源时间未知")?,
            metadata_at_ms: m.checked_at_ms,
        };
        draft.kraken_validation("local-compile", 1, now)?;
        let end = now
            .saturating_add(3000)
            .min(draft.source_at_ms.saturating_add(3000))
            .min(q.received_at_ms.saturating_add(3000))
            .min(a.observed_at_ms.saturating_add(30_000))
            .min(m.checked_at_ms.saturating_add(draft.metadata_max_age_ms()));
        Ok(Self {
            request: r,
            native_gap: text(gap),
            market: m,
            quote: q,
            account: a,
            draft,
            fee_quote: text(fees),
            quote_change: text(change),
            valid_until_ms: end,
            cancelled_at_ms: None,
            order: None,
            history: Default::default(),
        })
    }
    pub fn buy_usdc(&self) -> bool {
        self.draft.request.direction == StockChainDirection::Sell
    }
    pub fn check_current(
        &self,
        p: &StockPeerPlan,
        m: &VenueInstrument,
        q: &StockPeerQuote,
        a: &StockPeerAccount,
        now: i64,
    ) -> Result<(), String> {
        let mut metadata = m.clone();
        metadata.checked_at_ms = self.market.checked_at_ms;
        let qty = d(&self.draft.quantity)?;
        let price = d(&self.draft.limit_price)?;
        let buying = self.buy_usdc();
        if now < self.draft.prepared_at_ms
            || now >= self.valid_until_ms
            || metadata != self.market
            || !m.is_fresh_at(now, self.draft.metadata_max_age_ms())
            || q.symbol != self.quote.symbol
            || !q.fresh_ws(now)
            || a.venue != self.account.venue
            || a.native_symbol != self.account.native_symbol
            || a.quote_asset != self.account.quote_asset
            || a.fx_taker_pct != self.account.fx_taker_pct
            || !a.problems.is_empty()
            || now < a.observed_at_ms
            || now - a.observed_at_ms > 30_000
            || p.peer_conversion_gap()? != d(&self.native_gap)?
            || (buying && d(&q.ask)? > price)
            || (!buying && d(&q.bid)? < price)
            || d(if buying {
                q.ask_quantity.as_deref()
            } else {
                q.bid_quantity.as_deref()
            }
            .ok_or("当前换汇盘口量未知")?)?
                < qty
            || (buying
                && d(a.quote_available.as_deref().ok_or("当前原币余额未知")?)?
                    < -d(&self.quote_change)?)
            || (!buying && d(a.usdc_available.as_deref().ok_or("当前 USDC 余额未知")?)? < qty)
        {
            return Err("当前价格、规格、余额或费用不再满足原换汇计划".into());
        }
        Ok(())
    }
    /// Proven native movements remain visible even when execution exceeded its budget.
    pub fn native_cash_changes(&self) -> Result<BTreeMap<String, String>, String> {
        let r = self.order.as_ref().ok_or("换汇尚未提交")?;
        r.validate_stored()?;
        if r.draft != self.draft {
            return Err("换汇回执原参数不符".into());
        }
        let c = r
            .cash_settlement()
            .ok_or("换汇终态、逐笔成交或原币费用未核齐")?;
        if c.base_asset != "USDC" || c.quote_asset != self.draft.quote_asset {
            return Err("换汇币种不符".into());
        }
        Ok(BTreeMap::from([
            ("USDC".into(), text(d(&c.equity_shares_change)?)),
            (c.quote_asset, text(d(&c.quote_change)?)),
        ]))
    }
    pub fn cash_changes(&self) -> Result<BTreeMap<String, String>, String> {
        let changes = self.native_cash_changes()?;
        let r = self.order.as_ref().ok_or("换汇尚未提交")?;
        let base = d(&changes["USDC"])?;
        let quote = d(&changes[&self.draft.quote_asset])?;
        if r.phase == StockCexOrderPhase::Filled {
            let qty = d(&self.draft.quantity)?;
            let expected = d(&self.quote_change)?;
            if base != if self.buy_usdc() { qty } else { -qty } || quote < expected {
                return Err("换汇实际投入或净到账超出原计划".into());
            }
            let cost = d(r.cumulative_cost.as_deref().ok_or("换汇实际金额未知")?)?;
            let fee = if self.buy_usdc() {
                -quote - cost
            } else {
                cost - quote
            };
            let max = cost
                .checked_mul(d(self
                    .account
                    .fx_taker_pct
                    .as_deref()
                    .ok_or("原费用未知")?)?)
                .and_then(|n| n.checked_div(Decimal::from(100)))
                .ok_or("费用溢出")?;
            if fee > max {
                return Err("实际换汇费率超出预算".into());
            }
        } else if !base.is_zero() || !quote.is_zero() {
            return Err("换汇未足额成交但已有收支，需核对".into());
        }
        Ok(changes)
    }
}
