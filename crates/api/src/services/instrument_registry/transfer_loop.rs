use super::{normalized_native_symbol, snapshot::InstrumentLookupSnapshot, InstrumentRegistry};
use exchange::CurrencyTransferNetwork;
use shared_types::instrument_registry::{VenueInstrument, INSTRUMENT_SPEC_FRESHNESS_MS};
use shared_types::{market_monitor_net_bps_at, ArbitrageOpportunityDto, StrategyKind};
use std::sync::Arc;

mod candidate;
mod cost;
mod cross_spot_perp;
mod onchain;
mod registry;
mod route;
mod spot_perp;
mod status;

use cost::apply_transfer_cost;
pub(super) use registry::TransferNetworkSnapshot;
use route::{prove_probe, prove_route, TransferRoute};
pub(crate) use status::CandidateTransferStatus;

const BLOCKER_PREFIX: &str = shared_types::SPOT_CROSS_TRANSFER_BLOCKER_PREFIX;
const EVIDENCE_PREFIX: &str = "现货跨所充提闭环：";

struct TransferLoopIndex {
    instruments: Arc<InstrumentLookupSnapshot>,
    networks: Arc<TransferNetworkSnapshot>,
}

impl TransferLoopIndex {
    fn snapshot(registry: &InstrumentRegistry) -> Self {
        let instruments = registry.instrument_lookup_snapshot();
        let networks = registry.transfer_snapshot.load_full();
        Self {
            instruments,
            networks,
        }
    }

    fn spot(
        &self,
        registry: &InstrumentRegistry,
        venue: &str,
        symbol: &str,
        native_symbol: Option<&str>,
        now_ms: i64,
    ) -> Option<&VenueInstrument> {
        if !registry.probe_allows_execution(venue, now_ms) {
            return None;
        }
        let candidates = self
            .instruments
            .rows(venue, symbol)
            .iter()
            .filter(|row| {
                row.product_type
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("spot"))
            })
            .filter(|row| row.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS))
            .collect::<Vec<_>>();

        if let Some(native_symbol) = native_symbol {
            let native_symbol = normalized_native_symbol(native_symbol);
            if let Some(exact) = candidates
                .iter()
                .find(|row| normalized_native_symbol(&row.native_symbol) == native_symbol)
            {
                return Some(*exact);
            }
        }
        if candidates.len() == 1 {
            return candidates.first().copied();
        }
        let mut usdt = candidates.into_iter().filter(|row| {
            row.quote_asset
                .as_deref()
                .is_some_and(|quote| quote.eq_ignore_ascii_case("USDT"))
        });
        let selected = usdt.next()?;
        usdt.next().is_none().then_some(selected)
    }

    fn transfer_rows(&self, venue: &str, currency: &str) -> &[CurrencyTransferNetwork] {
        self.networks.rows(venue, currency)
    }
}

pub(super) fn apply(
    registry: &InstrumentRegistry,
    opportunities: &mut [ArbitrageOpportunityDto],
    now_ms: i64,
) {
    if !opportunities
        .iter()
        .any(|row| requires_transfer_evaluation(row, now_ms))
    {
        return;
    }
    let index = TransferLoopIndex::snapshot(registry);
    for opportunity in opportunities.iter_mut().filter(|row| {
        row.strategy_kind == Some(StrategyKind::SpotCross)
            && market_monitor_net_bps_at(row, now_ms).is_some()
            && !row
                .risk_warnings
                .iter()
                .any(|warning| warning.starts_with(EVIDENCE_PREFIX))
            && !row
                .execution_blockers
                .iter()
                .any(|blocker| blocker.starts_with(BLOCKER_PREFIX))
    }) {
        match prove_transfer_loop(registry, &index, opportunity, now_ms) {
            Ok((base_route, quote_route, transfer_cost_bps)) => {
                if let Some(blocker) = apply_transfer_cost(opportunity, transfer_cost_bps) {
                    block(opportunity, blocker);
                    continue;
                }
                opportunity.risk_warnings.push(format!(
                    "{EVIDENCE_PREFIX}Base 经 {}，Quote 经 {}；目标规模充提成本约 {:.3}%{}",
                    base_route.network,
                    quote_route.network,
                    transfer_cost_bps / 100.0,
                    if base_route.requires_tag || quote_route.requires_tag {
                        "；到账地址需绑定 Memo/Tag"
                    } else {
                        ""
                    }
                ));
            }
            Err(reason) => block(opportunity, reason),
        }
    }
    spot_perp::apply(registry, &index, opportunities, now_ms);
    cross_spot_perp::apply(registry, &index, opportunities, now_ms);
}

fn requires_transfer_evaluation(opportunity: &ArbitrageOpportunityDto, now_ms: i64) -> bool {
    matches!(
        opportunity.strategy_kind,
        Some(StrategyKind::SpotCross | StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    ) && market_monitor_net_bps_at(opportunity, now_ms).is_some()
}

fn prove_transfer_loop(
    registry: &InstrumentRegistry,
    index: &TransferLoopIndex,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> Result<(TransferRoute, TransferRoute, f64), String> {
    prove_probe(registry, &opportunity.long_exchange, now_ms)?;
    prove_probe(registry, &opportunity.short_exchange, now_ms)?;

    let long_spot = index
        .spot(
            registry,
            &opportunity.long_exchange,
            &opportunity.symbol,
            opportunity
                .long_leg_market_evidence
                .as_ref()
                .map(|row| row.symbol.as_str()),
            now_ms,
        )
        .ok_or_else(|| {
            format!(
                "{BLOCKER_PREFIX}{} 买入腿现货交易对无法解析，无法确认返程资产",
                opportunity.long_exchange.to_ascii_uppercase()
            )
        })?;
    let short_spot = index
        .spot(
            registry,
            &opportunity.short_exchange,
            &opportunity.symbol,
            opportunity
                .short_leg_market_evidence
                .as_ref()
                .map(|row| row.symbol.as_str()),
            now_ms,
        )
        .ok_or_else(|| {
            format!(
                "{BLOCKER_PREFIX}{} 卖出腿现货交易对无法解析，无法确认返程资产",
                opportunity.short_exchange.to_ascii_uppercase()
            )
        })?;
    let long_quote = long_spot
        .quote_asset
        .as_deref()
        .ok_or_else(|| format!("{BLOCKER_PREFIX}买入腿缺少官方 Quote 资产证据"))?;
    let short_quote = short_spot
        .quote_asset
        .as_deref()
        .ok_or_else(|| format!("{BLOCKER_PREFIX}卖出腿缺少官方 Quote 资产证据"))?;
    if !long_quote.eq_ignore_ascii_case(short_quote) {
        return Err(format!(
            "{BLOCKER_PREFIX}两腿计价资产 {} / {} 不同，返程兑换订单与成本尚未绑定",
            long_quote.to_ascii_uppercase(),
            short_quote.to_ascii_uppercase()
        ));
    }

    let notional = opportunity.optimal_position;
    let base_price = opportunity
        .long_price
        .filter(|price| *price > 0.0)
        .ok_or_else(|| format!("{BLOCKER_PREFIX}买入腿价格缺失，无法核验提币最低限额与成本"))?;
    if !notional.is_finite() || notional <= 0.0 {
        return Err(format!(
            "{BLOCKER_PREFIX}目标仓位未绑定，无法核验提币最低限额与成本"
        ));
    }
    let base_amount = notional / base_price;
    let base_route = prove_route(
        index,
        &opportunity.long_exchange,
        &opportunity.short_exchange,
        &opportunity.symbol,
        base_amount,
        now_ms,
    )?;
    let quote_route = prove_route(
        index,
        &opportunity.short_exchange,
        &opportunity.long_exchange,
        long_quote,
        notional,
        now_ms,
    )?;
    let transfer_cost_usd = base_route.fee_units * base_price + quote_route.fee_units;
    let transfer_cost_bps = transfer_cost_usd / notional * 10_000.0;
    if !transfer_cost_bps.is_finite() || transfer_cost_bps < 0.0 {
        return Err(format!(
            "{BLOCKER_PREFIX}官方充提成本无法换算为目标仓位成本"
        ));
    }
    Ok((base_route, quote_route, transfer_cost_bps))
}

fn block(opportunity: &mut ArbitrageOpportunityDto, reason: String) {
    opportunity.execution_eligible = false;
    if !opportunity.execution_blockers.contains(&reason) {
        opportunity.execution_blockers.push(reason);
    }
}

fn canonical_currency(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}
