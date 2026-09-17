use super::*;
use rust_decimal::{prelude::ToPrimitive, Decimal};
mod native_topup;
pub use native_topup::*;

// Tether's published Solana mint, not an exchange ticker or a bridged lookalike.
pub const STOCK_SOLANA_USDT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockStablecoinRequest {
    pub asset: String,
    pub wallet_address: String,
    pub input_usdt: String,
    pub target_usdc: String,
    pub keyed: bool,
}

impl StockStablecoinRequest {
    pub fn amounts_raw(&self) -> Result<(u64, u64), String> {
        fn raw(s: &str) -> Result<u64, String> {
            if s.is_empty() || s.len() > 24 || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
                return Err("金额须为普通十进制数字".into());
            }
            let n = Decimal::from_str_exact(s).map_err(|_| "金额格式无效")?;
            if n <= Decimal::ZERO || n > Decimal::from(1_000_000) || n.scale() > 6 {
                return Err("金额须大于零、不超过 1000000，最多 6 位小数".into());
            }
            n.checked_mul(Decimal::from(1_000_000))
                .and_then(|n| n.to_u64())
                .ok_or_else(|| "金额超出支持范围".into())
        }
        Ok((raw(&self.input_usdt)?, raw(&self.target_usdc)?))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockStablecoinPreview {
    pub request: StockStablecoinRequest,
    pub quote: StockDexQuote,
    pub wallet: StockWalletEvidence,
    pub cost: Option<StockChainCost>,
    pub checked_at_ms: i64,
    pub valid_until_ms: i64,
    pub minimum_usdc: String,
    pub native_cost_usdc: Option<String>,
    pub after_native_cost_usdc: Option<String>,
    pub shortfall_usdc: String,
    pub input_sufficient: Option<bool>,
    pub blockers: Vec<String>,
}

impl StockStablecoinPreview {
    pub fn current(&self, now: i64) -> bool {
        now >= self.checked_at_ms && now < self.valid_until_ms
    }

    pub fn can_reserve(&self, now: i64) -> bool {
        self.current(now)
            && self.blockers.is_empty()
            && self.input_sufficient == Some(true)
            && self.shortfall_usdc == "0"
            && self.cost.as_ref().is_some_and(|c| {
                c.transaction.is_some() && c.complete_native_usdc_budget(now).is_some()
            })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockStablecoinPlanRequest {
    pub request_id: String,
    pub conversion: StockStablecoinRequest,
    pub preview_at_ms: i64,
    pub transaction_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockStablecoinPlanPhase {
    Reserved,
    SubmissionUnknown,
    Completed,
    Failed,
    NeedsReview,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockStablecoinPlan {
    pub plan_id: String,
    pub request: StockStablecoinPlanRequest,
    pub preview: StockStablecoinPreview,
    pub phase: StockStablecoinPlanPhase,
    pub revision: u64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission: Option<StockChainSubmission>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_topups: Vec<StockStablecoinNativeTopup>,
}

impl StockStablecoinPlan {
    pub fn phase_at(&self, now: i64) -> StockStablecoinPlanPhase {
        if self.phase == StockStablecoinPlanPhase::Reserved && now >= self.preview.valid_until_ms {
            StockStablecoinPlanPhase::Expired
        } else {
            self.phase
        }
    }

    pub fn holds_funds(&self, now: i64) -> bool {
        matches!(
            self.phase_at(now),
            StockStablecoinPlanPhase::Reserved
                | StockStablecoinPlanPhase::SubmissionUnknown
                | StockStablecoinPlanPhase::NeedsReview
        ) || self.native_topups.iter().any(|r| r.holds_funds(now))
            || (!self.native_topups.is_empty() && self.native_accounting().is_err())
    }

    pub fn receipt_phase(&self) -> StockStablecoinPlanPhase {
        use StockStablecoinPlanPhase::*;
        let Some(receipt) = self.submission.as_ref().and_then(|s| s.receipt.as_ref()) else {
            return SubmissionUnknown;
        };
        let Some(input) = stablecoin_change(receipt, STOCK_SOLANA_USDT) else {
            return NeedsReview;
        };
        let Some(output) = stablecoin_change(receipt, comparison::SOLANA_USDC) else {
            return NeedsReview;
        };
        let Some(native) = receipt.wallet_native_change_lamports.parse::<i128>().ok() else {
            return NeedsReview;
        };
        let Some(fee) = receipt.network_fee_lamports.parse::<u64>().ok() else {
            return NeedsReview;
        };
        let unique = receipt
            .asset_changes
            .iter()
            .map(|a| &a.mint)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == receipt.asset_changes.len();
        if !unique {
            return NeedsReview;
        }
        if !receipt.succeeded {
            let wallet_fee = if receipt.fee_payer == self.request.conversion.wallet_address {
                i128::from(fee)
            } else {
                0
            };
            // The receipt parser adds a second problem when rollback cannot be proved.
            return if receipt.problems.len() == 1
                && native == -wallet_fee
                && receipt.asset_changes.iter().all(|a| a.raw_change == "0")
            {
                Failed
            } else {
                NeedsReview
            };
        }
        let exact_input = self
            .request
            .conversion
            .amounts_raw()
            .ok()
            .map(|(n, _)| -i128::from(n));
        let minimum = self.preview.quote.minimum_output_raw.parse::<u64>().ok();
        let native_budget = self
            .preview
            .cost
            .as_ref()
            .and_then(|c| c.wallet_debit_lamports.as_deref())
            .and_then(|s| s.parse::<u64>().ok());
        if receipt.within_plan
            && receipt.problems.is_empty()
            && Some(input) == exact_input
            && minimum.is_some_and(|n| output >= i128::from(n))
            && native_budget.is_some_and(|n| native >= -i128::from(n))
            && receipt.asset_changes.iter().all(|a| {
                a.mint == STOCK_SOLANA_USDT
                    || a.mint == comparison::SOLANA_USDC
                    || a.raw_change.parse::<i128>().is_ok_and(|n| n >= 0)
            })
        {
            Completed
        } else {
            NeedsReview
        }
    }
}

pub fn stablecoin_change(receipt: &StockChainReceipt, mint: &str) -> Option<i128> {
    let mut changes = receipt.asset_changes.iter().filter(|a| a.mint == mint);
    let row = changes.next()?;
    (row.decimals == 6 && changes.next().is_none())
        .then(|| row.raw_change.parse().ok())
        .flatten()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockStablecoinSubmitRequest {
    pub plan_id: String,
    pub revision: u64,
    pub confirm_live: bool,
}

pub fn stablecoin_preview(
    request: StockStablecoinRequest,
    wallet: StockWalletEvidence,
    quote: StockDexQuote,
    cost: Option<StockChainCost>,
    mut blockers: Vec<String>,
    now: i64,
) -> Result<StockStablecoinPreview, String> {
    let (input, target) = request.amounts_raw()?;
    let expected = quote
        .output_raw
        .parse::<u64>()
        .map_err(|_| "输出金额无效")?;
    let minimum = quote
        .minimum_output_raw
        .parse::<u64>()
        .map_err(|_| "最低到账无效")?;
    if quote.input_mint != STOCK_SOLANA_USDT
        || quote.output_mint != comparison::SOLANA_USDC
        || quote.input_raw != input.to_string()
        || minimum == 0
        || minimum > expected
        || quote.requested_at_ms <= 0
        || quote.received_at_ms < quote.requested_at_ms
        || now < quote.received_at_ms
        || wallet.owner != request.wallet_address
        || wallet.mint != STOCK_SOLANA_USDT
    {
        return Err("兑换合约、金额或钱包与本次请求不一致".into());
    }
    let fresh_wallet = wallet.checked_at_ms > 0
        && now >= wallet.checked_at_ms
        && now - wallet.checked_at_ms <= 30_000;
    let input_sufficient = fresh_wallet
        .then(|| {
            wallet
                .stock_raw
                .as_deref()
                .and_then(|s| s.parse::<u128>().ok())
                .map(|n| n >= u128::from(input))
        })
        .flatten();
    if input_sufficient != Some(true) {
        blockers.push(
            if input_sufficient == Some(false) {
                "链上 USDT 不足，不能借用交易所余额"
            } else {
                "链上 USDT 余额未核实，未当作零"
            }
            .into(),
        );
    }
    blockers.extend(wallet.problems.iter().cloned());
    if !comparison::quote_current(&quote, now) {
        blockers.push("兑换报价已过期，请重新试算".into());
    }
    if quote.fee_bps.is_none_or(|n| n > 10_000)
        || quote
            .fee_mint
            .as_deref()
            .is_none_or(|m| ![STOCK_SOLANA_USDT, comparison::SOLANA_USDC].contains(&m))
    {
        blockers.push("路由费用或费用币种未核实，不能当作零费用".into());
    }
    let mut native_cost_usdc = None;
    let mut valid_until_ms = quote
        .requested_at_ms
        .saturating_add(comparison::STOCK_QUOTE_MAX_AGE_MS)
        .min(quote.expires_at_ms.unwrap_or(i64::MAX))
        .min(wallet.checked_at_ms.saturating_add(30_000));
    if let Some(c) = &cost {
        if c.asset != "USDT"
            || c.direction != StockChainDirection::Sell
            || c.quote != quote
            || c.wallet_address != request.wallet_address
            || c.mint.address != STOCK_SOLANA_USDT
            || c.mint.decimals != 6
            || c.mint.ui_multiplier != "1"
            || !c.mint.extensions.is_empty()
            || c.mint.next_change_at_ms.is_some()
        {
            return Err("兑换费用模拟与本次输入不一致".into());
        }
        valid_until_ms = valid_until_ms
            .min(c.valid_until_ms)
            .min(c.mint.checked_at_ms.saturating_add(60_000));
        if let Some(v) = &c.native_valuation {
            valid_until_ms = valid_until_ms
                .min(
                    v.quote
                        .requested_at_ms
                        .saturating_add(comparison::STOCK_QUOTE_MAX_AGE_MS),
                )
                .min(v.quote.expires_at_ms.unwrap_or(i64::MAX))
                .min(
                    v.replenishment
                        .as_ref()
                        .map_or(i64::MAX, |p| p.valid_until_ms),
                );
        }
        blockers.extend(c.problems.iter().cloned());
        if !c.simulation_passed || c.simulation_slot.is_none_or(|s| s == 0) {
            blockers.push("兑换交易模拟未通过，尚未发送".into());
        }
        if c.checked_at_ms <= 0
            || c.mint.checked_at_ms <= 0
            || c.mint.slot == 0
            || now < c.checked_at_ms
            || now < c.mint.checked_at_ms
        {
            blockers.push("费用或 Mint 时戳无效".into());
        }
        if c.network_fee_lamports
            .as_deref()
            .is_none_or(|s| s.parse::<u64>().is_err())
        {
            blockers.push("交易网络费未核实，不能当作零".into());
        }
        native_cost_usdc = c.complete_native_usdc_budget(now);
        let required = c.total_native_required_lamports(now);
        let available = fresh_wallet
            .then(|| {
                wallet
                    .sol_lamports
                    .as_deref()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .flatten();
        match required.zip(available) {
            Some((r, a)) if a >= r => {}
            Some(_) => blockers.push("SOL 周转余额不足，兑换前需补入".into()),
            None => blockers.push("SOL 周转余额或所需备款未核实".into()),
        }
    } else {
        blockers.push("尚未取得未签名交易的费用模拟，报价不代表可执行".into());
    }
    if native_cost_usdc.is_none() {
        blockers.push("SOL 成本尚未完整折合 USDC，未当作零".into());
    }
    let minimum_usdc = token_amount(minimum);
    let after_native_cost_usdc = native_cost_usdc
        .as_deref()
        .and_then(|n| Decimal::from_str_exact(n).ok())
        .and_then(|n| {
            Decimal::from(minimum)
                .checked_div(Decimal::from(1_000_000))?
                .checked_sub(n)
        })
        .map(|n| n.normalize().to_string());
    let retained = after_native_cost_usdc
        .as_deref()
        .and_then(|s| Decimal::from_str_exact(s).ok())
        .unwrap_or(Decimal::from(minimum) / Decimal::from(1_000_000));
    let shortfall =
        (Decimal::from(target) / Decimal::from(1_000_000) - retained).max(Decimal::ZERO);
    let shortfall_usdc = shortfall.normalize().to_string();
    if shortfall > Decimal::ZERO {
        blockers.push("最低到账扣除已知 SOL 补回预算后不足目标，不会自动提高 USDT 投入".into());
    }
    if native_cost_usdc.is_some() && after_native_cost_usdc.is_none() {
        blockers.push("SOL 补回预算无法完整计算，不能保存兑换计划".into());
    }
    blockers.sort();
    blockers.dedup();
    Ok(StockStablecoinPreview {
        request,
        quote,
        wallet,
        cost,
        checked_at_ms: now,
        valid_until_ms,
        minimum_usdc,
        native_cost_usdc,
        after_native_cost_usdc,
        shortfall_usdc,
        input_sufficient,
        blockers,
    })
}

fn token_amount(raw: u64) -> String {
    (Decimal::from(raw) / Decimal::from(1_000_000))
        .normalize()
        .to_string()
}

#[cfg(test)]
mod tests;
