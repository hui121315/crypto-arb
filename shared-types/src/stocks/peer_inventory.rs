use super::*;
use crate::VenueInstrument;
use rust_decimal::Decimal;

pub const MAX_STOCK_PEER_INVENTORY_ORDERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerInventoryRequest {
    pub plan_id: String,
    pub revision: u64,
    // Native quote, including fees: maximum debit for a buy, minimum credit for a sell.
    pub quote_limit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerInventory {
    pub request: StockPeerInventoryRequest,
    pub stock_gap: String,
    pub market: VenueInstrument,
    pub quote: StockPeerQuote,
    pub account: StockPeerAccount,
    pub mint: StockMintEvidence,
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
    pub fn peer_inventory_gap(&self) -> Result<Decimal, String> {
        let a = self.accounting();
        if self.phase != StockPeerPlanPhase::SubmissionUnknown
            || a.status != StockAccountingStatus::LegsReconciled
            || a.fee_budget_matched != Some(true)
            || a.recovery_target.is_some()
        {
            return Err("先核齐两腿、实际费用和股票差额，再恢复交易所库存".into());
        }
        let gap = d(a
            .cex_stock_shares
            .as_deref()
            .ok_or("Kraken 实际股票变动未知")?)?;
        if gap.is_zero() {
            return Err("Kraken 股票数量已回到交易前水平".into());
        }
        Ok(gap)
    }

    pub fn peer_inventory_idle(&self, now: i64) -> bool {
        !self.inventory_orders.iter().any(|r| {
            r.order
                .as_ref()
                .is_some_and(|_| r.actual_changes().is_err())
                || (r.order.is_none() && r.cancelled_at_ms.is_none() && now < r.valid_until_ms)
        })
    }

    pub fn peer_dispositions_idle(&self, now: i64) -> bool {
        !self.recoveries.iter().any(|r| {
            r.submission.as_ref().is_some_and(|s| s.receipt.is_none())
                || (r.submission.is_none()
                    && r.cancelled_at_ms.is_none()
                    && now < r.cost.valid_until_ms)
        }) && !self.conversions.iter().any(|r| {
            r.order.as_ref().is_some_and(|_| r.cash_changes().is_err())
                || (r.order.is_none() && r.cancelled_at_ms.is_none() && now < r.valid_until_ms)
        }) && !self.native_topups.iter().any(|r| {
            r.current(now)
                || r.terms
                    .submission
                    .as_ref()
                    .is_some_and(|s| s.receipt.is_none())
        })
    }

    pub fn peer_inventory_available(&self, now: i64) -> bool {
        self.inventory_orders.len() < MAX_STOCK_PEER_INVENTORY_ORDERS
            && self.peer_inventory_idle(now)
            && self.peer_dispositions_idle(now)
    }
}

impl StockPeerInventory {
    pub fn compile(
        p: &StockPeerPlan,
        r: StockPeerInventoryRequest,
        m: VenueInstrument,
        q: StockPeerQuote,
        a: StockPeerAccount,
        mint: StockMintEvidence,
        now: i64,
    ) -> Result<Self, String> {
        let gap = p.peer_inventory_gap()?;
        let buying = gap < Decimal::ZERO;
        let qty = gap.abs();
        let profile = identity::backpack_issuer(&p.terms.basis.security)?;
        let base = &profile
            .kraken
            .as_ref()
            .ok_or("未核实 Kraken 股票身份")?
            .base;
        let quote = &p.terms.draft.quote_asset;
        let native = format!("{base}/{quote}");
        let limit = stock_peer_recovery_limit(&r.quote_limit)
            .ok_or("原币限额必须大于零，不超过 100000，最多六位小数")?;
        if r.plan_id != p.plan_id
            || r.revision != p.revision
            || !p.peer_inventory_available(now)
            || !p.terms.basis.peer.share_unit_verified
            || m.venue != "kraken"
            || m.native_symbol != native
            || p.request.selection.native_symbol != native
            || m.canonical_symbol != base.to_ascii_uppercase()
            || m.quote_asset.as_deref() != Some(quote)
            || m.product_type.as_deref() != Some("spot")
            || m.asset_class != crate::InstrumentAssetClass::Equity
            // Stock orders use the dedicated compiler; the registry's generic
            // hedge executor remains disabled for tokenized equities.
            || !m.is_structurally_valid()
            || !m.has_official_provenance()
            || m.listing_status != crate::InstrumentListingStatus::Trading
            || (m.min_qty.is_none() && m.min_notional.is_none())
            || !m.is_fresh_at(now, 60_000)
            || m.source_url.as_deref()
                != Some(
                    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument",
                )
            || m.schema_version.as_deref() != Some("kraken-spot-ws-v2-instrument-2026-08-06")
            || q.symbol != native
            || !q.fresh_ws(now)
            || a.venue != "kraken"
            || a.native_symbol != native
            || a.quote_asset != *quote
            || !a.stock_asset.eq_ignore_ascii_case(base)
            || !a.problems.is_empty()
            || now < a.observed_at_ms
            || now - a.observed_at_ms > 30_000
            || mint.address != p.terms.basis.chain_cost.mint.address
            || mint.decimals != p.terms.basis.chain_cost.mint.decimals
            || d(&mint.ui_multiplier)? != d(&p.terms.basis.chain_cost.mint.ui_multiplier)?
            || mint.slot < p.peer_minimum_slot()?
            || now < mint.checked_at_ms
            || now - mint.checked_at_ms > 30_000
            || mint.next_change_at_ms.is_some_and(|t| now >= t)
        {
            return Err("恢复库存的原计划、股票身份、官方规格、WS 盘口或账户未就绪".into());
        }
        let fee =
            d(a.stock_taker_pct.as_deref().ok_or("本账户股票费率未知")?)? / Decimal::from(100);
        let step = d(&m.qty_step.ok_or("股票数量步长未知")?.to_string())?;
        let tick = d(&m.price_tick.ok_or("股票价格步长未知")?.to_string())?;
        let price = d(if buying { &q.ask } else { &q.bid })?;
        let depth = d(if buying {
            q.ask_quantity.as_deref()
        } else {
            q.bid_quantity.as_deref()
        }
        .ok_or("股票 WS 盘口量未知")?)?;
        if fee < Decimal::ZERO
            || fee >= Decimal::ONE
            || step <= Decimal::ZERO
            || tick <= Decimal::ZERO
            || price <= Decimal::ZERO
            || qty.checked_rem(step) != Some(Decimal::ZERO)
            || price.checked_rem(tick) != Some(Decimal::ZERO)
        {
            return Err("实际库存差额或价格不符合官方步长，不会扩大股票数量凑单".into());
        }
        let gross = qty.checked_mul(price).ok_or("股票恢复金额溢出")?;
        let fees = gross.checked_mul(fee).ok_or("股票恢复费用溢出")?;
        let cash = (if buying { -gross } else { gross })
            .checked_sub(fees)
            .ok_or("净收支溢出")?;
        if qty > Decimal::from(1_000_000)
            || qty > depth
            || m.min_qty
                .is_some_and(|v| d(&v.to_string()).map_or(true, |v| qty < v))
            || m.min_notional
                .is_some_and(|v| d(&v.to_string()).map_or(true, |v| gross < v))
            || (buying && -cash > limit)
            || (!buying && cash < limit)
        {
            return Err("恢复库存的净收支、盘口量或最小交易额不满足本次限额".into());
        }
        if buying && d(a.quote_available.as_deref().ok_or("原币可用余额未知")?)? < -cash
            || !buying && d(a.stock_available.as_deref().ok_or("股票可用余额未知")?)? < qty
        {
            return Err("Kraken 库存恢复余额不足，不借款或动用其他股票".into());
        }
        let draft = StockPeerOrderDraft {
            purpose: StockPeerOrderPurpose::Equity,
            request: StockPeerOrderCheckRequest {
                asset: p.request.asset.clone(),
                selection: p.request.selection.clone(),
                direction: if buying {
                    StockChainDirection::Sell
                } else {
                    StockChainDirection::Buy
                },
            },
            quantity: text(qty),
            limit_price: text(price),
            quote_asset: quote.clone(),
            prepared_at_ms: now,
            source_at_ms: q.source_at_ms.ok_or("源行情时间未知")?,
            metadata_at_ms: m.checked_at_ms,
        };
        draft.kraken_validation("local-inventory-compile", 1, now)?;
        let end = now
            .saturating_add(3000)
            .min(draft.source_at_ms.saturating_add(3000))
            .min(q.received_at_ms.saturating_add(3000))
            .min(m.checked_at_ms.saturating_add(60_000))
            .min(a.observed_at_ms.saturating_add(30_000))
            .min(mint.checked_at_ms.saturating_add(30_000))
            .min(mint.next_change_at_ms.unwrap_or(i64::MAX));
        Ok(Self {
            request: r,
            stock_gap: text(gap),
            market: m,
            quote: q,
            account: a,
            mint,
            draft,
            fee_quote: text(fees),
            quote_change: text(cash),
            valid_until_ms: end,
            cancelled_at_ms: None,
            order: None,
            history: Default::default(),
        })
    }

    pub fn buying(&self) -> bool {
        self.draft.request.direction == StockChainDirection::Sell
    }

    pub fn check_current(
        &self,
        p: &StockPeerPlan,
        m: &VenueInstrument,
        q: &StockPeerQuote,
        a: &StockPeerAccount,
        mint: &StockMintEvidence,
        now: i64,
    ) -> Result<(), String> {
        let mut metadata = m.clone();
        metadata.checked_at_ms = self.market.checked_at_ms;
        if now < self.draft.prepared_at_ms
            || now >= self.valid_until_ms
            || metadata != self.market
            || self.account.stock_taker_pct != a.stock_taker_pct
            || p.peer_inventory_gap()? != d(&self.stock_gap)?
        {
            return Err("恢复库存的版本、费用、份额或原规格已变化".into());
        }
        let mut request = self.request.clone();
        request.revision = p.revision;
        let current = Self::compile(
            p,
            request,
            m.clone(),
            q.clone(),
            a.clone(),
            mint.clone(),
            now,
        )?;
        if current.draft.quantity != self.draft.quantity
            || (self.buying() && d(&q.ask)? > d(&self.draft.limit_price)?)
            || (!self.buying() && d(&q.bid)? < d(&self.draft.limit_price)?)
            || (self.buying()
                && d(a.quote_available.as_deref().ok_or("原币余额未知")?)?
                    < -d(&self.quote_change)?)
        {
            return Err("当前价格或余额不满足原库存恢复订单，不更改原限价和数量".into());
        }
        Ok(())
    }

    pub fn observed_changes(&self) -> Result<StockPeerCashSettlement, String> {
        let r = self.order.as_ref().ok_or("库存恢复尚未提交")?;
        r.validate_stored()?;
        let c = r
            .cash_settlement()
            .ok_or("库存恢复终态、成交或原币费用未核齐")?;
        if r.draft != self.draft
            || c.quote_asset != self.draft.quote_asset
            || Some(c.base_asset.as_str())
                != self
                    .draft
                    .request
                    .selection
                    .native_symbol
                    .split_once('/')
                    .map(|s| s.0)
        {
            return Err("库存恢复回执的原股票或币种不符".into());
        }
        Ok(c)
    }

    pub fn actual_changes(&self) -> Result<StockPeerCashSettlement, String> {
        let c = self.observed_changes()?;
        let r = self.order.as_ref().ok_or("库存恢复尚未提交")?;
        let stock = d(&c.equity_shares_change)?;
        let cash = d(&c.quote_change)?;
        if r.phase == StockCexOrderPhase::Filled {
            let qty = d(&self.draft.quantity)?;
            let cost = d(r.cumulative_cost.as_deref().ok_or("实际股票金额未知")?)?;
            let fee = if self.buying() {
                -cash - cost
            } else {
                cost - cash
            };
            let max_fee = cost
                .checked_mul(d(self
                    .account
                    .stock_taker_pct
                    .as_deref()
                    .ok_or("原股票费率未知")?)?)
                .and_then(|v| v.checked_div(Decimal::from(100)))
                .ok_or("股票实际费用溢出")?;
            if stock != if self.buying() { qty } else { -qty }
                || cash < d(&self.quote_change)?
                || fee > max_fee
            {
                return Err("实际股票数量、原币支出或费用超出库存恢复预算".into());
            }
        } else if !stock.is_zero() || !cash.is_zero() {
            return Err("库存恢复未足额成交却已有收支，需核对".into());
        }
        Ok(c)
    }
}
