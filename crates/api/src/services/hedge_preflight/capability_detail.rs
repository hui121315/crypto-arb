use super::*;

pub(super) fn capability_detail(blockers: &[String]) -> String {
    if blockers.is_empty() {
        "通过".to_owned()
    } else {
        format!("订单能力阻断: {}", blockers.join("; "))
    }
}

pub(super) fn capability_blockers(checks: &[OrderCapabilityCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .flat_map(check_blockers)
        .fold(Vec::new(), push_unique)
}

pub(super) fn check_blockers(check: &OrderCapabilityCheck<'_>) -> Vec<String> {
    let mut blockers = check.plan.blockers.clone();
    match check.capabilities.as_ref() {
        Ok(capabilities) => {
            blockers.extend(product_support_blockers(check.plan, capabilities));
            blockers.extend(order_type_blockers(check.plan, capabilities));
        }
        Err(error) => blockers.push(format!(
            "{} {} 未注册实盘路由或能力不可读: {error}",
            check.plan.exchange, check.plan.symbol
        )),
    }
    blockers
}

pub(super) fn product_support_blockers(
    plan: &OrderCompilePlan,
    capabilities: &ExchangeCapabilities,
) -> Vec<String> {
    match plan.product {
        FeeProduct::Spot if !capabilities.supports_spot => {
            vec![format!("{} 不支持现货腿实盘下单", plan.exchange)]
        }
        FeeProduct::Perp if !capabilities.supports_perp => {
            vec![format!("{} 不支持永续腿实盘下单", plan.exchange)]
        }
        FeeProduct::Margin | FeeProduct::Unknown => {
            vec![format!(
                "{} {} 交易产品类型未验证",
                plan.exchange, plan.symbol
            )]
        }
        _ => Vec::new(),
    }
}

pub(super) fn order_type_blockers(
    plan: &OrderCompilePlan,
    capabilities: &ExchangeCapabilities,
) -> Vec<String> {
    match plan.effective_order_type {
        OrderType::Limit if !capabilities.supports_limit_orders => {
            vec![format!("{} 不支持限价单", plan.exchange)]
        }
        OrderType::Market if !capabilities.supports_market_orders => {
            vec![format!("{} 不支持市价单", plan.exchange)]
        }
        OrderType::PostOnly
            if !capabilities.supports_post_only || !capabilities.supports_limit_orders =>
        {
            vec![format!("{} 不支持 Post-only", plan.exchange)]
        }
        _ => Vec::new(),
    }
}

pub(super) fn capability_scope(checks: &[OrderCapabilityCheck<'_>]) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: capability_venues(checks),
        symbols: capability_symbols(checks),
        account_modes: Vec::new(),
        operations: vec![HedgePreflightOperation::Capability],
    }
}

pub(super) fn capability_venues(checks: &[OrderCapabilityCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .map(|check| normalized_venue_name(&check.plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn observed_capability_venues(checks: &[OrderCapabilityCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| check.capabilities.is_ok())
        .map(|check| normalized_venue_name(&check.plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn capability_symbols(checks: &[OrderCapabilityCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .map(|check| check.plan.symbol.clone())
        .fold(Vec::new(), push_unique)
}

pub(super) fn push_unique(mut values: Vec<String>, value: String) -> Vec<String> {
    if !values.contains(&value) {
        values.push(value);
    }
    values
}

pub(super) fn push_unique_problem(
    mut values: Vec<ApiProblem>,
    value: ApiProblem,
) -> Vec<ApiProblem> {
    if !values.contains(&value) {
        values.push(value);
    }
    values
}
