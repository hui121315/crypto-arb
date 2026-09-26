use super::*;
use rust_decimal::Decimal;

pub const MAX_STOCK_PEER_RECOVERIES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerRecoveryRequest {
    pub plan_id: String,
    pub revision: u64,
    pub usdc_limit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerRecoverySubmitRequest {
    pub plan_id: String,
    pub revision: u64,
    pub index: usize,
    pub confirm_live: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerRecovery {
    pub source_revision: u64,
    pub prepared_at_ms: i64,
    // Buy: maximum debit; sell: minimum credit, after the quoted SOL replacement budget.
    // This is not a combined USD/USDC profit or loss limit.
    pub usdc_limit: String,
    pub target: StockRecoveryTarget,
    pub cost: StockChainCost,
    pub wallet: StockWalletEvidence,
    pub cancelled_at_ms: Option<i64>,
    pub submission: Option<StockChainSubmission>,
}

pub fn stock_peer_recovery_limit(value: &str) -> Option<Decimal> {
    stock_exact_decimal(value).ok().filter(|v| {
        *v > Decimal::ZERO && *v <= Decimal::from(100_000) && v.normalize().scale() <= 6
    })
}

impl StockPeerPlan {
    pub fn peer_recovery_target(&self) -> Result<StockRecoveryTarget, String> {
        self.accounting()
            .recovery_target
            .ok_or("原回执、费用或实际股票差额不满足补偿条件".into())
    }
    pub fn peer_recovery_available(&self, now: i64) -> bool {
        self.phase == StockPeerPlanPhase::SubmissionUnknown
            && self.recoveries.len() < MAX_STOCK_PEER_RECOVERIES
            && self.peer_inventory_idle(now)
            && self.peer_dispositions_idle(now)
            && !self.recoveries.last().is_some_and(|r| {
                r.submission.as_ref().is_some_and(|s| s.receipt.is_none())
                    || (r.submission.is_none()
                        && r.cancelled_at_ms.is_none()
                        && now < r.cost.valid_until_ms)
            })
    }
    pub fn peer_minimum_slot(&self) -> Result<u64, String> {
        self.chain_submission
            .as_ref()
            .and_then(|s| s.receipt.as_ref())
            .map(|r| r.slot)
            .into_iter()
            .chain(
                self.recoveries
                    .iter()
                    .filter_map(|r| r.submission.as_ref()?.receipt.as_ref().map(|r| r.slot)),
            )
            .chain(self.native_topups.iter().filter_map(|r| {
                r.terms
                    .submission
                    .as_ref()?
                    .receipt
                    .as_ref()
                    .map(|r| r.slot)
            }))
            .max()
            .ok_or("原链上最终区块未知".into())
    }
}

impl StockPeerRecovery {
    pub fn conservative_usdc(&self, now: i64) -> Option<String> {
        let q = &self.cost.quote;
        let raw = if self.target.direction == StockChainDirection::Buy {
            &q.input_raw
        } else {
            &q.minimum_output_raw
        };
        let cash = Decimal::from(raw.parse::<u64>().ok()?) / Decimal::from(1_000_000);
        let cash = if self.target.direction == StockChainDirection::Buy {
            -cash
        } else {
            cash
        };
        let gas = stock_exact_decimal(&self.cost.complete_native_usdc_budget(now)?).ok()?;
        cash.checked_sub(gas).map(|v| v.normalize().to_string())
    }
    pub fn validate(&self, p: &StockPeerPlan, now: i64) -> Result<(), String> {
        let target = p.peer_recovery_target()?;
        let c = &self.cost;
        let m = &p.terms.basis.chain_cost.mint;
        let limit = stock_peer_recovery_limit(&self.usdc_limit)
            .ok_or("USDC 限额须大于 0、不超过 100000，最多六位小数")?;
        if self.target != target
            || c.asset != p.request.asset
            || c.direction != target.direction
            || c.wallet_address != p.request.wallet_address
            || c.mint.address != m.address
            || c.mint.decimals != m.decimals
            || stock_exact_decimal(&c.mint.ui_multiplier)? != stock_exact_decimal(&m.ui_multiplier)?
            || c.mint.slot < p.peer_minimum_slot()?
            || c.mint.checked_at_ms > now
            || now - c.mint.checked_at_ms > 60_000
            || c.mint.next_change_at_ms.is_some_and(|t| now >= t)
            || c.checked_at_ms > now
            || now >= c.valid_until_ms
            || c.valid_until_ms
                > c.quote
                    .requested_at_ms
                    .saturating_add(comparison::STOCK_QUOTE_MAX_AGE_MS)
            || !comparison::quote_current(&c.quote, now)
            || !c.simulation_passed
            || !c.problems.is_empty()
            || c.simulation_slot.is_none_or(|s| s < c.mint.slot)
        {
            return Err("补偿的原差额、合约、份额、钱包或报价已变化".into());
        }
        let raw = target
            .stock_raw
            .parse::<u64>()
            .map_err(|_| "补偿股票数量无效")?;
        let input = c
            .quote
            .input_raw
            .parse::<u64>()
            .map_err(|_| "补偿输入无效")?;
        let output = c
            .quote
            .minimum_output_raw
            .parse::<u64>()
            .map_err(|_| "补偿最低到账无效")?;
        let buying = target.direction == StockChainDirection::Buy;
        if input == 0
            || output == 0
            || if buying {
                c.quote.input_mint != comparison::SOLANA_USDC
                    || c.quote.output_mint != m.address
                    || output < raw
            } else {
                c.quote.input_mint != m.address
                    || c.quote.output_mint != comparison::SOLANA_USDC
                    || input != raw
            }
        {
            return Err("补偿数量或资产路径不能覆盖原差额".into());
        }
        let net = stock_exact_decimal(
            &self
                .conservative_usdc(now)
                .ok_or("补偿 SOL 费用与补回预算尚未核实")?,
        )?;
        if (buying && net < -limit) || (!buying && net < limit) {
            return Err("新报价在预留 SOL 补回成本后不满足本次 USDC 限额".into());
        }
        let w = &self.wallet;
        let usdc = w
            .usdc_raw
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .map(|v| Decimal::from(v) / Decimal::from(1_000_000));
        if w.owner != p.request.wallet_address
            || w.mint != m.address
            || !w.problems.is_empty()
            || w.checked_at_ms > now
            || now - w.checked_at_ms > 30_000
            || usdc
                .and_then(|u| u.checked_add(net))
                .is_none_or(|u| u < Decimal::ZERO)
            || w.sol_lamports
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .zip(c.total_native_required_lamports(now))
                .is_none_or(|(a, b)| a < b)
            || (!buying
                && w.stock_raw
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_none_or(|v| v < raw))
        {
            return Err("补偿钱包的股票、USDC 或 SOL 余额不足或未核实".into());
        }
        Ok(())
    }
}
