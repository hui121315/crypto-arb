use super::super::InstrumentRegistry;
use super::{block, CandidateTransferStatus, TransferLoopIndex};
use shared_types::{
    market_monitor_net_bps_at, monitoring_blockers_are_execution_only, p0_hedge_leg_product,
    ArbitrageOpportunityDto, FeeProduct, HedgeLegRole, StrategyKind,
};

pub(super) const BLOCKER_PREFIX: &str = shared_types::SPOT_PERP_TRANSFER_BLOCKER_PREFIX;
const EVIDENCE_PREFIX: &str = "现货-永续充提状态：";

pub(super) fn blockers_allow_refresh(opportunity: &ArbitrageOpportunityDto) -> bool {
    opportunity
        .execution_blockers
        .iter()
        .any(|blocker| blocker.starts_with(BLOCKER_PREFIX))
        && opportunity.execution_blockers.iter().all(|blocker| {
            blocker.starts_with(BLOCKER_PREFIX)
                || monitoring_blockers_are_execution_only(std::slice::from_ref(blocker))
        })
}

pub(super) fn apply(
    registry: &InstrumentRegistry,
    index: &TransferLoopIndex,
    opportunities: &mut [ArbitrageOpportunityDto],
    now_ms: i64,
) {
    for opportunity in opportunities.iter_mut().filter(|row| {
        row.strategy_kind == Some(StrategyKind::SpotPerp)
            && market_monitor_net_bps_at(row, now_ms).is_some()
    }) {
        match status(registry, index, opportunity, now_ms) {
            CandidateTransferStatus::Available { detail, .. }
            | CandidateTransferStatus::NotRequired { detail } => {
                let warning = format!("{EVIDENCE_PREFIX}{detail}");
                if !opportunity.risk_warnings.contains(&warning) {
                    opportunity.risk_warnings.push(warning);
                }
            }
            CandidateTransferStatus::Warming { detail }
            | CandidateTransferStatus::Blocked { detail } => {
                block(opportunity, format!("{BLOCKER_PREFIX}{detail}"));
            }
            CandidateTransferStatus::NotApplicable => {}
        }
    }
}

pub(super) fn status(
    registry: &InstrumentRegistry,
    index: &TransferLoopIndex,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> CandidateTransferStatus {
    if !shared_types::venue_family(&opportunity.long_exchange)
        .eq_ignore_ascii_case(shared_types::venue_family(&opportunity.short_exchange))
    {
        return blocked("两腿不在同一交易所，应按跨所期现核验双向调拨路径");
    }
    let Some((venue, native_symbol)) = spot_leg(opportunity) else {
        return blocked("现货腿方向证据缺失，无法确认充提资产");
    };
    if let Some(status) = super::cross_spot_perp::probe_status(registry, venue, now_ms) {
        return status;
    }
    let Some(spot) = index.spot(registry, venue, &opportunity.symbol, native_symbol, now_ms) else {
        return blocked("现货交易对无法解析，无法确认 Base/Quote 充提资产");
    };
    let Some(quote) = spot.quote_asset.as_deref() else {
        return blocked("现货交易对缺少官方 Quote 资产证据");
    };
    let base = prove_asset_mobility(index, venue, &opportunity.symbol, now_ms);
    let quote_route = prove_asset_mobility(index, venue, quote, now_ms);
    let (base, quote_route) = match (base, quote_route) {
        (Ok(base), Ok(quote_route)) => (base, quote_route),
        (Err(detail), _) | (_, Err(detail)) => return blocked(&detail),
    };
    CandidateTransferStatus::Available {
        detail: format!(
            "同场资产通道可用：Base 经 {} 可充可提，Quote 经 {} 可充可提；本次开仓无需跨所搬币",
            base.network, quote_route.network,
        ),
        base_network: base.network,
        quote_network: quote_route.network,
        requires_tag: base.requires_tag || quote_route.requires_tag,
    }
}

fn spot_leg(opportunity: &ArbitrageOpportunityDto) -> Option<(&str, Option<&str>)> {
    for role in [HedgeLegRole::Long, HedgeLegRole::Short] {
        if p0_hedge_leg_product(opportunity.strategy_kind, opportunity.spot_leg_mode, role)
            != Some(FeeProduct::Spot)
        {
            continue;
        }
        return match role {
            HedgeLegRole::Long => Some((
                opportunity.long_exchange.as_str(),
                opportunity
                    .long_leg_market_evidence
                    .as_ref()
                    .map(|row| row.symbol.as_str()),
            )),
            HedgeLegRole::Short => Some((
                opportunity.short_exchange.as_str(),
                opportunity
                    .short_leg_market_evidence
                    .as_ref()
                    .map(|row| row.symbol.as_str()),
            )),
        };
    }
    None
}

fn prove_asset_mobility(
    index: &TransferLoopIndex,
    venue: &str,
    currency: &str,
    now_ms: i64,
) -> Result<AssetMobility, String> {
    let mut rows = index
        .transfer_rows(venue, currency)
        .iter()
        .filter(|row| row.is_fresh_at(now_ms))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(format!(
            "{} {} 缺少新鲜官方充提网络明细",
            venue.to_ascii_uppercase(),
            currency.to_ascii_uppercase(),
        ));
    }
    rows.retain(|row| row.deposit_enabled && row.withdraw_enabled);
    rows.sort_by(|left, right| left.canonical_network.cmp(&right.canonical_network));
    let Some(row) = rows.first() else {
        return Err(format!(
            "{} {} 当前没有同时开放充值和提币的网络",
            venue.to_ascii_uppercase(),
            currency.to_ascii_uppercase(),
        ));
    };
    Ok(AssetMobility {
        network: row.canonical_network.clone(),
        requires_tag: row.requires_tag,
    })
}

struct AssetMobility {
    network: String,
    requires_tag: bool,
}

fn blocked(detail: &str) -> CandidateTransferStatus {
    CandidateTransferStatus::Blocked {
        detail: detail.to_owned(),
    }
}
