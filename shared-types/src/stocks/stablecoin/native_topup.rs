use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockStablecoinNativeTopup {
    pub terms: StockNativeTopup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at_ms: Option<i64>,
}

impl StockStablecoinNativeTopup {
    pub fn current(&self, now: i64) -> bool {
        self.cancelled_at_ms.is_none()
            && self.terms.submission.is_none()
            && now >= self.terms.prepared_at_ms
            && self
                .terms
                .valuation
                .replenishment
                .as_ref()
                .is_some_and(|p| now < p.valid_until_ms)
    }

    pub fn holds_funds(&self, now: i64) -> bool {
        self.current(now)
            || self
                .terms
                .submission
                .as_ref()
                .is_some_and(|s| s.receipt.is_none())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockStablecoinNativeAccounting {
    pub retained_usdc_raw: i128,
    pub spent_usdc_raw: i128,
    pub net_native_lamports: i128,
    pub minimum_slot: u64,
}

impl StockStablecoinPlan {
    /// Actual wallet SOL changes already contain wallet-paid fees and retained rent.
    pub fn native_accounting(&self) -> Result<StockStablecoinNativeAccounting, String> {
        if self.phase != StockStablecoinPlanPhase::Completed
            || self.receipt_phase() != StockStablecoinPlanPhase::Completed
        {
            return Err("先核对原兑换到账，不能在未知收支上补回 SOL".into());
        }
        let main = self
            .submission
            .as_ref()
            .and_then(|s| s.receipt.as_ref())
            .ok_or("兑换回执缺失")?;
        let mut report = StockStablecoinNativeAccounting {
            retained_usdc_raw: stablecoin_change(main, comparison::SOLANA_USDC)
                .ok_or("USDC 到账未知")?,
            spent_usdc_raw: 0,
            net_native_lamports: main
                .wallet_native_change_lamports
                .parse()
                .map_err(|_| "SOL 变化未知")?,
            minimum_slot: main.slot,
        };
        for row in &self.native_topups {
            let Some(submission) = &row.terms.submission else {
                continue;
            };
            let receipt = submission
                .receipt
                .as_ref()
                .ok_or("SOL 补回已提交，先核对原交易")?;
            let native = receipt
                .wallet_native_change_lamports
                .parse::<i128>()
                .map_err(|_| "补回 SOL 收支未知")?;
            let cash =
                stablecoin_change(receipt, comparison::SOLANA_USDC).ok_or("补回 USDC 收支未知")?;
            let fee = receipt
                .network_fee_lamports
                .parse::<u64>()
                .map_err(|_| "补回网络费未知")?;
            let proof = row
                .terms
                .valuation
                .replenishment
                .as_ref()
                .ok_or("补回模拟缺失")?;
            let expected = row
                .terms
                .valuation
                .quote
                .input_raw
                .parse::<u64>()
                .map_err(|_| "补回金额无效")?;
            let minimum = proof
                .minimum_credit_lamports
                .parse::<u64>()
                .map_err(|_| "补回最低到账无效")?;
            let unique = receipt
                .asset_changes
                .iter()
                .map(|a| &a.mint)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == receipt.asset_changes.len();
            let valid = if receipt.succeeded {
                receipt.within_plan
                    && receipt.problems.is_empty()
                    && cash == -i128::from(expected)
                    && native >= i128::from(minimum)
                    && receipt.asset_changes.iter().all(|a| {
                        if a.mint == STOCK_WRAPPED_SOL {
                            a.decimals == 9 && a.raw_change == "0"
                        } else {
                            a.mint == comparison::SOLANA_USDC
                                || a.raw_change.parse::<i128>().is_ok_and(|n| n >= 0)
                        }
                    })
            } else {
                let wallet_fee = if receipt.fee_payer == self.request.conversion.wallet_address {
                    i128::from(fee)
                } else {
                    0
                };
                receipt.problems.len() == 1
                    && native == -wallet_fee
                    && receipt.asset_changes.iter().all(|a| a.raw_change == "0")
            };
            if !valid || !unique || receipt.slot < proof.simulation_slot {
                return Err("SOL 补回收支不符，保留占用，不能继续补回".into());
            }
            report.retained_usdc_raw = report
                .retained_usdc_raw
                .checked_add(cash)
                .ok_or("USDC 收支溢出")?;
            report.spent_usdc_raw = report
                .spent_usdc_raw
                .checked_sub(cash)
                .ok_or("USDC 费用溢出")?;
            report.net_native_lamports = report
                .net_native_lamports
                .checked_add(native)
                .ok_or("SOL 收支溢出")?;
            report.minimum_slot = report.minimum_slot.max(receipt.slot);
        }
        Ok(report)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockStablecoinTopupSubmitRequest {
    pub plan_id: String,
    pub revision: u64,
    pub index: usize,
    pub confirm_live: bool,
}
