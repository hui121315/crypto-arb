use super::*;
use rust_decimal::Decimal;

pub const MAX_STOCK_PEER_NATIVE_TOPUPS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerNativeTopupRequest {
    pub plan_id: String,
    pub revision: u64,
    pub usdc_limit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerNativeTopup {
    pub terms: StockNativeTopup,
    pub usdc_limit: String,
    pub cancelled_at_ms: Option<i64>,
}

impl StockPeerPlan {
    pub fn peer_native_target(&self) -> Result<(u64, u64), String> {
        let a = self.accounting();
        if self.phase != StockPeerPlanPhase::SubmissionUnknown
            || a.status != StockAccountingStatus::LegsReconciled
            || a.fee_budget_matched != Some(true)
            || a.recovery_target.is_some()
        {
            return Err("先核齐双边及补偿、换汇的实际收支，再补回 SOL".into());
        }
        let sol = stock_exact_decimal(a.wallet_sol_change.as_deref().ok_or("实际 SOL 扣款未知")?)?;
        let raw = sol
            .checked_mul(Decimal::from(1_000_000_000))
            .filter(|n| *n < Decimal::ZERO && n.fract().is_zero())
            .and_then(|n| (-n).normalize().to_string().parse::<u64>().ok())
            .ok_or("没有需要补回的已核实 SOL 扣款")?;
        let slot = self
            .native_topups
            .iter()
            .filter_map(|r| {
                r.terms
                    .submission
                    .as_ref()?
                    .receipt
                    .as_ref()
                    .map(|r| r.slot)
            })
            .chain(std::iter::once(self.peer_minimum_slot()?))
            .max()
            .unwrap();
        Ok((raw, slot))
    }

    pub fn peer_native_available(&self, now: i64) -> bool {
        self.native_topups.len() < MAX_STOCK_PEER_NATIVE_TOPUPS
            && self.peer_inventory_idle(now)
            && self.peer_native_target().is_ok()
            && !self.recoveries.iter().any(|r| {
                r.submission.is_none() && r.cancelled_at_ms.is_none() && now < r.cost.valid_until_ms
            })
            && !self
                .conversions
                .iter()
                .any(|r| r.order.is_none() && r.cancelled_at_ms.is_none() && now < r.valid_until_ms)
            && !self.native_topups.iter().any(|r| {
                r.current(now)
                    || r.terms
                        .submission
                        .as_ref()
                        .is_some_and(|s| s.receipt.is_none())
            })
    }
}

impl StockPeerNativeTopup {
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

    pub fn cost(&self, p: &StockPeerPlan) -> Result<StockChainCost, String> {
        let mut cost = self
            .terms
            .valuation
            .execution_cost(&p.terms.basis.chain_cost)?;
        // Subsequent wallet reads must not use a slot older than this replenishment simulation.
        cost.mint.slot = cost
            .mint
            .slot
            .max(cost.simulation_slot.ok_or("补回模拟区块缺失")?);
        Ok(cost)
    }

    pub fn validate(&self, p: &StockPeerPlan, now: i64) -> Result<(), String> {
        let (target, slot) = p.peer_native_target()?;
        let limit =
            stock_peer_recovery_limit(&self.usdc_limit).ok_or("SOL 补回的 USDC 限额无效")?;
        let v = &self.terms.valuation;
        let budget = v
            .complete_budget(&target.to_string(), &p.request.wallet_address, now)
            .ok_or("SOL 补回报价、目标或自身费用未核实")?;
        let proof = v.replenishment.as_ref().ok_or("SOL 补回模拟缺失")?;
        let w = &self.terms.wallet;
        if !p.peer_native_available(now)
            || self.terms.source_revision > p.revision
            || self.terms.prepared_at_ms > now
            || stock_exact_decimal(&budget)? > limit
            || proof.simulation_slot < slot
            || w.owner != p.request.wallet_address
            || w.mint != p.terms.basis.chain_cost.mint.address
            || !w.problems.is_empty()
            || w.checked_at_ms <= 0
            || w.checked_at_ms > now
            || now - w.checked_at_ms > 30_000
            || w.usdc_raw
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .zip(v.quote.input_raw.parse::<u64>().ok())
                .is_none_or(|(a, b)| a < b)
            || w.sol_lamports
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .zip(proof.wallet_required_lamports.parse::<u64>().ok())
                .is_none_or(|(a, b)| a < b)
        {
            return Err("SOL 补回超出限额，或原收支、当前钱包与模拟区块未核齐".into());
        }
        self.cost(p)?;
        Ok(())
    }

    /// Net wallet SOL already includes wallet-paid fees; do not deduct them twice.
    pub fn observed_changes(&self) -> Result<Option<(i128, i128, u64)>, String> {
        let Some(s) = &self.terms.submission else {
            return Ok(None);
        };
        let Some(r) = &s.receipt else {
            return Ok(None);
        };
        let proof = self
            .terms
            .valuation
            .replenishment
            .as_ref()
            .ok_or("SOL 补回模拟缺失")?;
        let raw = |s: &str| s.parse::<i128>().map_err(|_| "补回原始收支无效".to_owned());
        let sol = raw(&r.wallet_native_change_lamports)?;
        let fee = r
            .network_fee_lamports
            .parse::<u64>()
            .map_err(|_| "补回网络费未知")?;
        let mut seen = std::collections::BTreeSet::new();
        let mut cash = None;
        for a in &r.asset_changes {
            if !seen.insert(&a.mint) {
                return Err("补回回执资产重复".into());
            }
            let n = raw(&a.raw_change)?;
            if a.mint == comparison::SOLANA_USDC && a.decimals == 6 {
                cash = Some(n);
            } else if (a.mint != STOCK_WRAPPED_SOL || a.decimals == 9) && n == 0 {
                // Unchanged stock/WSOL rows may be included by the original receipt parser.
            } else {
                return Err("SOL 补回包含其他资产变动，需独立核账".into());
            }
        }
        let cash = cash.ok_or("补回 USDC 收支缺失，不能当作零")?;
        if s.transaction_id.as_ref() != Some(&r.transaction_id)
            || r.slot < proof.simulation_slot
            || r.asset_changes.len() > 256
        {
            return Err("SOL 补回回执身份或区块不符合原交易".into());
        }
        Ok(Some((cash, sol, fee)))
    }

    pub fn actual_changes(&self, p: &StockPeerPlan) -> Result<Option<(i128, i128, u64)>, String> {
        let Some((cash, sol, fee)) = self.observed_changes()? else {
            return Ok(None);
        };
        let r = self
            .terms
            .submission
            .as_ref()
            .and_then(|s| s.receipt.as_ref())
            .ok_or("补回回执缺失")?;
        let proof = self
            .terms
            .valuation
            .replenishment
            .as_ref()
            .ok_or("补回模拟缺失")?;
        let raw = |s: &str| {
            s.parse::<u64>()
                .map(i128::from)
                .map_err(|_| "补回数量无效".to_owned())
        };
        if !stock_receipt_recoverable(r)
            || if r.succeeded {
                cash != -raw(&self.terms.valuation.quote.input_raw)?
                    || sol < raw(&proof.minimum_credit_lamports)?
            } else {
                cash != 0
                    || sol
                        != if r.fee_payer == p.request.wallet_address {
                            -i128::from(fee)
                        } else {
                            0
                        }
            }
        {
            return Err("SOL 补回实际收支不符合原交易，保留占用".into());
        }
        Ok(Some((cash, sol, fee)))
    }
}
