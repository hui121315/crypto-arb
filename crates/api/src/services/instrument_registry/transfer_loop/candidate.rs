use super::super::InstrumentRegistry;
use super::{
    cross_spot_perp, prove_transfer_loop, spot_perp, status, CandidateTransferStatus,
    TransferLoopIndex, BLOCKER_PREFIX,
};
use shared_types::{market_monitor_net_bps_at, ArbitrageOpportunityDto, StrategyKind};

impl InstrumentRegistry {
    pub(crate) fn should_refresh_transfer_for_candidate(
        &self,
        opportunity: &ArbitrageOpportunityDto,
        now_ms: i64,
    ) -> bool {
        if !opportunity.net_single_yield.is_finite()
            || opportunity.net_single_yield <= 0.0
            || market_monitor_net_bps_at(opportunity, now_ms).is_none()
            || !self.probe_allows_execution(&opportunity.long_exchange, now_ms)
            || !self.probe_allows_execution(&opportunity.short_exchange, now_ms)
        {
            return false;
        }
        match opportunity.strategy_kind {
            Some(StrategyKind::SpotCross) => blockers_only_match(opportunity, BLOCKER_PREFIX),
            Some(StrategyKind::CrossSpotPerp) => {
                cross_spot_perp::blockers_allow_refresh(opportunity)
            }
            Some(StrategyKind::SpotPerp) => spot_perp::blockers_allow_refresh(opportunity),
            _ => false,
        }
    }

    pub(crate) fn candidate_transfer_status(
        &self,
        opportunity: &ArbitrageOpportunityDto,
        now_ms: i64,
    ) -> CandidateTransferStatus {
        let index = TransferLoopIndex::snapshot(self);
        match opportunity.strategy_kind {
            Some(StrategyKind::SpotCross) => {
                match prove_transfer_loop(self, &index, opportunity, now_ms) {
                    Ok((base, quote, cost_bps)) => CandidateTransferStatus::Available {
                        detail: format!(
                            "双向充提可用：Base 经 {}，Quote 经 {}；目标规模成本约 {:.3}%",
                            base.network,
                            quote.network,
                            cost_bps / 100.0,
                        ),
                        base_network: base.network,
                        quote_network: quote.network,
                        requires_tag: base.requires_tag || quote.requires_tag,
                    },
                    Err(reason) => status::from_blocker(&reason, BLOCKER_PREFIX),
                }
            }
            Some(StrategyKind::CrossSpotPerp) => {
                cross_spot_perp::status(self, &index, opportunity, now_ms)
            }
            Some(StrategyKind::SpotPerp) => spot_perp::status(self, &index, opportunity, now_ms),
            _ => CandidateTransferStatus::NotApplicable,
        }
    }
}

fn blockers_only_match(opportunity: &ArbitrageOpportunityDto, prefix: &str) -> bool {
    opportunity
        .execution_blockers
        .iter()
        .any(|blocker| blocker.starts_with(prefix))
        && opportunity
            .execution_blockers
            .iter()
            .all(|blocker| blocker.starts_with(prefix))
}
