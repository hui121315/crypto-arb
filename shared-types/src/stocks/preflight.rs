use super::*;
use rust_decimal::{Decimal, RoundingStrategy};
use std::{collections::BTreeMap, str::FromStr};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPreflightRequest {
    pub asset: String,
    pub wallet_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plan: Option<StockInventorySource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockAccountBalance {
    pub available: String,
    pub locked: String,
    pub staked: String,
    pub observed_at_ms: i64,
    pub source_at_us: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockAccountEvidence {
    pub fingerprint: String,
    pub spot_maker_fee_bps: String,
    pub spot_taker_fee_bps: String,
    pub liquidating: bool,
    pub fees_at_ms: i64,
    pub balances_at_ms: i64,
    pub balances: BTreeMap<String, StockAccountBalance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockWalletEvidence {
    pub owner: String,
    pub mint: String,
    pub stock_raw: Option<String>,
    pub usdc_raw: Option<String>,
    pub sol_lamports: Option<String>,
    pub checked_at_ms: i64,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockInventoryRequirement {
    pub location: String,
    pub asset: String,
    pub required: Option<String>,
    pub available: Option<String>,
    pub sufficient: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPreflightDirection {
    pub direction: String,
    pub gross_usdc: Option<String>,
    pub cex_fee_usdc: Option<String>,
    #[serde(default)]
    pub native_fee_usdc: Option<String>,
    pub after_known_costs_usdc: Option<String>,
    pub fee_basis: String,
    pub inventory: Vec<StockInventoryRequirement>,
    pub transfer_problem: Option<String>,
    pub blockers: Vec<String>,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPriceBasis {
    pub asset: Option<String>,
    pub mint_units: Option<(String, u8, String)>,
    pub buy_terms: Option<(String, String, String, String)>,
    pub sell_terms: Option<(String, String, String, String)>,
    pub buy_requested_at_ms: Option<i64>,
    pub sell_requested_at_ms: Option<i64>,
    pub books: Vec<(String, u64)>,
    pub rfqs: Vec<(String, String, i64)>,
}

impl StockPriceBasis {
    pub fn from_snapshot(s: &StockMarketSnapshot) -> Self {
        Self {
            asset: s.security.as_ref().map(|s| s.asset.clone()),
            mint_units: s.comparison.as_ref().map(|c| {
                (
                    c.mint.address.clone(),
                    c.mint.decimals,
                    c.mint.ui_multiplier.clone(),
                )
            }),
            buy_terms: s.comparison.as_ref().map(|c| terms(&c.buy)),
            sell_terms: s
                .comparison
                .as_ref()
                .and_then(|c| c.sell.as_ref())
                .map(terms),
            buy_requested_at_ms: s.comparison.as_ref().map(|c| c.buy.requested_at_ms),
            sell_requested_at_ms: s
                .comparison
                .as_ref()
                .and_then(|c| c.sell.as_ref())
                .map(|q| q.requested_at_ms),
            books: s
                .books
                .iter()
                .map(|b| (b.symbol.clone(), b.update_id))
                .collect(),
            rfqs: s
                .rfqs
                .iter()
                .filter(|r| {
                    r.phase == StockRfqPhase::Candidate && !r.needs_recheck && !r.cancel_requested
                })
                .filter_map(|r| {
                    r.candidate.as_ref().map(|q| {
                        (
                            r.request.request_id.clone(),
                            q.quote_id.clone(),
                            q.source_at_us,
                        )
                    })
                })
                .collect(),
        }
    }
}

fn terms(q: &StockDexQuote) -> (String, String, String, String) {
    (
        q.input_mint.clone(),
        q.output_mint.clone(),
        q.input_raw.clone(),
        q.minimum_output_raw.clone(),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPreflight {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plan: Option<StockInventorySource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding: Vec<StockFundingDirection>,
    pub asset: String,
    pub wallet_address: Option<String>,
    pub checked_at_ms: i64,
    pub valid_until_ms: i64,
    pub price_basis: StockPriceBasis,
    pub spot_taker_fee_pct: Option<String>,
    pub account_at_ms: Option<i64>,
    pub wallet_at_ms: Option<i64>,
    pub directions: Vec<StockPreflightDirection>,
    pub problems: Vec<String>,
}

impl StockPreflight {
    pub fn current(&self, snapshot: &StockMarketSnapshot, now: i64) -> bool {
        self.source_plan.is_none()
            && now >= self.checked_at_ms
            && now < self.valid_until_ms
            && self.price_basis == StockPriceBasis::from_snapshot(snapshot)
            && snapshot.trading_route.as_ref().is_some_and(|r| {
                r.valid_until_ms > now
                    && match r.kind {
                        StockRouteKind::OrderBook => {
                            snapshot.connected && snapshot.problem.is_none()
                        }
                        StockRouteKind::Rfq => {
                            snapshot.rfq_connected && snapshot.rfq_problem.is_none()
                        }
                        _ => false,
                    }
            })
            && self.directions.len() == 2
            && comparison::evaluate(snapshot, now)
                .iter()
                .zip(&self.directions)
                .all(|(price, report)| {
                    let cost = self.wallet_address.as_deref().and_then(|wallet| {
                        snapshot.chain_costs.iter().find(|c| {
                            c.direction.label() == report.direction
                                && c.current(snapshot, wallet, now)
                        })
                    });
                    price.direction == report.direction
                        && price.gross_usdc == report.gross_usdc
                        && report.native_fee_usdc == cost.and_then(|c| c.native_usdc_budget(now))
                        && report.inventory.get(2).and_then(|i| i.required.clone())
                            == cost.and_then(|c| required_sol(c, now)).map(amount)
                })
    }
}

fn decimal(s: &str) -> Option<Decimal> {
    Decimal::from_str(s).ok()
}
fn amount(d: Decimal) -> String {
    d.normalize().to_string()
}
fn required_sol(cost: &StockChainCost, now: i64) -> Option<Decimal> {
    if !cost.simulation_passed {
        return None;
    }
    let n = if cost
        .native_valuation
        .as_ref()
        .is_some_and(|v| v.replenishment.is_some())
    {
        cost.total_native_required_lamports(now)?
    } else {
        cost.wallet_required_lamports
            .as_deref()?
            .parse::<u64>()
            .ok()?
    };
    Decimal::from(n).checked_div(Decimal::from(1_000_000_000))
}
fn requirement(
    location: &str,
    asset: &str,
    required: Option<Decimal>,
    available: Option<Decimal>,
) -> StockInventoryRequirement {
    StockInventoryRequirement {
        location: location.into(),
        asset: asset.into(),
        required: required.map(amount),
        available: available.map(amount),
        sufficient: required.zip(available).map(|(r, a)| a >= r),
    }
}

pub fn evaluate_preflight(
    snapshot: &StockMarketSnapshot,
    account: Option<&StockAccountEvidence>,
    wallet: Option<&StockWalletEvidence>,
    now: i64,
) -> Vec<StockPreflightDirection> {
    let account = account.filter(|a| {
        now >= a.fees_at_ms
            && now - a.fees_at_ms <= 300_000
            && now >= a.balances_at_ms
            && now - a.balances_at_ms <= 30_000
    });
    let wallet = wallet.filter(|w| now >= w.checked_at_ms && now - w.checked_at_ms <= 30_000);
    let rfq = snapshot
        .trading_route
        .as_ref()
        .is_some_and(|r| r.kind == StockRouteKind::Rfq);
    comparison::evaluate(snapshot,now).into_iter().enumerate().map(|(index, estimate)| {
        let chain_buy=index==0;
        let mut row=StockPreflightDirection { direction:estimate.direction.into(),gross_usdc:estimate.gross_usdc.clone(),cex_fee_usdc:None,native_fee_usdc:None,after_known_costs_usdc:None,
            fee_basis:if rfq {"RFQ taker 价已含报价费，不再叠加现货费"} else {"账户 taker 费率 · USDC 扣费预算，实扣以成交回执为准"}.into(),inventory:vec![],transfer_problem:None,blockers:vec![],executable:false };
        row.blockers.extend(estimate.blockers.into_iter().filter(|b|!b.starts_with("尚缺账户费率")));
        let notional=estimate.cex_notional_usdc.as_deref().and_then(decimal);
        let fee=if rfq {notional.map(|_|Decimal::ZERO)} else {account.and_then(|a|StockCexFeeBudget::calculate(estimate.cex_notional_usdc.as_deref()?,if chain_buy {StockRfqSide::Ask}else{StockRfqSide::Bid},StockCexFeeBasis::OrderBookQuote{taker_bps:a.spot_taker_fee_bps.clone(),observed_at_ms:a.fees_at_ms})).and_then(|b|decimal(&b.additional_fee))};
        row.cex_fee_usdc=fee.map(amount);
        row.after_known_costs_usdc=estimate.gross_usdc.as_deref().and_then(decimal).zip(fee).and_then(|(g,f)|g.checked_sub(f)).map(|d| amount(d.round_dp_with_strategy(6,RoundingStrategy::ToNegativeInfinity)));
        if account.is_none() {row.blockers.push("Backpack 账户费率/余额未读取或已陈旧".into());}
        if account.is_some_and(|a|a.liquidating) {row.blockers.push("Backpack 账户正在清算".into());}
        if fee.is_none() {row.blockers.push("交易所手续费预算未确认，不能把未知费用当作零".into());}
        let Some(c)=snapshot.comparison.as_ref() else{return row};
        let cex_asset=if chain_buy{c.asset.as_str()}else{"USDC"};
        let cex_available=account.and_then(|a|a.balances.get(cex_asset)).filter(|b| now>=b.observed_at_ms && now-b.observed_at_ms<=30_000).and_then(|b|decimal(&b.available));
        let cex_required=if chain_buy {estimate.shares.as_deref().and_then(decimal)}else{notional.zip(fee).and_then(|(n,f)|n.checked_add(f))};
        row.inventory.push(requirement("Backpack",cex_asset,cex_required,cex_available));
        let wallet=wallet.filter(|w|w.mint==c.mint.address);
        let chain_required=if chain_buy {decimal(&c.buy.input_raw).and_then(|n|n.checked_div(Decimal::from(1_000_000)))}else{c.sell.as_ref().and_then(|q|comparison::shares(&q.input_raw,&c.mint))};
        let chain_available=if chain_buy {wallet.and_then(|w|w.usdc_raw.as_deref()).and_then(decimal).and_then(|n|n.checked_div(Decimal::from(1_000_000)))}else{
            wallet.and_then(|w|w.stock_raw.as_deref()).and_then(decimal).and_then(|n| n.checked_div(Decimal::from(10_u64.checked_pow(u32::from(c.mint.decimals))?))?.checked_mul(comparison::positive(&c.mint.ui_multiplier)?))};
        row.inventory.push(requirement("Solana",if chain_buy{"USDC"}else{&c.asset},chain_required,chain_available));
        let direction=if chain_buy {StockChainDirection::Buy}else{StockChainDirection::Sell};
        let chain_cost=wallet.and_then(|w|snapshot.chain_costs.iter().find(|c|c.direction==direction && c.current(snapshot,&w.owner,now)));
        row.native_fee_usdc=chain_cost.and_then(|c|c.native_usdc_budget(now));
        if let Some(native)=row.native_fee_usdc.as_deref().and_then(decimal) {
            row.after_known_costs_usdc=estimate.gross_usdc.as_deref().and_then(decimal).zip(fee).and_then(|(g,f)|g.checked_sub(f)?.checked_sub(native)).map(|d|amount(d.round_dp_with_strategy(6,RoundingStrategy::ToNegativeInfinity)));
        }
        let gas_budget=chain_cost.and_then(|c|required_sol(c,now));
        row.inventory.push(requirement("Solana","SOL / 保守周转余额",gas_budget,wallet.and_then(|w|w.sol_lamports.as_deref()).and_then(decimal).and_then(|n|n.checked_div(Decimal::from(1_000_000_000)))));
        let complete_native=chain_cost.and_then(|c|c.complete_native_usdc_budget(now));
        if let Some(budget)=complete_native.as_deref().and_then(decimal).filter(|n|*n>Decimal::ZERO) {
            let usdc_available=wallet.and_then(|w|w.usdc_raw.as_deref()).and_then(decimal).and_then(|n|n.checked_div(Decimal::from(1_000_000)));
            if chain_buy {
                row.inventory[1]=requirement("Solana","USDC",chain_required.and_then(|n|n.checked_add(budget)),usdc_available);
            } else {
                row.inventory.push(requirement("Solana","USDC / SOL 补仓",Some(budget),usdc_available));
                if row.inventory[3].sufficient != Some(true) {row.blockers.push("链上 USDC 不足或未知，不能预支卖币到账来补 SOL".into());}
            }
        }
        for i in &row.inventory[..2] {match i.sufficient {Some(false)=>row.blockers.push(format!("{} {} 可用 {}，需要 {}",i.location,i.asset,i.available.as_deref().unwrap_or("未知"),i.required.as_deref().unwrap_or("未知"))),None=>row.blockers.push(format!("{} {} 库存或所需数量未确认",i.location,i.asset)),_=>{}}}
        if let Some(cost)=chain_cost {
            row.blockers.extend(cost.problems.clone());
            if !cost.simulation_passed {row.blockers.push("链上交易费用模拟未通过".into());}
            match row.inventory[2].sufficient {
                Some(false)=>row.blockers.push("SOL 可用余额低于本次保守周转余额（含钱包免租保留额）".into()),
                None=>row.blockers.push("SOL 周转余额缺少完整指令证据，不使用 Provider 估算代替".into()),
                _=>{}
            }
            if complete_native.is_none() {
                row.blockers.push(if row.native_fee_usdc.is_some() {"已计入模拟净扣 SOL 的补回报价；补仓交易自身费用仍待核实，不是可执行净利润"}else{"SOL 净扣尚缺保守 USDC 置换成本，不是净利润"}.into());
            }
        }else{row.blockers.push("Gas、账户创建租金及其 USDC 估值尚未取得，不是净利润".into());}
        row.blockers.push("资金尚未预留，链上签名与两腿执行计划尚未构建".into());
        if row.after_known_costs_usdc.as_deref().and_then(decimal).is_some_and(|n|n<=Decimal::ZERO) {row.blockers.push("已知成本后已无正差额".into());}
        let token=snapshot.tokens.iter().find(|t|t.blockchain.eq_ignore_ascii_case("solana") && t.contract_address.as_deref()==Some(&c.mint.address));
        row.transfer_problem=if snapshot.token_metadata_at_ms.is_none_or(|t|now<t || now-t>30_000) {Some("充提状态需重新读取".into())} else {
            token.and_then(|t|if t.deposit_enabled!=Some(true) || t.withdraw_enabled!=Some(true) {Some("股票充值或提现未开放；双边预置库存仅能消耗现有余额，不能假定可转币循环".into())}else{None}).or_else(||token.is_none().then(||"缺少同链同合约充提通道".into()))
        };
        row
    }).collect()
}

#[cfg(test)]
mod tests;
