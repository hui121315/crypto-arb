use super::super::{emit_value, trim_cursor, Cursor};
use super::transfer::candidate_transfer_payload;
use crate::services::instrument_registry::CandidateTransferStatus;
use crate::state::AppState;
use shared_types::{
    market_monitor_net_bps_at, ArbitrageOpportunityDto, StrategyKind, WebhookEventKind,
};
use std::hash::{Hash, Hasher};

const MONITOR_BASELINE_SETTLE_MS: i64 = 30_000;

pub(super) fn current_monitor_event_keys(
    state: &AppState,
    excluded_opportunity_id: Option<&str>,
) -> std::collections::HashMap<String, i64> {
    let now_ms = common::time::now_ms();
    monitored_transfer_candidates(state, excluded_opportunity_id)
        .into_iter()
        .map(|(row, status, _)| (monitor_event_key(&row.id, &status), now_ms))
        .collect()
}

pub(super) async fn emit_transfer_monitors(
    state: &AppState,
    cursor: &mut Cursor,
    excluded_opportunity_id: Option<&str>,
) {
    let candidates = monitored_transfer_candidates(state, excluded_opportunity_id);
    if !cursor.opportunity_monitor_initialized {
        let now_ms = common::time::now_ms();
        let next = candidates
            .iter()
            .map(|(row, status, _)| (monitor_event_key(&row.id, status), now_ms))
            .collect();
        settle_monitor_baseline(cursor, next, now_ms);
        return;
    }
    for (row, status, market_net_bps) in candidates {
        let event_key = monitor_event_key(&row.id, &status);
        if cursor.opportunity_monitor_keys.contains_key(&event_key) {
            continue;
        }
        let payload = transfer_monitor_payload(&row, &status, market_net_bps);
        if emit_value(
            state,
            WebhookEventKind::OpportunityMonitor,
            format!("opportunity-transfer-{event_key}"),
            &payload,
        )
        .await
        {
            cursor
                .opportunity_monitor_keys
                .insert(event_key, common::time::now_ms());
        }
    }
    trim_cursor(&mut cursor.opportunity_monitor_keys);
}

fn settle_monitor_baseline(
    cursor: &mut Cursor,
    next: std::collections::HashMap<String, i64>,
    now_ms: i64,
) {
    if next.is_empty() {
        cursor.opportunity_monitor_keys.clear();
        cursor.opportunity_monitor_arm_after_ms = 0;
        return;
    }
    if cursor.opportunity_monitor_force_arm_after_ms > 0
        && now_ms >= cursor.opportunity_monitor_force_arm_after_ms
    {
        cursor.opportunity_monitor_keys = next;
        cursor.opportunity_monitor_initialized = true;
        return;
    }
    let changed = next.len() != cursor.opportunity_monitor_keys.len()
        || next
            .keys()
            .any(|key| !cursor.opportunity_monitor_keys.contains_key(key));
    if changed || cursor.opportunity_monitor_arm_after_ms == 0 {
        cursor.opportunity_monitor_keys = next;
        cursor.opportunity_monitor_arm_after_ms = now_ms.saturating_add(MONITOR_BASELINE_SETTLE_MS);
        return;
    }
    if now_ms >= cursor.opportunity_monitor_arm_after_ms {
        cursor.opportunity_monitor_initialized = true;
    }
}

fn monitored_transfer_candidates(
    state: &AppState,
    excluded_opportunity_id: Option<&str>,
) -> Vec<(ArbitrageOpportunityDto, CandidateTransferStatus, f64)> {
    let Some(view) = state.opportunity_index().read() else {
        return Vec::new();
    };
    let now_ms = common::time::now_ms();
    view.rows()
        .iter()
        .filter(|row| {
            excluded_opportunity_id != Some(row.id.as_str())
                && is_transfer_strategy(row.strategy_kind)
                && crate::services::opportunity::is_product_visible_row(row, now_ms)
        })
        .filter_map(|row| {
            let status = state
                .instrument_registry()
                .candidate_transfer_status(row, now_ms);
            verified_market_net_bps(row, &status, now_ms)
                .map(|market_net_bps| (row.clone(), status, market_net_bps))
        })
        .collect()
}

fn verified_market_net_bps(
    row: &ArbitrageOpportunityDto,
    status: &CandidateTransferStatus,
    now_ms: i64,
) -> Option<f64> {
    if !transfer_status_is_resolved(status) {
        return None;
    }
    market_monitor_net_bps_at(row, now_ms)
}

const fn is_transfer_strategy(strategy: Option<StrategyKind>) -> bool {
    matches!(
        strategy,
        Some(StrategyKind::SpotCross | StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    )
}

const fn transfer_status_is_resolved(status: &CandidateTransferStatus) -> bool {
    matches!(
        status,
        CandidateTransferStatus::NotRequired { .. }
            | CandidateTransferStatus::Available { .. }
            | CandidateTransferStatus::Blocked { .. }
    )
}

fn monitor_event_key(opportunity_id: &str, status: &CandidateTransferStatus) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    opportunity_id.hash(&mut hasher);
    transfer_status_fingerprint(status).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn transfer_status_fingerprint(status: &CandidateTransferStatus) -> String {
    match status {
        CandidateTransferStatus::NotApplicable => "not_applicable".to_owned(),
        CandidateTransferStatus::NotRequired { .. } => "not_required".to_owned(),
        CandidateTransferStatus::Warming { .. } => "warming".to_owned(),
        CandidateTransferStatus::Available {
            base_network,
            quote_network,
            requires_tag,
            ..
        } => format!(
            "available:{}:{}:{requires_tag}",
            base_network.trim().to_ascii_lowercase(),
            quote_network.trim().to_ascii_lowercase(),
        ),
        CandidateTransferStatus::Blocked { .. } => "blocked".to_owned(),
    }
}

fn transfer_monitor_payload(
    row: &ArbitrageOpportunityDto,
    status: &CandidateTransferStatus,
    market_net_bps: f64,
) -> serde_json::Value {
    let class = monitor_profit_class(row.strategy_kind);
    let transfer = candidate_transfer_payload(status);
    let detail = transfer
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("充提状态未知");
    let available = transfer
        .get("available")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let conclusion = monitor_conclusion(class, status, available);
    let transfer_cost_included =
        class.is_locked() && matches!(status, CandidateTransferStatus::Available { .. });
    let edge_label = if transfer_cost_included {
        "完整循环费后价差"
    } else if class.is_locked() {
        "交易费后价差"
    } else {
        "交易费后投影价差"
    };
    serde_json::json!({
        "message": format!(
            "价差与充提监控\n{} · {}\n{} → {}\n{} {:.4}%\n充提：{}\n{}",
            row.strategy_kind
                .map_or("现货策略", |strategy| strategy.label_zh()),
            row.symbol,
            row.long_exchange,
            row.short_exchange,
            edge_label,
            market_net_bps / 100.0,
            detail,
            conclusion,
        ),
        "mode": "transfer_monitor",
        "marketEdgeClass": class.key(),
        "lockedMarketEdge": class.is_locked(),
        "deterministicOpportunity": false,
        "verifiedMarketNetBps": market_net_bps,
        "transferChecked": true,
        "transferAvailable": available,
        "transferCostIncluded": transfer_cost_included,
        "opportunityId": row.id,
        "strategy": row.strategy_kind,
        "symbol": row.symbol,
        "longVenue": row.long_exchange,
        "shortVenue": row.short_exchange,
        "netSingleYield": row.net_single_yield,
        "transfer": transfer,
        "executionEligible": row.execution_eligible,
        "executionBlockers": row.execution_blockers,
        "observedAtMs": common::time::now_ms(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorProfitClass {
    LockedSpread,
    ProjectedBasis,
}

impl MonitorProfitClass {
    const fn key(self) -> &'static str {
        match self {
            Self::LockedSpread => "locked_spread",
            Self::ProjectedBasis => "projected_basis",
        }
    }

    const fn is_locked(self) -> bool {
        matches!(self, Self::LockedSpread)
    }
}

const fn monitor_profit_class(strategy: Option<StrategyKind>) -> MonitorProfitClass {
    match strategy {
        Some(StrategyKind::SpotCross) => MonitorProfitClass::LockedSpread,
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) | None => {
            MonitorProfitClass::ProjectedBasis
        }
        Some(_) => MonitorProfitClass::ProjectedBasis,
    }
}

fn monitor_conclusion(
    class: MonitorProfitClass,
    status: &CandidateTransferStatus,
    available: bool,
) -> &'static str {
    if !available {
        return "充提闭环不可用，当前价差不能形成套利闭环";
    }
    if !class.is_locked() {
        return if matches!(status, CandidateTransferStatus::NotRequired { .. }) {
            "同所无需跨所充提，但基差退出收益尚未锁定"
        } else {
            "充提路径可用，但基差退出收益尚未锁定"
        };
    }
    "充提闭环可用，仍需执行前深度与余额复核"
}

#[cfg(test)]
#[path = "monitor/tests.rs"]
mod tests;
