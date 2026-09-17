use super::*;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockRecoveryBuildRequest {
    pub plan_id: String,
    pub revision: u64,
    pub max_loss_usdc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockRecoveryActionRequest {
    pub plan_id: String,
    pub revision: u64,
    pub index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRecovery {
    pub source_revision: u64,
    pub prepared_at_ms: i64,
    pub max_loss_usdc: String,
    pub target: StockRecoveryTarget,
    pub cost: StockChainCost,
    pub wallet: StockWalletEvidence,
    pub minimum_net_usdc: String,
    pub cancelled_at_ms: Option<i64>,
    pub submission: Option<StockChainSubmission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRecoveryTarget {
    pub direction: StockChainDirection,
    pub stock_raw: String,
    pub stock_shares: String,
}

pub fn stock_recovery_loss_limit(value: &str) -> Option<Decimal> {
    Decimal::from_str_exact(value)
        .ok()
        .filter(|v| *v >= Decimal::ZERO && *v <= Decimal::from(100_000))
}

/// A failed Solana instruction rolls token changes back; only proven fees remain.
pub fn stock_receipt_recoverable(r: &StockChainReceipt) -> bool {
    if r.succeeded {
        r.within_plan && r.problems.is_empty()
    } else {
        !r.within_plan
            && r.problems.len() == 1
            && !r.asset_changes.is_empty()
            && r.network_fee_lamports.parse::<u64>().is_ok()
            && r.asset_changes
                .iter()
                .all(|a| a.raw_change.parse::<i128>() == Ok(0))
            && r.wallet_native_change_lamports
                .parse::<i128>()
                .is_ok_and(|n| n <= 0)
    }
}

impl StockExecutionPlan {
    pub fn recovery_target(&self) -> Result<StockRecoveryTarget, String> {
        let order = self
            .cex_order
            .as_ref()
            .ok_or("RFQ 实际费用尚未核齐，不能自动编制补偿")?;
        let chain = self
            .chain_submission
            .as_ref()
            .and_then(|s| s.receipt.as_ref())
            .ok_or("先核对原链上最终回执")?;
        if self.phase != StockPlanPhase::SubmissionUnknown
            || self.two_leg_started_at_ms.is_none()
            || !order.receipt_complete()
            || !stock_receipt_recoverable(chain)
            || !self.native_topups.is_empty()
            || self.recoveries.iter().any(|r| {
                r.submission.as_ref().is_some_and(|s| {
                    s.receipt
                        .as_ref()
                        .is_none_or(|r| !stock_receipt_recoverable(r))
                })
            })
        {
            return Err("原交易、费用或既有补偿未核齐，不能再次补偿".into());
        }
        let report = self.accounting();
        if report.fee_basis_matched != Some(true) || report.net_usdc_change.is_none() {
            return Err("实际手续费或 USDC 收支未知，不能编制补偿".into());
        }
        let shares = report
            .net_stock_shares
            .as_deref()
            .and_then(|s| Decimal::from_str_exact(s).ok())
            .ok_or("实际股票差额未知")?;
        let complete = order.phase == StockCexOrderPhase::Filled
            && chain.succeeded
            && order
                .executed_quantity
                .as_deref()
                .and_then(|s| Decimal::from_str_exact(s).ok())
                == Decimal::from_str_exact(&self.terms.cex_shares).ok();
        let repaired = self
            .recoveries
            .iter()
            .rev()
            .find_map(|r| r.submission.as_ref())
            .and_then(|s| s.receipt.as_ref())
            .is_some_and(|r| r.succeeded && stock_receipt_recoverable(r));
        if shares.is_zero() || ((complete || repaired) && shares >= Decimal::ZERO) {
            return Err("没有需要补偿的股票缺口".into());
        }
        let scale = Decimal::from(
            10u64
                .checked_pow(u32::from(self.terms.chain_cost.mint.decimals))
                .ok_or("股票精度无效")?,
        );
        let multiplier = Decimal::from_str_exact(&self.terms.chain_cost.mint.ui_multiplier)
            .ok()
            .filter(|n| *n > Decimal::ZERO)
            .ok_or("股票份额倍率未知")?;
        let raw = shares
            .abs()
            .checked_mul(scale)
            .and_then(|n| n.checked_div(multiplier))
            .ok_or("股票补偿数量溢出")?;
        let raw = if shares < Decimal::ZERO {
            raw.ceil()
        } else {
            raw.floor()
        };
        if raw <= Decimal::ZERO {
            return Err("股票差额小于一个链上单位，需要人工核对余量".into());
        }
        let stock_raw = raw.normalize().to_string();
        stock_raw
            .parse::<u64>()
            .map_err(|_| "股票补偿数量超出范围")?;
        Ok(StockRecoveryTarget {
            direction: if shares < Decimal::ZERO {
                StockChainDirection::Buy
            } else {
                StockChainDirection::Sell
            },
            stock_raw,
            stock_shares: shares.normalize().to_string(),
        })
    }
}
