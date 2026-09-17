use super::*;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockAccountingStatus {
    AwaitingReceipts,
    NeedsReview,
    LegsReconciled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockAssetMovement {
    pub location: String,
    pub asset: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPlanAccounting {
    pub status: StockAccountingStatus,
    pub movements: Vec<StockAssetMovement>,
    pub net_usdc_change: Option<String>,
    pub net_stock_shares: Option<String>,
    pub net_sol_change: Option<String>,
    pub cex_fee_usdc: Option<String>,
    pub fee_basis_matched: Option<bool>,
    pub problems: Vec<String>,
}

impl StockPlanAccounting {
    pub fn can_settle(&self) -> bool {
        self.status == StockAccountingStatus::LegsReconciled
            && self.problems.is_empty()
            && self.fee_basis_matched == Some(true)
            && self.net_usdc_change.as_deref().and_then(decimal).is_some()
            && [&self.net_sol_change, &self.net_stock_shares]
                .iter()
                .all(|n| {
                    n.as_deref()
                        .and_then(decimal)
                        .is_some_and(|v| v >= Decimal::ZERO)
                })
    }
}

impl StockExecutionPlan {
    /// Derived from the original receipts, never from marks or a fresh account balance.
    pub fn accounting(&self) -> StockPlanAccounting {
        let mut out = StockPlanAccounting {
            status: StockAccountingStatus::AwaitingReceipts,
            movements: vec![],
            net_usdc_change: None,
            net_stock_shares: None,
            net_sol_change: None,
            cex_fee_usdc: None,
            fee_basis_matched: None,
            problems: vec![],
        };
        let mut review = false;
        let mut incomplete_execution = false;
        let mut cex_changes = None;
        let mut chain_changes = None;
        if let Some(order) = &self.cex_order {
            if let Some(instruction) = &self.terms.cex_instruction {
                cex_changes = order.net_asset_changes(instruction);
            }
            if order.evidence_conflict {
                review = true;
                out.problems.push("交易所原始回执存在冲突".into());
            }
            if !order.receipt_complete() {
                out.problems.push("交易所成交明细或实际扣费尚未齐全".into());
            } else {
                if order.phase != StockCexOrderPhase::Filled
                    || order.executed_quantity.as_deref().and_then(decimal)
                        != decimal(&self.terms.cex_shares)
                {
                    incomplete_execution = true;
                }
                let fee = order.fills.iter().try_fold(Decimal::ZERO, |sum, f| {
                    let fee = f.fee.as_ref()?;
                    if fee.asset != "USDC" {
                        return None;
                    }
                    sum.checked_add(decimal(&fee.quantity)?)
                });
                out.cex_fee_usdc = fee.map(amount);
                if let Some(budget) = &self.terms.cex_fee_budget {
                    if let StockCexFeeBasis::OrderBookQuote { taker_bps, .. } = &budget.basis {
                        out.fee_basis_matched =
                            fee.zip(order.fill_totals()).and_then(|(f, (_, v))| {
                                let allowed = v
                                    .checked_mul(decimal(taker_bps)?)?
                                    .checked_div(Decimal::from(10_000))?;
                                Some(f <= allowed)
                            });
                        if out.fee_basis_matched != Some(true) {
                            review = true;
                            out.problems
                                .push("实际手续费币种或费率偏离预算，已保留原币扣款".into());
                        }
                    }
                } else {
                    out.problems.push("旧计划没有绑定手续费预算".into());
                }
            }
        } else if let Some(rfq) = &self.rfq_acceptance {
            if rfq
                .acceptance
                .as_ref()
                .is_some_and(|a| a.evidence_conflict || a.rejected)
                || matches!(
                    rfq.phase,
                    StockRfqPhase::Cancelled | StockRfqPhase::Expired | StockRfqPhase::Rejected
                )
            {
                review = true;
                out.problems
                    .push("RFQ 未完成结算或回执存在冲突，需要核对敞口".into());
            }
            out.problems.push(
                if rfq.phase == StockRfqPhase::Filled && !rfq.settlement_pending() {
                    "RFQ 成交额已核对，但请求方回执没有实际扣费明细，净到账仍待核实"
                } else {
                    "RFQ 尚未取得完整结算回执，接受报价不等于到账"
                }
                .into(),
            );
        } else {
            out.problems.push("尚无交易所腿回执".into());
        }
        if let Some(receipt) = self
            .chain_submission
            .as_ref()
            .and_then(|r| r.receipt.as_ref())
        {
            let mut changes = BTreeMap::new();
            let mut valid = true;
            for change in &receipt.asset_changes {
                let quantity = stock_chain_quantity(&change.raw_change, change.decimals)
                    .and_then(|q| decimal(&q));
                let (asset, value) = if change.mint == self.terms.chain_cost.mint.address {
                    (
                        self.request.asset.clone(),
                        quantity.and_then(|n| {
                            (change.decimals == self.terms.chain_cost.mint.decimals)
                                .then_some(())?;
                            n.checked_mul(decimal(&self.terms.chain_cost.mint.ui_multiplier)?)
                        }),
                    )
                } else if change.mint == comparison::SOLANA_USDC {
                    ("USDC".into(), quantity.filter(|_| change.decimals == 6))
                } else {
                    (change.mint.clone(), quantity)
                };
                if let Some(n) = value {
                    if changes.insert(asset, amount(n)).is_some() {
                        valid = false;
                    }
                } else {
                    valid = false;
                }
            }
            out.net_sol_change = stock_chain_quantity(&receipt.wallet_native_change_lamports, 9);
            if let Some(sol) = &out.net_sol_change {
                changes.insert("SOL".into(), sol.clone());
            } else {
                valid = false;
            }
            if valid {
                chain_changes = Some(changes);
            } else {
                review = true;
                out.problems.push("链上实际数量或份额换算凭据不完整".into());
            }
            if !stock_receipt_recoverable(receipt) {
                review = true;
                out.problems
                    .push("链上交易失败或实际收支偏离原计划，需要处置".into());
            } else if !receipt.succeeded {
                incomplete_execution = true;
            }
        } else {
            out.problems
                .push("尚无链上最终回执，Provider 回复不代表成交".into());
        }
        if let Some(changes) = &mut chain_changes {
            for recovery in &self.recoveries {
                let Some(submission) = &recovery.submission else {
                    continue;
                };
                let Some(receipt) = &submission.receipt else {
                    out.problems
                        .push("补偿交易尚无最终回执，不能继续处置或释放占用".into());
                    continue;
                };
                if !stock_receipt_recoverable(receipt)
                    || !merge_recovery(changes, receipt, &recovery.cost, &self.request.asset)
                {
                    review = true;
                    out.problems
                        .push("补偿实际收支异常，不能按预估金额收尾".into());
                }
            }
            for topup in &self.native_topups {
                let Some(submission) = &topup.submission else {
                    continue;
                };
                let Some(receipt) = &submission.receipt else {
                    out.problems
                        .push("SOL 补回交易尚无最终回执，保留占用".into());
                    continue;
                };
                let valid = merge_topup(changes, receipt);
                // A failed, fully rolled-back attempt still costs SOL; a later attempt may repair it.
                if !valid
                    || (receipt.succeeded && (!receipt.within_plan || !receipt.problems.is_empty()))
                    || (!receipt.succeeded && receipt.problems.len() != 1)
                {
                    review = true;
                    out.problems
                        .push("SOL 补回收支异常，需要按实际资产处置".into());
                }
            }
            out.net_sol_change = changes.get("SOL").cloned();
        }
        for (location, changes) in [("Backpack", &cex_changes), ("Solana", &chain_changes)] {
            if let Some(changes) = changes {
                out.movements
                    .extend(changes.iter().map(|(asset, n)| StockAssetMovement {
                        location: location.into(),
                        asset: asset.clone(),
                        quantity: n.clone(),
                    }));
            }
        }
        if let (Some(cex), Some(chain)) = (&cex_changes, &chain_changes) {
            let sum = |asset: &str| {
                let a = cex
                    .get(asset)
                    .map(|v| decimal(v))
                    .unwrap_or(Some(Decimal::ZERO))?;
                let b = chain
                    .get(asset)
                    .map(|v| decimal(v))
                    .unwrap_or(Some(Decimal::ZERO))?;
                a.checked_add(b)
            };
            out.net_usdc_change = sum("USDC").map(amount);
            out.net_stock_shares = sum(&self.request.asset).map(amount);
            if incomplete_execution {
                let repaired = sum(&self.request.asset).is_some_and(|n| n.is_zero())
                    || self
                        .recoveries
                        .iter()
                        .rev()
                        .find_map(|r| r.submission.as_ref())
                        .and_then(|s| s.receipt.as_ref())
                        .is_some_and(|r| r.succeeded && r.within_plan && r.problems.is_empty())
                        && sum(&self.request.asset).is_some_and(|n| n >= Decimal::ZERO);
                if !repaired {
                    review = true;
                    out.problems
                        .push("原两腿未足额完成，需要按实际股票差额补偿".into());
                }
            }
            if let Some(recovery) = self
                .recoveries
                .iter()
                .rev()
                .find(|r| r.submission.is_some())
            {
                if decimal(&recovery.max_loss_usdc)
                    .zip(sum("USDC"))
                    .is_none_or(|(limit, cash)| cash < -limit)
                {
                    review = true;
                    out.problems
                        .push("实际累计 USDC 损失超出补偿上限，需人工核对".into());
                }
            }
            if sum(&self.request.asset).is_none_or(|q| q < Decimal::ZERO) {
                review = true;
                out.problems.push("两腿股票份额未对齐，存在待补库存".into());
            }
            if out
                .net_sol_change
                .as_deref()
                .and_then(decimal)
                .is_some_and(|n| n < Decimal::ZERO)
            {
                out.problems
                    .push("实际净扣 SOL 尚未补回，USDC 差额不是全成本利润".into());
            }
            out.status = StockAccountingStatus::LegsReconciled;
        }
        if review {
            out.status = StockAccountingStatus::NeedsReview;
        }
        out
    }
}

fn merge_recovery(
    changes: &mut BTreeMap<String, String>,
    receipt: &StockChainReceipt,
    cost: &StockChainCost,
    asset: &str,
) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    let mut additions = vec![];
    for c in &receipt.asset_changes {
        if !seen.insert(c.mint.clone()) {
            return false;
        }
        let Some(q) = stock_chain_quantity(&c.raw_change, c.decimals)
            .as_deref()
            .and_then(decimal)
        else {
            return false;
        };
        if c.mint == cost.mint.address && c.decimals == cost.mint.decimals {
            let Some(q) = decimal(&cost.mint.ui_multiplier).and_then(|m| q.checked_mul(m)) else {
                return false;
            };
            additions.push((asset, q));
        } else if c.mint == comparison::SOLANA_USDC && c.decimals == 6 {
            additions.push(("USDC", q));
        } else if !q.is_zero() {
            return false;
        }
    }
    if !seen.contains(&cost.mint.address) || !seen.contains(comparison::SOLANA_USDC) {
        return false;
    }
    let Some(sol) = stock_chain_quantity(&receipt.wallet_native_change_lamports, 9)
        .as_deref()
        .and_then(decimal)
    else {
        return false;
    };
    additions.push(("SOL", sol));
    for (asset, n) in additions {
        let Some(total) = changes
            .get(asset)
            .map(|s| decimal(s))
            .unwrap_or(Some(Decimal::ZERO))
            .and_then(|old| old.checked_add(n))
        else {
            return false;
        };
        changes.insert(asset.into(), amount(total));
    }
    true
}

fn merge_topup(changes: &mut BTreeMap<String, String>, receipt: &StockChainReceipt) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    let mut additions = Vec::new();
    for change in &receipt.asset_changes {
        if !seen.insert(&change.mint) {
            return false;
        }
        let Some(n) = stock_chain_quantity(&change.raw_change, change.decimals)
            .as_deref()
            .and_then(decimal)
        else {
            return false;
        };
        if change.mint == comparison::SOLANA_USDC && change.decimals == 6 {
            if n > Decimal::ZERO || (!receipt.succeeded && !n.is_zero()) {
                return false;
            }
            additions.push(("USDC", n));
        } else if !n.is_zero() || (change.mint == STOCK_WRAPPED_SOL && change.decimals != 9) {
            return false;
        }
    }
    if !seen.contains(&comparison::SOLANA_USDC.to_owned()) {
        return false;
    }
    let Some(sol) = stock_chain_quantity(&receipt.wallet_native_change_lamports, 9)
        .as_deref()
        .and_then(decimal)
    else {
        return false;
    };
    additions.push(("SOL", sol));
    for (asset, n) in additions {
        let Some(total) = changes
            .get(asset)
            .map(|s| decimal(s))
            .unwrap_or(Some(Decimal::ZERO))
            .and_then(|old| old.checked_add(n))
        else {
            return false;
        };
        changes.insert(asset.into(), amount(total));
    }
    true
}

fn decimal(s: &str) -> Option<Decimal> {
    Decimal::from_str_exact(s).ok()
}
fn amount(n: Decimal) -> String {
    n.normalize().to_string()
}
