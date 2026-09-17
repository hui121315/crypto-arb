use super::super::{normalized_native_symbol, InstrumentRegistry};
use super::{block, prove_route, CandidateTransferStatus, TransferLoopIndex};
use shared_types::instrument_registry::{VenueInstrument, INSTRUMENT_SPEC_FRESHNESS_MS};
use shared_types::{
    market_monitor_net_bps_at, monitoring_blockers_are_execution_only, ArbitrageOpportunityDto,
    StrategyKind,
};

pub(super) const BLOCKER_PREFIX: &str = shared_types::CROSS_SPOT_PERP_TRANSFER_BLOCKER_PREFIX;
const EVIDENCE_PREFIX: &str = "跨所期现充提闭环：";

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
        row.strategy_kind == Some(StrategyKind::CrossSpotPerp)
            && market_monitor_net_bps_at(row, now_ms).is_some()
    }) {
        match status(registry, index, opportunity, now_ms) {
            CandidateTransferStatus::NotRequired { detail }
            | CandidateTransferStatus::Available { detail, .. } => {
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
    if shared_types::venue_family(&opportunity.long_exchange)
        .eq_ignore_ascii_case(shared_types::venue_family(&opportunity.short_exchange))
    {
        return CandidateTransferStatus::NotRequired {
            detail: "同场现货-永续，无需跨交易所充提".into(),
        };
    }
    if let Some(status) = probe_status(registry, &opportunity.long_exchange, now_ms) {
        return status;
    }
    if let Some(status) = probe_status(registry, &opportunity.short_exchange, now_ms) {
        return status;
    }

    let Some(long_spot) = index.spot(
        registry,
        &opportunity.long_exchange,
        &opportunity.symbol,
        opportunity
            .long_leg_market_evidence
            .as_ref()
            .map(|row| row.symbol.as_str()),
        now_ms,
    ) else {
        return blocked("买入腿现货交易对无法解析，无法确认充提资产");
    };
    let Some(short_perp) = perp_instrument(registry, index, opportunity, now_ms) else {
        return blocked("卖出腿永续交易对无法解析，无法确认充提资产");
    };
    let Some(long_quote) = long_spot.quote_asset.as_deref() else {
        return blocked("买入腿缺少官方 Quote 资产证据");
    };
    let Some(short_quote) = short_perp.quote_asset.as_deref() else {
        return blocked("卖出腿缺少官方 Quote 资产证据");
    };
    if !long_quote.eq_ignore_ascii_case(short_quote) {
        return blocked(&format!(
            "两腿计价资产 {} / {} 不同，跨币种调拨路径需在票据中核验",
            long_quote.to_ascii_uppercase(),
            short_quote.to_ascii_uppercase(),
        ));
    }

    let notional = opportunity.optimal_position;
    let Some(base_price) = opportunity.long_price.filter(|price| *price > 0.0) else {
        return blocked("买入腿价格缺失，无法核验充提限额");
    };
    if !notional.is_finite() || notional <= 0.0 {
        return blocked("目标仓位未绑定，无法核验充提限额");
    }
    let base = prove_route(
        index,
        &opportunity.long_exchange,
        &opportunity.short_exchange,
        &opportunity.symbol,
        notional / base_price,
        now_ms,
    );
    let quote = prove_route(
        index,
        &opportunity.short_exchange,
        &opportunity.long_exchange,
        long_quote,
        notional,
        now_ms,
    );
    let (base, quote) = match (base, quote) {
        (Ok(base), Ok(quote)) => (base, quote),
        (Err(reason), _) | (_, Err(reason)) => {
            return blocked(
                reason
                    .strip_prefix(super::BLOCKER_PREFIX)
                    .unwrap_or(&reason),
            );
        }
    };
    let requires_tag = base.requires_tag || quote.requires_tag;
    CandidateTransferStatus::Available {
        detail: format!(
            "双向充提可用：Base 经 {}，Quote 经 {}{}",
            base.network,
            quote.network,
            if requires_tag {
                "；到账需 Memo/Tag"
            } else {
                ""
            },
        ),
        base_network: base.network,
        quote_network: quote.network,
        requires_tag,
    }
}

pub(super) fn probe_status(
    registry: &InstrumentRegistry,
    venue: &str,
    now_ms: i64,
) -> Option<CandidateTransferStatus> {
    use super::super::{TransferProbeState, TRANSFER_SUPPORTED_VENUES};
    use exchange::TRANSFER_NETWORK_FRESHNESS_MS;

    let label = venue.to_ascii_uppercase();
    let family = shared_types::venue_family(venue);
    if !TRANSFER_SUPPORTED_VENUES.contains(&family) {
        return Some(CandidateTransferStatus::Blocked {
            detail: format!("{label} 尚无可核验的官方充提接口"),
        });
    }
    match registry.transfer_probe_state(venue) {
        Some(TransferProbeState::Success { checked_at_ms })
            if now_ms >= checked_at_ms
                && now_ms.saturating_sub(checked_at_ms) < TRANSFER_NETWORK_FRESHNESS_MS =>
        {
            None
        }
        Some(TransferProbeState::Refreshing { checked_at_ms }) => {
            Some(CandidateTransferStatus::Warming {
                detail: format!("{label} 正在按当前候选读取官方充提状态（开始 {checked_at_ms}）"),
            })
        }
        Some(TransferProbeState::Success { .. }) => Some(CandidateTransferStatus::Warming {
            detail: format!("{label} 官方充提状态已过期，等待按候选刷新"),
        }),
        Some(TransferProbeState::Unavailable {
            checked_at_ms,
            problem,
            ..
        }) => Some(CandidateTransferStatus::Blocked {
            detail: format!(
                "{label} 当前候选币种没有官方充提网络（核验 {checked_at_ms}）：{}",
                problem.message
            ),
        }),
        Some(TransferProbeState::Failed {
            checked_at_ms,
            problem,
        }) => Some(CandidateTransferStatus::Blocked {
            detail: format!(
                "{label} 官方充提状态刷新失败（核验 {checked_at_ms}）：{}",
                problem.message
            ),
        }),
        Some(TransferProbeState::Unsupported {
            checked_at_ms,
            problem,
        }) => Some(CandidateTransferStatus::Blocked {
            detail: format!(
                "{label} 尚无可核验的官方充提接口（核验 {checked_at_ms}）：{}",
                problem.message
            ),
        }),
        None => Some(CandidateTransferStatus::Warming {
            detail: format!("{label} 尚未取得官方充提状态，等待候选触发读取"),
        }),
    }
}

fn perp_instrument<'a>(
    registry: &InstrumentRegistry,
    index: &'a TransferLoopIndex,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> Option<&'a VenueInstrument> {
    if !registry.probe_allows_execution(&opportunity.short_exchange, now_ms) {
        return None;
    }
    let native = opportunity
        .short_leg_market_evidence
        .as_ref()
        .map(|row| normalized_native_symbol(&row.symbol));
    let mut candidates = index
        .instruments
        .rows(&opportunity.short_exchange, &opportunity.symbol)
        .iter()
        .filter(|instrument| {
            let product = instrument.product_type.as_deref().unwrap_or_default();
            (product.eq_ignore_ascii_case("perp") || product.eq_ignore_ascii_case("perpetual"))
                && instrument.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
        })
        .collect::<Vec<_>>();
    if let Some(native) = native {
        if let Some(index) = candidates
            .iter()
            .position(|row| normalized_native_symbol(&row.native_symbol) == native)
        {
            return Some(candidates.swap_remove(index));
        }
    }
    if candidates.len() == 1 {
        return candidates.pop();
    }
    let mut usdt = candidates.into_iter().filter(|row| {
        row.quote_asset
            .as_deref()
            .is_some_and(|quote| quote.eq_ignore_ascii_case("USDT"))
    });
    let selected = usdt.next()?;
    usdt.next().is_none().then_some(selected)
}

fn blocked(detail: &str) -> CandidateTransferStatus {
    CandidateTransferStatus::Blocked {
        detail: detail.to_owned(),
    }
}
