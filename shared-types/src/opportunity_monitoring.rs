//! Public-market monitoring boundary shared by backend selection and frontend summaries.

use crate::{
    is_p0_executable_strategy, ArbitrageOpportunityDto, MarketDataQuality, MarketDataSourceKind,
    OpportunityLegMarketEvidence, StrategyKind, HEDGE_PREVIEW_MARKET_MAX_AGE_MS,
};

/// Inventory or borrow proof is intentionally deferred to the ticket-bound execution preview.
/// It may block execution, but it must not hide an otherwise verified public-market candidate.
pub const DEFERRED_INVENTORY_OR_BORROW_BLOCKER: &str =
    "反向现货腿需要现货库存或借币 API 证据，当前仅观察不执行";

/// Manual ticket construction owns bilateral depth and the eventual basis-exit proof.
/// Keeping this typed prevents the product list from treating unrelated blockers as previewable.
pub const DEFERRED_SPOT_PERP_TICKET_BLOCKER: &str =
    "期现策略尚缺票据绑定的退出、借贷与持有成本下限，仅观察不执行";

/// Price-spread convergence cannot be guaranteed by public quotes alone. It remains visible to
/// monitoring while live execution stays blocked until a ticket owns the exit proof.
pub const DEFERRED_PERP_PRICE_SPREAD_EXIT_BLOCKER: &str =
    "永续价差依赖未来收敛，历史样本不能锁定本次盈利，仅观察不执行";

/// REST funding may discover a candidate, but only exact fresh WS funding rows may publish it.
pub const FUNDING_WS_EVIDENCE_BLOCKER: &str =
    "Funding 尚未取得相关永续腿所需的精确实时 WS 证据，等待候选订阅后再展示";

/// Transfer failures keep a verified public-market spread visible while blocking execution.
pub const SPOT_CROSS_TRANSFER_BLOCKER_PREFIX: &str = "现货跨所充提闭环未通过：";
pub const SPOT_PERP_TRANSFER_BLOCKER_PREFIX: &str = "现货-永续充提状态未通过：";
pub const CROSS_SPOT_PERP_TRANSFER_BLOCKER_PREFIX: &str = "跨所期现充提闭环未通过：";

/// Product-list build can defer only the basis exit proof owned by the explicit ticket preview.
/// Every other blocker must keep the build action disabled.
#[must_use]
pub fn opportunity_build_blockers_allow_preflight(
    strategy: Option<StrategyKind>,
    blockers: &[String],
) -> bool {
    blockers.is_empty()
        || matches!(
            strategy,
            Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
        ) && blockers == [DEFERRED_SPOT_PERP_TICKET_BLOCKER]
}

/// Unknown, strategy, identity, settlement, market and cost blockers remain fail-closed.
#[must_use]
pub fn monitoring_blockers_are_execution_only(blockers: &[String]) -> bool {
    blockers.iter().all(|blocker| {
        matches!(
            blocker.as_str(),
            DEFERRED_INVENTORY_OR_BORROW_BLOCKER
                | DEFERRED_PERP_PRICE_SPREAD_EXIT_BLOCKER
                | DEFERRED_SPOT_PERP_TICKET_BLOCKER
        ) || blocker.starts_with(SPOT_CROSS_TRANSFER_BLOCKER_PREFIX)
            || blocker.starts_with(SPOT_PERP_TRANSFER_BLOCKER_PREFIX)
            || blocker.starts_with(CROSS_SPOT_PERP_TRANSFER_BLOCKER_PREFIX)
    })
}

/// Keeps the execution flag fail-closed unless every blocker belongs to ticket-bound execution.
#[must_use]
pub fn execution_state_allows_market_monitoring(
    execution_eligible: bool,
    blockers: &[String],
) -> bool {
    execution_eligible && blockers.is_empty()
        || !execution_eligible
            && !blockers.is_empty()
            && monitoring_blockers_are_execution_only(blockers)
}

/// Returns verified positive edge for monitoring without requiring list-level order-book depth.
///
/// Depth, balances, inventory and exit-ticket proof remain mandatory in the explicit hedge
/// preview. This boundary only accepts fresh public WS prices and fresh verified fee evidence.
#[must_use]
pub fn market_monitor_net_bps_at(row: &ArbitrageOpportunityDto, now_ms: i64) -> Option<f64> {
    if !row.strategy_kind.is_some_and(is_p0_executable_strategy)
        || !execution_state_allows_market_monitoring(
            row.execution_eligible,
            &row.execution_blockers,
        )
        || !market_evidence_is_fresh_ws(row.long_leg_market_evidence.as_ref(), now_ms)
        || !market_evidence_is_fresh_ws(row.short_leg_market_evidence.as_ref(), now_ms)
        || !row.quote_conversions.iter().all(|conversion| {
            conversion.rate.is_finite()
                && conversion.rate > 0.0
                && !conversion.venue.trim().is_empty()
                && !conversion.symbol.trim().is_empty()
                && market_evidence_is_fresh_ws(conversion.market_evidence.as_ref(), now_ms)
        })
    {
        return None;
    }

    let cost = row.execution_cost.as_ref()?;
    let round_trip = cost.round_trip.as_ref()?;
    if !cost.one_cycle.covers_round_trip_cost
        || !cost.one_cycle.net_bps.is_finite()
        || cost.one_cycle.net_bps <= f64::EPSILON
        || !round_trip
            .long_leg
            .fee_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_fresh_verified(now_ms))
        || !round_trip
            .short_leg
            .fee_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_fresh_verified(now_ms))
    {
        return None;
    }

    Some(cost.one_cycle.net_bps)
}

fn market_evidence_is_fresh_ws(
    evidence: Option<&OpportunityLegMarketEvidence>,
    now_ms: i64,
) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
            && evidence.health.observed_at_ms > 0
            && evidence.health.observed_at_ms <= now_ms
            && now_ms.saturating_sub(evidence.health.observed_at_ms)
                <= HEDGE_PREVIEW_MARKET_MAX_AGE_MS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ticket_bound_proof_is_deferred_for_monitoring() {
        assert!(monitoring_blockers_are_execution_only(&[]));
        assert!(monitoring_blockers_are_execution_only(&[
            DEFERRED_INVENTORY_OR_BORROW_BLOCKER.to_owned()
        ]));
        assert!(monitoring_blockers_are_execution_only(&[
            DEFERRED_INVENTORY_OR_BORROW_BLOCKER.to_owned(),
            DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned(),
        ]));
        assert!(monitoring_blockers_are_execution_only(&[
            DEFERRED_PERP_PRICE_SPREAD_EXIT_BLOCKER.to_owned()
        ]));
        assert!(monitoring_blockers_are_execution_only(&[format!(
            "{SPOT_CROSS_TRANSFER_BLOCKER_PREFIX}共同网络暂停充提"
        )]));
        assert!(execution_state_allows_market_monitoring(
            false,
            &[format!(
                "{CROSS_SPOT_PERP_TRANSFER_BLOCKER_PREFIX}现货场所提现暂停"
            )]
        ));
        assert!(!monitoring_blockers_are_execution_only(&[
            "双边价格异常".to_owned()
        ]));
        assert!(!execution_state_allows_market_monitoring(false, &[]));
        assert!(execution_state_allows_market_monitoring(
            false,
            &[DEFERRED_INVENTORY_OR_BORROW_BLOCKER.to_owned()]
        ));
        assert!(!execution_state_allows_market_monitoring(
            true,
            &["矛盾的执行阻断".to_owned()]
        ));
    }

    #[test]
    fn build_preflight_accepts_only_the_ticket_bound_basis_blocker() {
        assert!(opportunity_build_blockers_allow_preflight(
            Some(StrategyKind::PerpCross),
            &[],
        ));
        for strategy in [StrategyKind::SpotPerp, StrategyKind::CrossSpotPerp] {
            assert!(opportunity_build_blockers_allow_preflight(
                Some(strategy),
                &[DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned()],
            ));
        }
        assert!(!opportunity_build_blockers_allow_preflight(
            Some(StrategyKind::SpotCross),
            &[DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned()],
        ));
        assert!(!opportunity_build_blockers_allow_preflight(
            Some(StrategyKind::SpotPerp),
            &[
                DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned(),
                "交易所标的身份未通过".to_owned(),
            ],
        ));
    }
}
