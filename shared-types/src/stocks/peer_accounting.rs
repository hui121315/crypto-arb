use super::*;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerAccounting {
    pub plan_id: String,
    pub source_revision: u64,
    pub status: StockAccountingStatus,
    pub movements: Vec<StockAssetMovement>,
    pub quote_asset: String,
    pub cex_fee_quote: Option<String>,
    pub fee_budget_matched: Option<bool>,
    pub cex_stock_shares: Option<String>,
    pub chain_stock_shares: Option<String>,
    pub net_stock_shares: Option<String>,
    pub wallet_sol_change: Option<String>,
    pub network_fee_sol: Option<String>,
    // Only known movements; AwaitingReceipts never implies a missing leg is zero.
    pub cash_totals: BTreeMap<String, String>,
    pub recovery_target: Option<StockRecoveryTarget>,
    pub problems: Vec<String>,
    pub remaining: Vec<String>,
}

fn decimal(s: &str) -> Result<Decimal, String> {
    stock_exact_decimal(s).map_err(str::to_owned)
}
fn amount(n: Decimal) -> String {
    n.normalize().to_string()
}
fn add(a: Decimal, b: Decimal) -> Result<Decimal, String> {
    a.checked_add(b)
        .filter(|v| v.checked_sub(a) == Some(b) && v.checked_sub(b) == Some(a))
        .ok_or("实际收支累加溢出或精度丢失".into())
}
fn quantity(raw: &str, decimals: u8) -> Result<Decimal, String> {
    stock_chain_quantity(raw, decimals)
        .ok_or("链上实际数量精度无效".into())
        .and_then(|s| decimal(&s))
}
fn movement(out: &mut StockPeerAccounting, location: &str, asset: &str, n: Decimal) {
    out.movements.push(StockAssetMovement {
        location: location.into(),
        asset: asset.into(),
        quantity: amount(n),
    });
}

impl StockPeerPlan {
    pub fn peer_settlement_problem(&self, now: i64) -> Option<String> {
        if self.phase != StockPeerPlanPhase::SubmissionUnknown {
            return Some("计划不在收尾阶段".into());
        }
        let a = self.accounting();
        if a.status != StockAccountingStatus::LegsReconciled
            || !a.problems.is_empty()
            || a.fee_budget_matched != Some(true)
            || a.recovery_target.is_some()
        {
            return Some("原交易、补偿或实际费用尚未核齐，不能释放占用".into());
        }
        if !self.peer_inventory_idle(now) || !self.peer_dispositions_idle(now) {
            return Some("还有待提交的处置报价或未明交易，请先取消报价或核对原交易".into());
        }
        if [
            &a.cex_stock_shares,
            &a.chain_stock_shares,
            &a.net_stock_shares,
        ]
        .iter()
        .any(|n| n.as_deref().and_then(|s| decimal(s).ok()) != Some(Decimal::ZERO))
        {
            return Some("两边股票库存尚未分别恢复，不同发行方股票不能互相抵消".into());
        }
        if a.wallet_sol_change
            .as_deref()
            .and_then(|s| decimal(s).ok())
            .is_none_or(|n| n < Decimal::ZERO)
        {
            return Some("钱包实际 SOL 扣款尚未补回".into());
        }
        if !a.cash_totals.contains_key("USDC")
            || !a.cash_totals.contains_key(&a.quote_asset)
            || a.cash_totals.iter().any(|(asset, n)| {
                (asset != "USDC" && asset != &a.quote_asset) || decimal(n).is_err()
            })
        {
            return Some("原币实际现金收支尚未核齐".into());
        }
        None
    }

    /// A native-currency receipt ledger, not a new mark-to-market profit estimate.
    pub fn accounting(&self) -> StockPeerAccounting {
        if self.phase == StockPeerPlanPhase::Settled {
            if let Some(s) = &self.settlement {
                return s.accounting.clone();
            }
        }
        let mut out = StockPeerAccounting {
            plan_id: self.plan_id.clone(),
            source_revision: self.revision,
            status: StockAccountingStatus::AwaitingReceipts,
            movements: vec![],
            quote_asset: self.terms.draft.quote_asset.clone(),
            cex_fee_quote: None,
            fee_budget_matched: None,
            cex_stock_shares: None,
            chain_stock_shares: None,
            net_stock_shares: None,
            wallet_sol_change: None,
            network_fee_sol: None,
            cash_totals: BTreeMap::new(),
            recovery_target: None,
            problems: vec![],
            remaining: vec![],
        };
        if self.phase != StockPeerPlanPhase::SubmissionUnknown {
            out.remaining.push("尚未提交双边交易".into());
            return out;
        }
        let mut cex_ready = false;
        match cex_changes(self) {
            Ok(Some(CexChanges {
                cash,
                stock,
                quote,
                fee,
                matched,
            })) => {
                out.cex_stock_shares = Some(amount(stock));
                out.cex_fee_quote = Some(amount(fee));
                out.fee_budget_matched = Some(matched);
                movement(&mut out, "Kraken", &cash.base_asset, stock);
                movement(&mut out, "Kraken", &cash.quote_asset, quote);
                out.cash_totals
                    .insert(cash.quote_asset.clone(), amount(quote));
                if !matched {
                    out.problems
                        .push("Kraken 实际扣费超过原费率预算，需核对后再处置".into());
                }
                if cash.quote_asset != "USDC" && !quote.is_zero() {
                    out.remaining.push(format!(
                        "{} 与 USDC 之间尚无实际换汇回执，不能把两种币直接相加",
                        cash.quote_asset
                    ));
                }
                cex_ready = true;
            }
            Ok(None) => out
                .remaining
                .push("Kraken 原订单终态、逐笔成交或实际手续费尚未核齐".into()),
            Err(e) => out.problems.push(e),
        }
        for (i, row) in self
            .inventory_orders
            .iter()
            .enumerate()
            .filter(|(_, r)| r.order.is_some())
        {
            match row.observed_changes() {
                Ok(c) => {
                    if let Err(e) = row.actual_changes() {
                        out.problems.push(e);
                        out.fee_budget_matched = Some(false);
                    }
                    let apply = (|| -> Result<(), String> {
                        let stock = decimal(&c.equity_shares_change)?;
                        let quote = decimal(&c.quote_change)?;
                        let total_stock = add(
                            decimal(out.cex_stock_shares.as_deref().ok_or("原股票数量未知")?)?,
                            stock,
                        )?;
                        let total_quote =
                            difference_total(&out.cash_totals, &c.quote_asset, &c.quote_change)?;
                        let fee = row.order.as_ref().unwrap().fills.iter().try_fold(
                            Decimal::ZERO,
                            |sum, f| {
                                f.fees
                                    .as_ref()
                                    .ok_or("实际费用未知")?
                                    .iter()
                                    .try_fold(sum, |sum, fee| add(sum, decimal(&fee.quantity)?))
                            },
                        )?;
                        let total_fee = add(
                            decimal(out.cex_fee_quote.as_deref().ok_or("原费用未知")?)?,
                            fee,
                        )?;
                        out.cex_stock_shares = Some(amount(total_stock));
                        out.cex_fee_quote = Some(amount(total_fee));
                        out.cash_totals
                            .insert(c.quote_asset.clone(), amount(total_quote));
                        let location = format!("Kraken · 库存恢复 {}", i + 1);
                        movement(&mut out, &location, &c.base_asset, stock);
                        movement(&mut out, &location, &c.quote_asset, quote);
                        Ok(())
                    })();
                    if let Err(e) = apply {
                        out.problems.push(e);
                        cex_ready = false;
                    }
                }
                Err(e) => {
                    cex_ready = false;
                    if row
                        .order
                        .as_ref()
                        .is_some_and(|o| o.evidence_conflict || o.receipt_complete())
                    {
                        out.problems.push(e);
                    } else {
                        out.remaining.push(e);
                    }
                }
            }
        }
        let mut chain_ready = true;
        let mut chain_stock = Decimal::ZERO;
        let mut chain_sol = Decimal::ZERO;
        let mut chain_fee = Decimal::ZERO;
        let chain_rows = std::iter::once((
            &self.terms.basis.chain_cost,
            self.chain_submission.as_ref(),
            "Solana".to_owned(),
        ))
        .chain(
            self.recoveries
                .iter()
                .enumerate()
                .filter(|(_, r)| r.submission.is_some())
                .map(|(i, r)| {
                    (
                        &r.cost,
                        r.submission.as_ref(),
                        format!("Solana · 补偿 {}", i + 1),
                    )
                }),
        );
        for (cost, submission, location) in chain_rows {
            match chain_changes(self, cost, submission) {
                Ok(Some(ChainChanges {
                    stock,
                    cash,
                    sol,
                    fee,
                    assets,
                })) => {
                    match add(chain_stock, stock)
                        .and_then(|a| Ok((a, add(chain_sol, sol)?, add(chain_fee, fee)?)))
                    {
                        Ok((a, b, c)) => {
                            chain_stock = a;
                            chain_sol = b;
                            chain_fee = c;
                        }
                        Err(e) => {
                            out.problems.push(e);
                            chain_ready = false;
                        }
                    }
                    for (asset, n) in assets {
                        movement(&mut out, &location, &asset, n);
                    }
                    movement(&mut out, &location, "SOL", sol);
                    let total = out
                        .cash_totals
                        .get("USDC")
                        .map(|s| decimal(s))
                        .transpose()
                        .and_then(|old| add(old.unwrap_or(Decimal::ZERO), cash));
                    match total {
                        Ok(n) => {
                            out.cash_totals.insert("USDC".into(), amount(n));
                        }
                        Err(e) => out.problems.push(e),
                    }
                }
                Ok(None) => {
                    chain_ready = false;
                    out.remaining.push(format!(
                        "{location} 尚无原交易最终回执，不能用 Provider 回复代替到账"
                    ));
                }
                Err(e) => {
                    chain_ready = false;
                    out.problems.push(e);
                }
            }
        }
        for (i, row) in self
            .native_topups
            .iter()
            .enumerate()
            .filter(|(_, r)| r.terms.submission.is_some())
        {
            match row.observed_changes() {
                Ok(Some((cash_raw, sol_raw, fee_raw))) => {
                    if let Err(e) = row.actual_changes(self) {
                        out.problems.push(e);
                    }
                    let parsed = quantity(&cash_raw.to_string(), 6).and_then(|cash| {
                        Ok((
                            cash,
                            quantity(&sol_raw.to_string(), 9)?,
                            quantity(&fee_raw.to_string(), 9)?,
                        ))
                    });
                    let applied = parsed.and_then(|(cash, sol, fee)| {
                        let previous = out
                            .cash_totals
                            .get("USDC")
                            .map(|s| decimal(s))
                            .transpose()?
                            .unwrap_or(Decimal::ZERO);
                        let total = add(previous, cash)?;
                        let next_sol = add(chain_sol, sol)?;
                        let next_fee = add(chain_fee, fee)?;
                        out.cash_totals.insert("USDC".into(), amount(total));
                        chain_sol = next_sol;
                        chain_fee = next_fee;
                        let location = format!("Solana · SOL 补回 {}", i + 1);
                        movement(&mut out, &location, "USDC", cash);
                        movement(&mut out, &location, "SOL", sol);
                        Ok::<_, String>(())
                    });
                    if let Err(e) = applied {
                        out.problems.push(e);
                        chain_ready = false;
                    }
                }
                Ok(None) => {
                    chain_ready = false;
                    out.remaining
                        .push(format!("SOL 补回 {} 尚无最终回执，只核对原交易", i + 1));
                }
                Err(e) => {
                    chain_ready = false;
                    out.problems.push(e);
                }
            }
        }
        if chain_ready {
            out.chain_stock_shares = Some(amount(chain_stock));
            out.wallet_sol_change = Some(amount(chain_sol));
            out.network_fee_sol = Some(amount(chain_fee));
            if chain_sol < Decimal::ZERO {
                out.remaining
                    .push("实际净扣 SOL 尚未补回；网络费已包含在该扣款中，不重复减一次".into());
            }
        }
        if cex_ready && chain_ready {
            let net = decimal(out.cex_stock_shares.as_deref().unwrap())
                .and_then(|a| add(a, decimal(out.chain_stock_shares.as_deref().unwrap())?));
            match net {
                Ok(n) => {
                    out.net_stock_shares = Some(amount(n));
                    let cex = self.cex_order.as_ref().unwrap();
                    let chain = self
                        .chain_submission
                        .as_ref()
                        .unwrap()
                        .receipt
                        .as_ref()
                        .unwrap();
                    let repaired = self
                        .recoveries
                        .iter()
                        .rev()
                        .find_map(|r| r.submission.as_ref())
                        .and_then(|s| s.receipt.as_ref())
                        .is_some_and(|r| r.succeeded && stock_receipt_recoverable(r));
                    let full =
                        (cex.phase == StockCexOrderPhase::Filled && chain.succeeded) || repaired;
                    if !n.is_zero()
                        && (!full
                            || n < Decimal::ZERO
                            || self.inventory_orders.iter().any(|r| r.order.is_some()))
                    {
                        if out.problems.is_empty() && out.fee_budget_matched == Some(true) {
                            match recovery_target(self, n) {
                                Ok(t) => out.recovery_target = Some(t),
                                Err(e) => out.problems.push(e),
                            }
                        }
                        out.problems
                            .push("原两腿没有对齐，需按实际股票差额重新报价并确认补偿".into());
                    }
                    if out.cex_stock_shares.as_deref() != Some("0")
                        || out.chain_stock_shares.as_deref() != Some("0")
                    {
                        out.remaining.push(
                            "股票库存仍分布在不同发行方和场所，尚未完成补库，不能直接互充".into(),
                        );
                    }
                    out.status = StockAccountingStatus::LegsReconciled;
                }
                Err(e) => out.problems.push(e),
            }
        }
        if !out.problems.is_empty() {
            out.status = StockAccountingStatus::NeedsReview;
        }
        for (i, c) in self
            .conversions
            .iter()
            .enumerate()
            .filter(|(_, c)| c.order.is_some())
        {
            match c.native_cash_changes() {
                Ok(changes) => {
                    if let Err(e) = c.cash_changes() {
                        out.problems.push(format!("换汇 {}：{e}", i + 1));
                        out.status = StockAccountingStatus::NeedsReview;
                    }
                    for (asset, quantity) in changes {
                        let total = difference_total(&out.cash_totals, &asset, &quantity);
                        match total {
                            Ok(n) => {
                                movement(
                                    &mut out,
                                    &format!("Kraken · 换汇 {}", i + 1),
                                    &asset,
                                    decimal(&quantity).unwrap(),
                                );
                                out.cash_totals.insert(asset, amount(n));
                            }
                            Err(e) => {
                                out.problems.push(e);
                                out.status = StockAccountingStatus::NeedsReview;
                            }
                        }
                    }
                }
                Err(e) => {
                    out.remaining.push(format!("换汇 {}：{e}", i + 1));
                    if c.order
                        .as_ref()
                        .is_some_and(|r| r.evidence_conflict || r.receipt_complete())
                    {
                        out.problems.push(e);
                        out.status = StockAccountingStatus::NeedsReview;
                    } else if out.status != StockAccountingStatus::NeedsReview {
                        out.status = StockAccountingStatus::AwaitingReceipts;
                    }
                }
            }
        }
        if self
            .conversions
            .iter()
            .any(|c| c.native_cash_changes().is_ok())
        {
            out.remaining
                .retain(|s| !s.contains("之间尚无实际换汇回执"));
            if let Some(q) = out.cash_totals.get(&out.quote_asset) {
                out.remaining.push(format!(
                    "已核实收支仍保留 {q} {}，零头未按 1:1 计入 USDC 盈亏",
                    out.quote_asset
                ));
            }
        }
        out
    }
}
fn difference_total(
    cash: &BTreeMap<String, String>,
    asset: &str,
    quantity: &str,
) -> Result<Decimal, String> {
    add(
        cash.get(asset)
            .map(|n| decimal(n))
            .transpose()?
            .unwrap_or(Decimal::ZERO),
        decimal(quantity)?,
    )
}

struct CexChanges {
    cash: StockPeerCashSettlement,
    stock: Decimal,
    quote: Decimal,
    fee: Decimal,
    matched: bool,
}

fn cex_changes(p: &StockPeerPlan) -> Result<Option<CexChanges>, String> {
    let Some(r) = &p.cex_order else {
        return Ok(None);
    };
    if r.draft != p.terms.draft
        || r.evidence_conflict
        || p.terms.basis.account.quote_asset != r.draft.quote_asset
        || p.terms.basis.account.native_symbol != r.draft.request.selection.native_symbol
    {
        return Err("Kraken 回执身份变化或存在冲突".into());
    }
    r.validate_stored().map_err(|_| "Kraken 原始成交记录无效")?;
    if !r.receipt_complete() {
        return Ok(None);
    };
    let cash = r
        .cash_settlement()
        .ok_or("实际扣费币种尚不能按原生股票与计价币核账，不能假定费用为零")?;
    let fee = r.fills.iter().try_fold(Decimal::ZERO, |sum, f| {
        f.fees
            .as_ref()
            .ok_or_else(|| "费用缺失".to_owned())?
            .iter()
            .try_fold(sum, |sum, f| add(sum, decimal(&f.quantity)?))
    })?;
    let cost = decimal(r.cumulative_cost.as_deref().ok_or("成交金额缺失")?)?;
    let rate = decimal(
        p.terms
            .basis
            .account
            .stock_taker_pct
            .as_deref()
            .ok_or("原手续费预算缺失")?,
    )?;
    if rate < Decimal::ZERO || rate >= Decimal::from(100) {
        return Err("原股票手续费率无效".into());
    }
    let budget = cost
        .checked_mul(rate)
        .and_then(|n| n.checked_div(Decimal::from(100)))
        .ok_or("手续费预算溢出")?;
    Ok(Some(CexChanges {
        stock: decimal(&cash.equity_shares_change)?,
        quote: decimal(&cash.quote_change)?,
        cash,
        fee,
        matched: fee <= budget,
    }))
}

struct ChainChanges {
    stock: Decimal,
    cash: Decimal,
    sol: Decimal,
    fee: Decimal,
    assets: Vec<(String, Decimal)>,
}
fn chain_changes(
    p: &StockPeerPlan,
    c: &StockChainCost,
    submission: Option<&StockChainSubmission>,
) -> Result<Option<ChainChanges>, String> {
    let Some(s) = submission else {
        return Ok(None);
    };
    let Some(r) = &s.receipt else { return Ok(None) };
    let profile = identity::backpack_issuer(&p.terms.basis.security)?;
    if c.asset != p.request.asset
        || c.asset != profile.asset
        || c.mint.address != profile.solana_mint
        || c.mint.decimals != profile.decimals
        || c.wallet_address != p.request.wallet_address
        || decimal(&c.mint.ui_multiplier)? != decimal(&p.terms.basis.chain_cost.mint.ui_multiplier)?
        || !p.terms.basis.peer.share_unit_verified
        || p.terms.draft.request.selection != p.request.selection
        || p.terms.draft.request.direction != p.request.direction
        || p.terms.draft.request.asset != p.request.asset
    {
        return Err("原计划的证券身份、份额口径或钱包不一致".into());
    }
    let (input_mint, output_mint) = if c.direction == StockChainDirection::Buy {
        (comparison::SOLANA_USDC, c.mint.address.as_str())
    } else {
        (c.mint.address.as_str(), comparison::SOLANA_USDC)
    };
    if c.quote.input_mint != input_mint || c.quote.output_mint != output_mint {
        return Err("原股票交易的资产路径不一致".into());
    }
    if s.transaction_id.as_ref() != Some(&r.transaction_id)
        || r.slot < c.simulation_slot.unwrap_or(c.mint.slot)
        || !stock_receipt_recoverable(r)
        || r.asset_changes.len() > 256
    {
        return Err("链上最终回执不符合原交易或失败回滚结果，需先核对".into());
    }
    let fee = Decimal::from(
        r.network_fee_lamports
            .parse::<u64>()
            .map_err(|_| "网络费无效")?,
    ) / Decimal::from(1_000_000_000);
    let sol = quantity(&r.wallet_native_change_lamports, 9)?;
    if !r.succeeded
        && ((r.fee_payer == p.request.wallet_address && sol != -fee)
            || (r.fee_payer != p.request.wallet_address && !sol.is_zero()))
    {
        return Err("失败交易的钱包 SOL 扣款与付款方不一致".into());
    }
    let mut seen = BTreeSet::new();
    let mut assets = vec![];
    let mut stock = None;
    let mut cash = None;
    for a in &r.asset_changes {
        if !seen.insert(&a.mint) {
            return Err("链上回执重复记录同一资产".into());
        }
        let n = quantity(&a.raw_change, a.decimals)?;
        if a.mint == c.mint.address {
            if a.decimals != c.mint.decimals {
                return Err("股票回执精度与原合约不一致".into());
            }
            let m = decimal(&c.mint.ui_multiplier)?;
            if m <= Decimal::ZERO {
                return Err("原股票份额换算无效".into());
            }
            let shares = n
                .checked_mul(m)
                .filter(|v| v.checked_div(m) == Some(n))
                .ok_or("股票份额换算溢出或精度丢失")?;
            stock = Some(shares);
            assets.push((a.mint.clone(), n));
        } else if a.mint == comparison::SOLANA_USDC {
            if a.decimals != 6 {
                return Err("USDC 回执精度不是 6".into());
            }
            cash = Some(n);
            assets.push(("USDC".into(), n));
        } else if !n.is_zero() {
            return Err("原交易出现其他资产收支，需独立核账".into());
        }
    }
    let stock = stock.ok_or("原股票资产变化缺失，不能按零处理")?;
    let cash = cash.ok_or("原 USDC 资产变化缺失，不能按零处理")?;
    if r.succeeded {
        let input = c
            .quote
            .input_raw
            .parse::<u64>()
            .map_err(|_| "原输入数量无效")?;
        let minimum = c
            .quote
            .minimum_output_raw
            .parse::<u64>()
            .map_err(|_| "原最低到账无效")?;
        let raw = |mint: &str| {
            r.asset_changes
                .iter()
                .find(|a| a.mint == mint)
                .and_then(|a| a.raw_change.parse::<i128>().ok())
        };
        if raw(&c.quote.input_mint) != Some(-i128::from(input))
            || raw(&c.quote.output_mint).is_none_or(|n| n < i128::from(minimum))
            || c.wallet_debit_lamports
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .is_none_or(|n| sol < -Decimal::from(n) / Decimal::from(1_000_000_000))
        {
            return Err("实际链上输入、最低到账或 SOL 支出不符合原计划".into());
        }
    }
    Ok(Some(ChainChanges {
        stock,
        cash,
        sol,
        fee,
        assets,
    }))
}

fn recovery_target(p: &StockPeerPlan, shares: Decimal) -> Result<StockRecoveryTarget, String> {
    let m = &p.terms.basis.chain_cost.mint;
    let scale = Decimal::from(
        10u64
            .checked_pow(u32::from(m.decimals))
            .ok_or("股票精度超限")?,
    );
    let multiplier = decimal(&m.ui_multiplier)?;
    if multiplier <= Decimal::ZERO {
        return Err("原股票份额换算无效".into());
    }
    let raw = shares
        .abs()
        .checked_mul(scale)
        .filter(|n| n.checked_div(scale) == Some(shares.abs()))
        .and_then(|n| n.checked_div(multiplier))
        .ok_or("补偿目标计算溢出")?;
    let raw = if shares < Decimal::ZERO {
        raw.ceil()
    } else {
        raw.floor()
    };
    if raw <= Decimal::ZERO || amount(raw).parse::<u64>().is_err() {
        return Err("股票差额小于可执行单位或超出范围，需核对余量".into());
    }
    let actual = raw
        .checked_mul(multiplier)
        .and_then(|n| n.checked_div(scale))
        .ok_or("补偿份额计算溢出")?;
    if (shares < Decimal::ZERO && actual < shares.abs())
        || (shares > Decimal::ZERO && actual > shares)
    {
        return Err("补偿份额精度不足，需核对原始数量".into());
    }
    Ok(StockRecoveryTarget {
        direction: if shares < Decimal::ZERO {
            StockChainDirection::Buy
        } else {
            StockChainDirection::Sell
        },
        stock_raw: amount(raw),
        stock_shares: amount(shares),
    })
}
