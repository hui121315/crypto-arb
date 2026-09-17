use crate::state::AppState;
use shared_types::{
    is_hyperliquid_builder_venue, venue_names_equal, ExecutionGuard, ExecutionMode, FeeProduct,
    OrderCompilePlan, OrderIntent, StrategyKind, VenueBalanceInfo, VenueId,
};

#[derive(Debug, Clone, PartialEq)]
struct InventoryRequirement {
    venue: String,
    currency: String,
    amount: f64,
    purpose: &'static str,
}

pub(super) async fn guard(
    state: &AppState,
    mode: ExecutionMode,
    strategy: Option<StrategyKind>,
    plans: [&OrderCompilePlan; 2],
    intents: [&OrderIntent; 2],
) -> Option<ExecutionGuard> {
    if strategy != Some(StrategyKind::SpotCross) {
        return None;
    }
    if !matches!(mode, ExecutionMode::Live | ExecutionMode::Testnet) {
        return Some(inventory_guard(true, "模拟模式不读取真实现货库存"));
    }
    let requirements = match inventory_requirements(plans, intents) {
        Ok(requirements) => requirements,
        Err(detail) => return Some(inventory_guard(false, &detail)),
    };
    let venues = requirements
        .iter()
        .map(|requirement| requirement.venue.clone())
        .collect::<Vec<_>>();
    let balances = match state.trading_service().list_scoped_balances(&venues).await {
        Ok(balances) => balances,
        Err(error) => {
            return Some(inventory_guard(
                false,
                &format!(
                    "现货库存读取失败: {error}；需为买卖两端配置现货账户 API Key 与 balance_read 权限"
                ),
            ));
        }
    };
    let missing = unmet_requirements(&requirements, &balances);
    if missing.is_empty() {
        Some(inventory_guard(true, "双边现货预置资产充足"))
    } else {
        Some(inventory_guard(false, &missing.join("；")))
    }
}

fn inventory_requirements(
    plans: [&OrderCompilePlan; 2],
    intents: [&OrderIntent; 2],
) -> Result<Vec<InventoryRequirement>, String> {
    if plans.iter().any(|plan| plan.product != FeeProduct::Spot) {
        return Err("现货跨所双腿必须绑定 Spot 产品规格".to_owned());
    }
    let buy_spec = plans[0]
        .instrument_spec
        .as_ref()
        .ok_or_else(|| instrument_requirement(&plans[0].exchange, &plans[0].symbol))?;
    let sell_spec = plans[1]
        .instrument_spec
        .as_ref()
        .ok_or_else(|| instrument_requirement(&plans[1].exchange, &plans[1].symbol))?;
    let buy_quote = non_empty_upper(buy_spec.quote_asset.as_deref())
        .ok_or_else(|| instrument_requirement(&plans[0].exchange, &plans[0].symbol))?;
    let sell_base = base_asset(sell_spec)
        .ok_or_else(|| instrument_requirement(&plans[1].exchange, &plans[1].symbol))?;
    let buy_amount = intents[0].quantity * intents[0].price.unwrap_or(0.0);
    let sell_amount = intents[1].quantity;
    if !positive_finite(buy_amount) || !positive_finite(sell_amount) {
        return Err("现货库存预检缺少有效双腿数量或价格".to_owned());
    }
    Ok(vec![
        InventoryRequirement {
            venue: plans[0].exchange.clone(),
            currency: buy_quote,
            amount: buy_amount,
            purpose: "买入端计价币",
        },
        InventoryRequirement {
            venue: plans[1].exchange.clone(),
            currency: sell_base,
            amount: sell_amount,
            purpose: "卖出端基础币库存",
        },
    ])
}

fn base_asset(spec: &shared_types::InstrumentSpec) -> Option<String> {
    let symbol = spec.canonical_symbol.trim().to_ascii_uppercase();
    let quote = non_empty_upper(spec.quote_asset.as_deref())?;
    for delimiter in ['/', '-', '_'] {
        if let Some((base, observed_quote)) = symbol.split_once(delimiter) {
            return (observed_quote == quote && !base.is_empty()).then(|| base.to_owned());
        }
    }
    symbol
        .strip_suffix(&quote)
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| (!symbol.is_empty() && symbol != quote).then_some(symbol))
}

fn unmet_requirements(
    requirements: &[InventoryRequirement],
    balances: &[VenueBalanceInfo],
) -> Vec<String> {
    requirements
        .iter()
        .filter_map(|requirement| {
            let available = balances
                .iter()
                .filter(|row| {
                    venue_balance_matches(&row.venue, &requirement.venue)
                        && row.currency.eq_ignore_ascii_case(&requirement.currency)
                        && row.available.is_finite()
                })
                .map(|row| row.available.max(0.0))
                .reduce(f64::max)
                .unwrap_or(0.0);
            (available + f64::EPSILON < requirement.amount).then(|| {
                format!(
                    "需配置 {} {} 可用余额 >= {:.8}（{}，当前 {:.8}）",
                    requirement.venue,
                    requirement.currency,
                    requirement.amount,
                    requirement.purpose,
                    available
                )
            })
        })
        .collect()
}

fn venue_balance_matches(left: &str, right: &str) -> bool {
    venue_names_equal(left, right)
        || (!is_hyperliquid_builder_venue(left)
            && !is_hyperliquid_builder_venue(right)
            && VenueId::from_exchange_name(left)
                .zip(VenueId::from_exchange_name(right))
                .is_some_and(|(left, right)| left == right))
}

fn non_empty_upper(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase)
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn instrument_requirement(venue: &str, symbol: &str) -> String {
    format!("需配置 {venue} {symbol} 官方 Spot instrument registry 与可执行规格")
}

fn inventory_guard(passed: bool, detail: &str) -> ExecutionGuard {
    ExecutionGuard {
        key: "spot_inventory".to_owned(),
        label: "现货预置库存".to_owned(),
        passed,
        detail: detail.to_owned(),
        preflight_outcome: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_assets_name_the_exact_configuration() {
        let requirements = vec![
            InventoryRequirement {
                venue: "binance".into(),
                currency: "USDT".into(),
                amount: 100.0,
                purpose: "买入端计价币",
            },
            InventoryRequirement {
                venue: "okx".into(),
                currency: "BTC".into(),
                amount: 0.01,
                purpose: "卖出端基础币库存",
            },
        ];
        let missing = unmet_requirements(&requirements, &[]);

        assert_eq!(missing.len(), 2);
        assert!(missing[0].contains("binance USDT"));
        assert!(missing[1].contains("okx BTC"));
    }

    #[test]
    fn sufficient_assets_pass_case_insensitive_venue_and_currency() {
        let requirements = vec![InventoryRequirement {
            venue: "OKX-LIVE".into(),
            currency: "USDT".into(),
            amount: 100.0,
            purpose: "买入端计价币",
        }];
        let balances = vec![VenueBalanceInfo {
            venue: "okx".into(),
            currency: "usdt".into(),
            total: 120.0,
            available: 110.0,
            frozen: 10.0,
            unrealized_pnl: 0.0,
        }];

        assert!(unmet_requirements(&requirements, &balances).is_empty());
    }
}
