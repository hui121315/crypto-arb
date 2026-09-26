use super::*;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerPreflightRequest {
    pub asset: String,
    pub selection: StockPeerSelection,
    pub wallet_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerAccount {
    pub venue: String,
    pub native_symbol: String,
    pub stock_asset: String,
    pub quote_asset: String,
    pub stock_available: Option<String>,
    pub quote_available: Option<String>,
    pub usdc_available: Option<String>,
    pub stock_taker_pct: Option<String>,
    pub fx_taker_pct: Option<String>,
    pub observed_at_ms: i64,
    pub sources: Vec<String>,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerPreflight {
    pub asset: String,
    pub selection: StockPeerSelection,
    pub checked_at_ms: i64,
    pub account: Option<StockPeerAccount>,
    pub wallet: Option<StockWalletEvidence>,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerReadiness {
    pub chain_buy: bool,
    pub gross_usdc: Option<String>,
    pub trading_cost_usdc: Option<String>,
    pub native_cost_usdc: Option<String>,
    pub after_known_costs_usdc: Option<String>,
    pub inventory: Vec<StockInventoryRequirement>,
    pub blockers: Vec<String>,
}

fn number(s: &str) -> Option<Decimal> {
    s.parse().ok()
}
fn nonnegative(s: &str) -> Option<Decimal> {
    number(s).filter(|v| *v >= Decimal::ZERO)
}
fn amount(v: Decimal) -> String {
    v.normalize().to_string()
}
fn rate(s: Option<&String>) -> Option<Decimal> {
    nonnegative(s?)
        .filter(|v| *v < Decimal::from(100))?
        .checked_div(Decimal::from(100))
}
fn required(
    location: &str,
    asset: &str,
    quantity: Option<Decimal>,
    available: Option<Decimal>,
) -> StockInventoryRequirement {
    StockInventoryRequirement {
        location: location.into(),
        asset: asset.into(),
        required: quantity.map(amount),
        available: available.map(amount),
        sufficient: quantity.zip(available).map(|(q, a)| a >= q),
    }
}

pub fn evaluate_peer_preflight(s: &StockMarketSnapshot, now: i64) -> Vec<StockPeerReadiness> {
    let Some(p) = &s.peer else { return vec![] };
    let preflight = s.peer_preflight.as_ref().filter(|r| {
        r.selection == p.selection
            && s.security
                .as_ref()
                .is_some_and(|stock| stock.asset == r.asset)
            && now >= r.checked_at_ms
            && now - r.checked_at_ms <= 15_000
    });
    let account = preflight.and_then(|r| r.account.as_ref()).filter(|a| {
        a.venue == p.selection.venue
            && a.native_symbol == p.selection.native_symbol
            && now >= a.observed_at_ms
            && now - a.observed_at_ms <= 15_000
            && p.instrument
                .as_ref()
                .is_some_and(|i| i.quote_asset.as_deref() == Some(a.quote_asset.as_str()))
            && p.selection
                .native_symbol
                .split_once('/')
                .is_some_and(|(base, _)| base.eq_ignore_ascii_case(&a.stock_asset))
    });
    let wallet = preflight.and_then(|r| r.wallet.as_ref()).filter(|w| {
        now >= w.checked_at_ms
            && now - w.checked_at_ms <= 15_000
            && s.comparison
                .as_ref()
                .is_some_and(|c| c.mint.address == w.mint)
    });
    evaluate_peer(s, now).into_iter().map(|e| {
        let mut row = StockPeerReadiness {chain_buy:e.chain_buy,gross_usdc:e.gross_usdc.clone(),trading_cost_usdc:None,
            native_cost_usdc:None,after_known_costs_usdc:None,inventory:vec![],blockers:vec!["当前仅为费用与库存预检，执行能力由双边计划另行核验；不同发行方不可直接互转，不是锁定利润".into()]};
        if e.gross_usdc.is_none() { row.blockers.extend(e.blockers.into_iter().skip(1)); }
        if account.is_none() { row.blockers.push("所选交易所账户未检查或已过期".into()); }
        if let Some(r)=preflight { row.blockers.extend(r.problems.clone()); }
        if let Some(a)=account {row.blockers.extend(a.problems.clone());}
        let fee = account.and_then(|a| rate(a.stock_taker_pct.as_ref()));
        let fx = if p.instrument.as_ref().and_then(|i|i.quote_asset.as_deref())==Some("USDC") {Some(Decimal::ZERO)}
            else {account.and_then(|a|rate(a.fx_taker_pct.as_ref()))};
        let notional = e.cex_notional_usdc.as_deref().and_then(nonnegative);
        // Quote-currency fee budget for both orders. Buy USDC fees consume the
        // quote proceeds; sell USDC fees reduce quote funds available to buy stock.
        let adjusted = notional.zip(fee).zip(fx).and_then(|((n,f),x)| {
            if e.chain_buy {n.checked_mul(Decimal::ONE-f)?.checked_div(Decimal::ONE+x)}
            else {n.checked_mul(Decimal::ONE+f)?.checked_div(Decimal::ONE-x)}
        });
        let adjusted=adjusted.filter(|a| e.chain_buy || p.quote_conversion.as_ref().is_none_or(|q|
            q.bid_quantity.as_deref().and_then(nonnegative).is_some_and(|quantity|quantity>=*a)));
        let trade_cost=notional.zip(adjusted).map(|(n,a)| if e.chain_buy {n-a}else{a-n});
        row.trading_cost_usdc=trade_cost.map(amount);
        if trade_cost.is_none() {row.blockers.push("股票/换汇费率、有效报价或费用后所需盘口数量未核齐，未计算费用后差额".into());}
        row.blockers.push("费用按计价币扣费预算；实际扣费币种、换汇步长和成交取整仍需执行器核验".into());
        let Some(c)=s.comparison.as_ref() else {return row};
        let cost=wallet.and_then(|w| s.chain_costs.iter().find(|cost| (cost.direction==StockChainDirection::Buy)==e.chain_buy
            && cost.current(s,&w.owner,now) && cost.simulation_passed));
        let native_cost=cost.and_then(|c|c.complete_native_usdc_budget(now)).as_deref().and_then(nonnegative);
        row.native_cost_usdc=native_cost.map(amount);
        row.after_known_costs_usdc=e.gross_usdc.as_deref().and_then(number).zip(trade_cost).zip(native_cost)
            .and_then(|((g,t),n)|g.checked_sub(t)?.checked_sub(n)).map(amount);
        if native_cost.is_none(){row.blockers.push("链上交易模拟和 SOL 补回成本未核实，完整差额保持未知".into());}
        let chain_raw = if e.chain_buy {Some(c.buy.input_raw.as_str())} else {c.sell.as_ref().map(|q|q.input_raw.as_str())};
        let chain_decimals=if e.chain_buy {6}else{c.mint.decimals};
        let units=Decimal::from(10_u64.checked_pow(u32::from(chain_decimals)).unwrap_or(0));
        let chain_qty=chain_raw.and_then(nonnegative).and_then(|n|n.checked_div(units));
        let chain_available=wallet.and_then(|w|if e.chain_buy {w.usdc_raw.as_deref()}else{w.stock_raw.as_deref()})
            .and_then(nonnegative).and_then(|n|n.checked_div(units));
        let native_buy_budget=e.shares.as_deref().and_then(nonnegative)
            .zip(p.quote.as_ref().and_then(|q|nonnegative(&q.ask))).zip(fee)
            .and_then(|((qty,price),fee)|qty.checked_mul(price)?.checked_mul(Decimal::ONE+fee));
        row.inventory.push(required(&p.selection.venue,if e.chain_buy {account.map(|a|a.stock_asset.as_str()).unwrap_or("股票份额")}else{account.map(|a|a.quote_asset.as_str()).unwrap_or("原生计价币")},
            if e.chain_buy {e.shares.as_deref().and_then(nonnegative)}else{native_buy_budget},
            account.and_then(|a|if e.chain_buy {a.stock_available.as_deref()}else{a.quote_available.as_deref()}).and_then(nonnegative)));
        row.inventory.push(required("Solana",if e.chain_buy {"USDC"}else{&c.mint.address},
            if e.chain_buy {chain_qty.zip(native_cost).and_then(|(q,c)|q.checked_add(c))}else{chain_qty},chain_available));
        row.inventory.push(required("Solana","SOL",cost.and_then(|c| c.total_native_required_lamports(now)).map(|n|Decimal::from(n)/Decimal::from(1_000_000_000)),
            wallet.and_then(|w| w.sol_lamports.as_deref()).and_then(nonnegative).map(|n|n/Decimal::from(1_000_000_000))));
        if !e.chain_buy {row.inventory.push(required("Solana","USDC / SOL 补回",native_cost,
            wallet.and_then(|w|w.usdc_raw.as_deref()).and_then(nonnegative).map(|n|n/Decimal::from(1_000_000))));}
        if row.after_known_costs_usdc.as_deref().and_then(number).is_some_and(|n|n<=Decimal::ZERO) {
            row.blockers.push("已知费用后已无正差额".into());
        }
        if row.inventory.iter().any(|i| i.sufficient!=Some(true)) {row.blockers.push("双边库存或 Gas 不足/未知，不能自动借款补足".into());}
        if fx.is_some_and(|f|f!=Decimal::ZERO) || p.instrument.as_ref().and_then(|i|i.quote_asset.as_deref())!=Some("USDC") {
            row.blockers.push("交易所按原生计价币预留；USDC 仅为含换汇费用的估值。换汇尚未执行，双边交易并非原子操作".into());
        }
        row
    }).collect()
}

#[cfg(test)]
mod tests;
