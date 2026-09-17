use super::*;

pub(super) fn sizing(
    params: &HedgeExecutionParams,
    target_notional: f64,
    notional_caps: [f64; 2],
    target_base_quantity: Option<f64>,
    long: &HedgeLegQuote,
    short: &HedgeLegQuote,
) -> HedgeSizing {
    HedgeSizing {
        requested_capital_usd: params.capital_usd,
        leverage: params.leverage,
        target_notional_usd: target_notional,
        long_notional_cap_usd: notional_caps[0],
        short_notional_cap_usd: notional_caps[1],
        target_base_quantity,
        max_executable_notional: executable_notional_for_quantity(
            target_notional,
            target_base_quantity,
            long,
            short,
        ),
    }
}

pub(super) fn executable_notional_for_quantity(
    fallback_notional: f64,
    target_base_quantity: Option<f64>,
    long: &HedgeLegQuote,
    short: &HedgeLegQuote,
) -> HedgeExecutableNotional {
    let long_target = target_base_quantity
        .and_then(|quantity| long.open_vwap_price.map(|price| quantity * price))
        .unwrap_or(fallback_notional);
    let short_target = target_base_quantity
        .and_then(|quantity| short.open_vwap_price.map(|price| quantity * price))
        .unwrap_or(fallback_notional);
    executable_leg_targets(long_target, short_target, long, short)
}

fn executable_leg_targets(
    long_target: f64,
    short_target: f64,
    long: &HedgeLegQuote,
    short: &HedgeLegQuote,
) -> HedgeExecutableNotional {
    let long_depth = executable_depth(long);
    let short_depth = executable_depth(short);
    match (long_depth, short_depth) {
        (Some(long_depth), Some(short_depth)) => {
            let amount = long_depth
                .min(short_depth)
                .min(long_target)
                .min(short_target)
                .max(0.0);
            let status = if long_depth + f64::EPSILON >= long_target
                && short_depth + f64::EPSILON >= short_target
            {
                HedgeDepthStatus::Available
            } else {
                HedgeDepthStatus::Insufficient
            };
            HedgeExecutableNotional {
                status,
                amount_usd: Some(amount),
                reason: executable_notional_reason(
                    status,
                    long,
                    short,
                    long_target.max(short_target),
                    amount,
                ),
                long_leg_depth_usd: Some(long_depth),
                short_leg_depth_usd: Some(short_depth),
            }
        }
        _ => HedgeExecutableNotional {
            status: HedgeDepthStatus::Unknown,
            amount_usd: None,
            reason: Some(unknown_depth_reason(long, short)),
            long_leg_depth_usd: long_depth,
            short_leg_depth_usd: short_depth,
        },
    }
}

pub(super) fn executable_depth(leg: &HedgeLegQuote) -> Option<f64> {
    leg.depth_usd_5bps
        .filter(|depth| depth.is_finite() && *depth >= 0.0)
}

pub(super) fn executable_notional_reason(
    status: HedgeDepthStatus,
    long: &HedgeLegQuote,
    short: &HedgeLegQuote,
    target_notional: f64,
    amount: f64,
) -> Option<String> {
    (status != HedgeDepthStatus::Available).then(|| {
        format!(
            "{} / {} 可执行深度 ${:.0} 低于目标 ${:.0}",
            leg_label(long),
            leg_label(short),
            amount,
            target_notional
        )
    })
}

pub(super) fn unknown_depth_reason(long: &HedgeLegQuote, short: &HedgeLegQuote) -> String {
    [long, short]
        .into_iter()
        .filter(|leg| executable_depth(leg).is_none())
        .map(leg_depth_reason)
        .collect::<Vec<_>>()
        .join("；")
}

pub(super) fn leg_depth_reason(leg: &HedgeLegQuote) -> String {
    if let Some(reason) = leg
        .depth_reason
        .as_ref()
        .filter(|reason| !reason.is_empty())
    {
        return reason.clone();
    }
    leg.blockers
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{} 可执行深度未知，等待 fresh orderbook", leg_label(leg)))
}

pub(super) fn leg_label(leg: &HedgeLegQuote) -> String {
    format!("{} {}", leg.exchange, leg.symbol)
}
