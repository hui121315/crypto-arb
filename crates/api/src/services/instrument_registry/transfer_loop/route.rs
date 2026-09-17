use super::super::TransferProbeState;
use super::{canonical_currency, InstrumentRegistry, TransferLoopIndex, BLOCKER_PREFIX};
use exchange::{contracts_compatible, TRANSFER_NETWORK_FRESHNESS_MS};
use rust_decimal::prelude::ToPrimitive;

#[derive(Clone)]
pub(super) struct TransferRoute {
    pub(super) network: String,
    pub(super) fee_units: f64,
    pub(super) requires_tag: bool,
}

pub(super) fn prove_probe(
    registry: &InstrumentRegistry,
    venue: &str,
    now_ms: i64,
) -> Result<(), String> {
    let label = venue.to_ascii_uppercase();
    let family = shared_types::venue_family(venue);
    if !super::super::TRANSFER_SUPPORTED_VENUES.contains(&family) {
        return Err(format!("{BLOCKER_PREFIX}{label} 尚无可核验的官方充提接口"));
    }
    match registry.transfer_probe_state(venue) {
        Some(TransferProbeState::Refreshing { checked_at_ms }) => Err(format!(
            "{BLOCKER_PREFIX}{label} 正在按当前候选读取官方充提网络（开始 {checked_at_ms}）"
        )),
        Some(TransferProbeState::Success { checked_at_ms })
            if now_ms >= checked_at_ms
                && now_ms.saturating_sub(checked_at_ms) < TRANSFER_NETWORK_FRESHNESS_MS =>
        {
            Ok(())
        }
        Some(TransferProbeState::Success { .. }) => {
            Err(format!("{BLOCKER_PREFIX}{label} 官方充提网络状态已过期"))
        }
        Some(TransferProbeState::Unavailable {
            checked_at_ms,
            problem,
            ..
        }) => Err(format!(
            "{BLOCKER_PREFIX}{label} 当前候选币种没有官方充提网络（核验 {checked_at_ms}）：{}",
            problem.message
        )),
        Some(TransferProbeState::Failed {
            checked_at_ms,
            problem,
        }) => Err(format!(
            "{BLOCKER_PREFIX}{label} 官方充提网络刷新失败（核验 {checked_at_ms}）：{}",
            problem.message
        )),
        Some(TransferProbeState::Unsupported {
            checked_at_ms,
            problem,
        }) => Err(format!(
            "{BLOCKER_PREFIX}{label} 尚无可核验的官方充提接口（核验 {checked_at_ms}）：{}",
            problem.message
        )),
        None => Err(format!("{BLOCKER_PREFIX}{label} 尚未取得官方充提网络状态")),
    }
}

pub(super) fn prove_route(
    index: &TransferLoopIndex,
    source_venue: &str,
    destination_venue: &str,
    currency: &str,
    amount: f64,
    now_ms: i64,
) -> Result<TransferRoute, String> {
    let source = index
        .transfer_rows(source_venue, currency)
        .iter()
        .filter(|row| row.is_fresh_at(now_ms))
        .collect::<Vec<_>>();
    let destination = index
        .transfer_rows(destination_venue, currency)
        .iter()
        .filter(|row| row.is_fresh_at(now_ms))
        .collect::<Vec<_>>();
    let route_label = format!(
        "{} {} -> {}",
        canonical_currency(currency),
        source_venue.to_ascii_uppercase(),
        destination_venue.to_ascii_uppercase()
    );
    if source.is_empty() || destination.is_empty() {
        return Err(format!(
            "{BLOCKER_PREFIX}{route_label} 缺少新鲜官方网络明细"
        ));
    }

    let common = source
        .iter()
        .flat_map(|withdraw| {
            destination.iter().filter_map(move |deposit| {
                (withdraw.canonical_network == deposit.canonical_network
                    && contracts_compatible(
                        withdraw.contract_address.as_deref(),
                        deposit.contract_address.as_deref(),
                    ))
                .then_some((*withdraw, *deposit))
            })
        })
        .collect::<Vec<_>>();
    if common.is_empty() {
        return Err(format!(
            "{BLOCKER_PREFIX}{route_label} 没有合约身份一致的共同网络"
        ));
    }
    let enabled = common
        .into_iter()
        .filter(|(withdraw, deposit)| withdraw.withdraw_enabled && deposit.deposit_enabled)
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        return Err(format!(
            "{BLOCKER_PREFIX}{route_label} 的共同网络当前未同时开放提币和充值"
        ));
    }

    let mut candidates = Vec::new();
    let mut missing_cost = false;
    let mut below_minimum = Vec::new();
    for (withdraw, deposit) in enabled {
        if !withdraw.has_cost_evidence() {
            missing_cost = true;
            continue;
        }
        let fixed = withdraw
            .withdrawal_fee
            .and_then(|value| value.to_f64())
            .unwrap_or_default();
        let rate = withdraw
            .withdrawal_fee_rate
            .and_then(|value| value.to_f64())
            .unwrap_or_default();
        let min_withdraw = withdraw
            .min_withdraw
            .and_then(|value| value.to_f64())
            .unwrap_or(f64::INFINITY);
        let fee_units = fixed + amount * rate;
        let received = amount - fee_units;
        let min_deposit = deposit
            .min_deposit
            .and_then(|value| value.to_f64())
            .unwrap_or_default();
        if amount + f64::EPSILON < min_withdraw || received + f64::EPSILON < min_deposit {
            below_minimum.push(format!(
                "{}(提币最少 {:.8}，到账最少 {:.8})",
                withdraw.canonical_network, min_withdraw, min_deposit
            ));
            continue;
        }
        candidates.push(TransferRoute {
            network: withdraw.canonical_network.clone(),
            fee_units,
            requires_tag: withdraw.requires_tag || deposit.requires_tag,
        });
    }
    candidates.sort_by(|left, right| left.fee_units.total_cmp(&right.fee_units));
    if let Some(route) = candidates.into_iter().next() {
        return Ok(route);
    }
    if missing_cost {
        return Err(format!(
            "{BLOCKER_PREFIX}{route_label} 共同网络缺少官方手续费或最低提币额证据"
        ));
    }
    Err(format!(
        "{BLOCKER_PREFIX}{route_label} 目标规模低于共同网络限额：{}",
        below_minimum.join("、")
    ))
}
