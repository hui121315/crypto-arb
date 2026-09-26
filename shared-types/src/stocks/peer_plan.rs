use super::*;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerPlanRequest {
    pub request_id: String,
    pub asset: String,
    pub selection: StockPeerSelection,
    pub direction: StockChainDirection,
    pub wallet_address: String,
    pub input_raw: String,
    pub keyed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerPlanBuildReceipt {
    pub plan_id: String,
    pub request: StockPeerPlanRequest,
    pub phase: StockPeerPlanPhase,
    pub observed_at_ms: i64,
}

impl StockPeerPlanBuildReceipt {
    pub fn valid_for(&self, request_id: &str) -> bool {
        self.request.request_id == request_id && self.request.validate().is_ok()
            && !self.plan_id.is_empty() && self.observed_at_ms > 0
    }
}

/// Only the reviewed inputs, not an ever-growing recursive market snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerPlanBasis {
    pub security: StockSecurity,
    pub tokens: Vec<StockChainToken>,
    pub token_metadata_at_ms: Option<i64>,
    pub comparison: StockComparison,
    pub peer: StockPeerComparison,
    pub account: StockPeerAccount,
    pub wallet: StockWalletEvidence,
    pub chain_cost: StockChainCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerPlanTerms {
    pub account_fingerprint: String,
    pub basis: StockPeerPlanBasis,
    pub draft: StockPeerOrderDraft,
    pub allocations: Vec<StockPlanAllocation>,
    pub cex_fee_quote: String,
    pub after_known_costs_usdc: String,
    pub remainder_shares: String,
    pub created_at_ms: i64,
    pub market_valid_until_ms: i64,
    pub reserved_until_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPeerPlanPhase {
    Reserved,
    Cancelled,
    SubmissionUnknown,
    Settled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerSettlement {
    pub source_revision: u64,
    pub settled_at_ms: i64,
    pub accounting: StockPeerAccounting,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerHistoryCheck {
    pub attempts: u32,
    pub next_check_at_ms: i64,
    pub checked_at_ms: Option<i64>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerPlan {
    pub plan_id: String,
    pub request: StockPeerPlanRequest,
    pub terms: StockPeerPlanTerms,
    pub phase: StockPeerPlanPhase,
    pub revision: u64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cex_order: Option<StockPeerOrderReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_submission: Option<StockChainSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_problem: Option<String>,
    #[serde(default)]
    pub cex_history: StockPeerHistoryCheck,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recoveries: Vec<StockPeerRecovery>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversions: Vec<StockPeerConversion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_topups: Vec<StockPeerNativeTopup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inventory_orders: Vec<StockPeerInventory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<StockPeerSettlement>,
}

impl StockPeerPlan {
    pub fn holds_funds(&self, now: i64) -> bool {
        self.phase == StockPeerPlanPhase::SubmissionUnknown
            || self.phase == StockPeerPlanPhase::Reserved && now < self.terms.reserved_until_ms
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerExecutionRequest {
    pub plan_id: String,
    pub revision: u64,
    pub confirm_live: bool,
}

impl StockPeerPlanRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !(16..=128).contains(&self.request_id.len())
            || !self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || self
                .input_raw
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .is_none_or(|n| n.to_string() != self.input_raw)
            || !(32..=44).contains(&self.wallet_address.len())
            || !self
                .wallet_address
                .bytes()
                .all(|b| b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(&b))
            || self.selection.venue != "kraken"
            || self.selection.product != StockPeerProduct::Spot
            || self.selection.native_symbol.len() > 100
            || identity::backpack_issuer_profile(&self.asset).is_none()
        {
            return Err("股票双边计划的请求编号、钱包、原投入或市场无效".into());
        }
        Ok(())
    }
}

fn decimal(s: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(s).map_err(|_| "股票计划金额精度无效".into())
}
fn nonnegative(s: &str) -> Result<Decimal, String> {
    decimal(s).and_then(|n| {
        if n >= Decimal::ZERO {
            Ok(n)
        } else {
            Err("股票计划金额不能为负".into())
        }
    })
}
fn amount(n: Decimal) -> String {
    n.normalize().to_string()
}
fn raw(s: Option<&str>, decimals: u8) -> Result<Decimal, String> {
    let n = s
        .ok_or("钱包余额尚未核实")?
        .parse::<u64>()
        .map_err(|_| "钱包原始数量无效")?;
    let divisor = 10_u64.checked_pow(decimals.into()).ok_or("钱包精度无效")?;
    Decimal::from(n)
        .checked_div(Decimal::from(divisor))
        .ok_or("钱包精度溢出".into())
}
fn allocation(
    location: &str,
    asset: &str,
    quantity: Decimal,
    available: Decimal,
) -> Result<StockPlanAllocation, String> {
    if quantity < Decimal::ZERO || available < quantity {
        return Err(format!(
            "{location} 缺少 {asset}：需要 {}，可用 {}",
            amount(quantity),
            amount(available)
        ));
    }
    Ok(StockPlanAllocation {
        location: location.into(),
        asset: asset.into(),
        quantity: amount(quantity),
        available_at_reservation: amount(available),
    })
}

pub fn prepare_peer_plan_terms(
    request: &StockPeerPlanRequest,
    basis: StockPeerPlanBasis,
    account_fingerprint: String,
    now: i64,
) -> Result<StockPeerPlanTerms, String> {
    request.validate()?;
    let b = &basis;
    let c = &b.chain_cost;
    let fresh = |t: i64, age: i64| t > 0 && now >= t && now - t <= age;
    if account_fingerprint.len() != 24
        || !account_fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
        || b.security.asset != request.asset
        || b.comparison.asset != request.asset
        || b.peer.selection != request.selection
        || b.comparison.keyed != request.keyed
        || c.direction != request.direction
        || c.asset != request.asset
        || c.quote.input_raw != request.input_raw
        || c.wallet_address != request.wallet_address
        || b.wallet.owner != request.wallet_address
        || b.wallet.mint != b.comparison.mint.address
        || b.account.venue != request.selection.venue
        || b.account.native_symbol != request.selection.native_symbol
        || request.selection.native_symbol
            != format!("{}/{}", b.account.stock_asset, b.account.quote_asset)
        || !fresh(b.account.observed_at_ms, 15_000)
        || !fresh(b.wallet.checked_at_ms, 15_000)
        || !b.account.problems.is_empty()
        || !b.wallet.problems.is_empty()
        || !c.simulation_passed
        || !c.problems.is_empty()
    {
        return Err("股票计划的账户、钱包、份额、原报价或新鲜度未核齐".into());
    }
    let s = StockMarketSnapshot {
        security: Some(b.security.clone()),
        tokens: b.tokens.clone(),
        token_metadata_at_ms: b.token_metadata_at_ms,
        comparison: Some(b.comparison.clone()),
        peer: Some(b.peer.clone()),
        chain_costs: vec![c.clone()],
        peer_preflight: Some(StockPeerPreflight {
            asset: request.asset.clone(),
            selection: request.selection.clone(),
            checked_at_ms: now,
            account: Some(b.account.clone()),
            wallet: Some(b.wallet.clone()),
            problems: vec![],
        }),
        ..Default::default()
    };
    if !c.current(&s, &request.wallet_address, now) {
        return Err("链上原交易试算已失效，请重新构建".into());
    }
    let draft = prepare_peer_order_check(
        &s,
        StockPeerOrderCheckRequest {
            asset: request.asset.clone(),
            selection: request.selection.clone(),
            direction: request.direction,
        },
        now,
    )?;
    let estimate = evaluate_peer(&s, now)
        .into_iter()
        .find(|r| r.chain_buy == (request.direction == StockChainDirection::Buy))
        .ok_or("缺少原方向报价")?;
    let budget = evaluate_peer_preflight(&s, now)
        .into_iter()
        .find(|r| r.chain_buy == estimate.chain_buy)
        .ok_or("缺少费用预算")?;
    let after = budget
        .after_known_costs_usdc
        .as_deref()
        .ok_or("股票、换汇或 SOL 补回费用尚未核齐")?;
    if decimal(after)? <= Decimal::ZERO {
        return Err("扣除已知费用后没有正差额".into());
    }
    let qty = nonnegative(&draft.quantity)?;
    let notional = qty
        .checked_mul(nonnegative(&draft.limit_price)?)
        .ok_or("股票金额溢出")?;
    let rate = nonnegative(
        b.account
            .stock_taker_pct
            .as_deref()
            .ok_or("缺少账户股票费率")?,
    )?;
    if rate >= Decimal::from(100) {
        return Err("股票费率无效".into());
    }
    let fee = notional
        .checked_mul(rate)
        .and_then(|n| n.checked_div(Decimal::from(100)))
        .ok_or("费用溢出")?;
    let native = nonnegative(
        &c.complete_native_usdc_budget(now)
            .ok_or("SOL 补回费用未知")?,
    )?;
    let mut allocations = vec![];
    // A USD order reserves USD, not an assumed USDC equivalent. The latter is
    // valuation only; no FX order or stock-token transfer is silently added.
    allocations.push(if estimate.chain_buy {
        allocation(
            "kraken",
            &b.account.stock_asset,
            qty,
            nonnegative(
                b.account
                    .stock_available
                    .as_deref()
                    .ok_or("Kraken 股票库存未知")?,
            )?,
        )?
    } else {
        allocation(
            "kraken",
            &b.account.quote_asset,
            notional.checked_add(fee).ok_or("费用溢出")?,
            nonnegative(
                b.account
                    .quote_available
                    .as_deref()
                    .ok_or("Kraken 原生计价币余额未知")?,
            )?,
        )?
    });
    let usdc = raw(b.wallet.usdc_raw.as_deref(), 6)?;
    if estimate.chain_buy {
        allocations.push(allocation(
            "Solana",
            "USDC",
            raw(Some(&request.input_raw), 6)?
                .checked_add(native)
                .ok_or("USDC 预算溢出")?,
            usdc,
        )?);
    } else {
        allocations.push(allocation(
            "Solana",
            &b.wallet.mint,
            raw(Some(&request.input_raw), c.mint.decimals)?,
            raw(b.wallet.stock_raw.as_deref(), c.mint.decimals)?,
        )?);
        allocations.push(allocation("Solana", "USDC", native, usdc)?);
    }
    allocations.push(allocation(
        "Solana",
        "SOL",
        Decimal::from(
            c.total_native_required_lamports(now)
                .ok_or("SOL 临时资金需求未知")?,
        ) / Decimal::from(1_000_000_000),
        raw(b.wallet.sol_lamports.as_deref(), 9)?,
    )?);
    let market_valid_until_ms = [
        draft.source_at_ms.saturating_add(3000),
        b.peer
            .quote
            .as_ref()
            .ok_or("股票盘口丢失")?
            .received_at_ms
            .saturating_add(3000),
        draft.metadata_at_ms.saturating_add(60_000),
        c.valid_until_ms,
        c.quote.expires_at_ms.unwrap_or(i64::MAX),
        c.quote
            .requested_at_ms
            .saturating_add(comparison::STOCK_QUOTE_MAX_AGE_MS),
        b.account.observed_at_ms.saturating_add(15_000),
        b.wallet.checked_at_ms.saturating_add(15_000),
        b.comparison.mint.checked_at_ms.saturating_add(60_000),
        b.comparison.mint.next_change_at_ms.unwrap_or(i64::MAX),
        b.peer
            .quote_conversion
            .as_ref()
            .and_then(|q| q.source_at_ms)
            .map(|t| t.saturating_add(3000))
            .unwrap_or(i64::MAX),
        b.peer
            .quote_conversion
            .as_ref()
            .map(|q| q.received_at_ms.saturating_add(3000))
            .unwrap_or(i64::MAX),
        c.native_valuation
            .as_ref()
            .and_then(|v| v.replenishment.as_ref())
            .map(|r| r.valid_until_ms)
            .unwrap_or(i64::MAX),
    ]
    .into_iter()
    .min()
    .unwrap();
    if market_valid_until_ms <= now {
        return Err("报价已过期，未预留".into());
    }
    Ok(StockPeerPlanTerms {
        account_fingerprint,
        basis,
        draft,
        allocations,
        cex_fee_quote: amount(fee),
        after_known_costs_usdc: after.into(),
        remainder_shares: estimate.remainder_shares.unwrap_or_else(|| "0".into()),
        created_at_ms: now,
        market_valid_until_ms,
        reserved_until_ms: now.checked_add(30_000).ok_or("计划时间溢出")?,
    })
}
