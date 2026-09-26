use super::*;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockInventorySource {
    pub plan_id: String,
    pub revision: u64,
}

impl StockExecutionPlan {
    pub fn inventory_source(&self) -> StockInventorySource {
        StockInventorySource {
            plan_id: self.plan_id.clone(),
            revision: self.revision,
        }
    }

    /// Original size is a replenishment benchmark, never a new executable quote.
    pub fn restock_direction(&self) -> Result<StockPreflightDirection, String> {
        let report = self.accounting();
        if self.phase != StockPlanPhase::Settled
            || !report.can_settle()
            || self
                .settlement
                .as_ref()
                .is_none_or(|s| s.accounting != report)
            || self.terms.allocations.is_empty()
        {
            return Err("先核清两腿并完成归档，再检查下一笔库存".into());
        }
        let mut requirements = std::collections::BTreeMap::new();
        for a in &self.terms.allocations {
            let asset = match a.asset.as_str() {
                "SOL / 保守周转余额" => "SOL",
                "USDC / SOL 补仓" => "USDC",
                a => a,
            };
            if !matches!(a.location.as_str(), "Solana" | "Backpack")
                || !["USDC", "SOL", self.request.asset.as_str()].contains(&asset)
                || stock_exact_decimal(&a.quantity).map_or(true, |n| n < Decimal::ZERO)
            {
                return Err("原计划库存备款不完整，不能据此补库".into());
            }
            let total = requirements.entry((a.location.clone(),asset.to_owned())).or_insert(Decimal::ZERO);
            *total = total.checked_add(stock_exact_decimal(&a.quantity).map_err(str::to_owned)?).ok_or("原规模备款溢出")?;
        }
        let inventory = requirements.into_iter().map(|((location,asset),required)|StockInventoryRequirement {
                location,
                asset,
                required: Some(required.normalize().to_string()),
                available: None,
                sufficient: None,
            }).collect();
        Ok(StockPreflightDirection {
            direction: self.request.direction.label().into(),
            inventory,
            gross_usdc: None,
            cex_fee_usdc: None,
            native_fee_usdc: None,
            after_known_costs_usdc: None,
            fee_basis: "按原计划规模复查库存；行情、费用和收益须另行构建".into(),
            transfer_problem: None,
            blockers: vec![],
            executable: false,
        })
    }

    pub fn restock_report(
        &self,
        snapshot: &StockMarketSnapshot,
        account: &StockAccountEvidence,
        wallet: &StockWalletEvidence,
        now: i64,
    ) -> Result<StockPreflight, String> {
        let mut row = self.restock_direction()?;
        let mint = &snapshot
            .comparison
            .as_ref()
            .ok_or("请先更新该股票的合约与份额")?
            .mint;
        let original = &self.terms.chain_cost.mint;
        if snapshot.security.as_ref().is_none_or(|s| {
            s.asset != self.terms.security.asset || s.cusip != self.terms.security.cusip
        }) || snapshot
            .comparison
            .as_ref()
            .is_none_or(|c| c.asset != self.request.asset)
            || account.fingerprint != self.terms.account_fingerprint
            || account.liquidating
            || wallet.owner != self.request.wallet_address
            || wallet.mint != original.address
            || mint.address != original.address
            || mint.decimals != original.decimals
            || stock_exact_decimal(&mint.ui_multiplier).ok()
                != stock_exact_decimal(&original.ui_multiplier).ok()
            || now < mint.checked_at_ms
            || now - mint.checked_at_ms > 60_000
            || mint.next_change_at_ms.is_some_and(|t| now >= t)
            || now < account.balances_at_ms
            || account.balances_at_ms < self.updated_at_ms
            || now - account.balances_at_ms > 30_000
            || now < wallet.checked_at_ms
            || wallet.checked_at_ms < self.updated_at_ms
            || now - wallet.checked_at_ms > 30_000
            || !wallet.problems.is_empty()
        {
            return Err("原计划账户、钱包、股票份额或当前余额已变化，不能沿用旧补库基准".into());
        }
        for i in &mut row.inventory {
            let available = funding::balance(
                snapshot,
                Some(account),
                Some(wallet),
                &i.location,
                &i.asset,
                now,
            );
            i.available = available.map(|n| n.normalize().to_string());
            i.sufficient = available
                .zip(
                    i.required
                        .as_deref()
                        .and_then(|n| stock_exact_decimal(n).ok()),
                )
                .map(|(a, r)| a >= r);
        }
        let rows = vec![row];
        Ok(StockPreflight {
            source_plan: Some(self.inventory_source()),
            funding: funding::evaluate_funding(snapshot, &rows, Some(account), Some(wallet), now),
            asset: self.request.asset.clone(),
            wallet_address: Some(wallet.owner.clone()),
            checked_at_ms: now,
            valid_until_ms: (now + 30_000)
                .min(account.balances_at_ms + 30_000)
                .min(wallet.checked_at_ms + 30_000)
                .min(mint.checked_at_ms + 60_000)
                .min(mint.next_change_at_ms.unwrap_or(i64::MAX)),
            price_basis: StockPriceBasis::from_snapshot(snapshot),
            spot_taker_fee_pct: None,
            account_at_ms: Some(account.balances_at_ms),
            wallet_at_ms: Some(wallet.checked_at_ms),
            directions: rows,
            problems: vec![],
        })
    }
}
