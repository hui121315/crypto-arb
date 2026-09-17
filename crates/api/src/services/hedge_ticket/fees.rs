use super::*;

pub(super) struct FeeLookup {
    pub(super) snapshot: Option<TradeFeeSnapshot>,
    pub(super) blockers: Vec<String>,
}

pub(super) fn fee_lookup(
    state: &AppState,
    opp: &ArbitrageOpportunityDto,
    role: HedgeLegRole,
    use_maker_fee: bool,
    now_ms: i64,
) -> FeeLookup {
    let risk = state.trading_service().risk_config();
    let spec = LegSpec::from_opp(opp, role);
    let product = fee_product_for(opp, role);
    if product == FeeProduct::Unknown {
        return FeeLookup {
            snapshot: None,
            blockers: fee_blockers(&risk, &spec, "交易产品费率类型未验证"),
        };
    }
    let snapshot = resolve_fee_snapshot(
        state
            .trade_fee_cache()
            .fresh(&spec.exchange, &spec.symbol, product, None, now_ms),
        || {
            crate::services::fees::standard_fee_snapshot(
                &spec.exchange,
                &spec.symbol,
                product,
                use_maker_fee,
                now_ms,
            )
        },
        now_ms,
    );
    let blockers = if snapshot.is_some() {
        Vec::new()
    } else {
        fee_blockers(&risk, &spec, "缺少已验证标准费率")
    };
    FeeLookup { snapshot, blockers }
}

pub(super) fn resolve_fee_snapshot(
    cache_hit: Option<TradeFeeSnapshot>,
    fallback: impl FnOnce() -> Option<TradeFeeSnapshot>,
    now_ms: i64,
) -> Option<TradeFeeSnapshot> {
    cache_hit
        .or_else(fallback)
        .filter(|snapshot| snapshot.is_fresh_verified(now_ms))
}

pub(super) fn fee_blockers(
    risk: &trading::RiskConfig,
    spec: &LegSpec,
    reason: &str,
) -> Vec<String> {
    if !risk.live_trading_enabled {
        return Vec::new();
    }
    vec![format!(
        "{} {} {reason}，实盘下单前需有 maker/taker/open/close fee",
        spec.exchange, spec.symbol
    )]
}

pub(super) fn use_maker_fee(params: &HedgeExecutionParams) -> bool {
    params.post_only || params.order_type == OrderType::PostOnly
}

pub(crate) fn fee_product_for(opp: &ArbitrageOpportunityDto, role: HedgeLegRole) -> FeeProduct {
    p0_hedge_leg_product(opp.strategy_kind, opp.spot_leg_mode, role).unwrap_or(FeeProduct::Unknown)
}

pub(super) fn fee_snapshots(long: &FeeLookup, short: &FeeLookup) -> Vec<TradeFeeSnapshot> {
    [long.snapshot.clone(), short.snapshot.clone()]
        .into_iter()
        .flatten()
        .collect()
}
