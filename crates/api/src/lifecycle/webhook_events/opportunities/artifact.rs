use super::transfer::{candidate_transfer_payload, status_allows_deterministic_delivery};
use crate::services::instrument_registry::CandidateTransferStatus;
use shared_types::{
    DeterministicExecutionArtifact, HedgeLegRole, MarketDataQuality, MarketDataSourceKind,
    OrderSide, StrategyKind, HEDGE_PREVIEW_MARKET_MAX_AGE_MS, TRANSFER_ROUTE_EVIDENCE_KEY,
};

pub(super) fn opportunity_artifact_payload(
    artifact: &DeterministicExecutionArtifact,
    current_transfer: Option<&CandidateTransferStatus>,
    now_ms: i64,
) -> serde_json::Value {
    let transfer = artifact_transfer_payload(artifact, current_transfer);
    let message = artifact_message(artifact, &transfer);
    let mut payload = serde_json::to_value(artifact).unwrap_or_else(|_| {
        serde_json::json!({
            "artifactId": artifact.artifact_id,
            "opportunityId": artifact.opportunity_id,
            "symbol": artifact.symbol,
        })
    });
    if let Some(object) = payload.as_object_mut() {
        object.insert("message".to_owned(), message.into());
        object.insert("mode".to_owned(), "execution_artifact".into());
        object.insert("transfer".to_owned(), transfer);
        object.insert(
            "deterministicOpportunity".to_owned(),
            (deterministic_artifact_ready_at(artifact, now_ms)
                && (!transfer_is_relevant(artifact.strategy)
                    || current_transfer.is_none_or(status_allows_deterministic_delivery)))
            .into(),
        );
    }
    payload
}

pub(super) fn deterministic_artifact_ready_at(
    artifact: &DeterministicExecutionArtifact,
    now_ms: i64,
) -> bool {
    if !artifact.status.is_ready()
        || !artifact.expected_net_edge_usd.is_finite()
        || artifact.expected_net_edge_usd <= 0.0
        || artifact.generated_at_ms <= 0
        || artifact.generated_at_ms > now_ms
        || artifact.expires_at_ms < artifact.generated_at_ms
        || now_ms > artifact.expires_at_ms
        || !artifact_market_evidence_is_current(artifact, now_ms)
    {
        return false;
    }
    if !transfer_is_relevant(artifact.strategy) {
        return true;
    }
    artifact
        .evidence
        .iter()
        .find(|row| row.key == TRANSFER_ROUTE_EVIDENCE_KEY)
        .is_some_and(|row| {
            row.passed
                && row
                    .observed_at_ms
                    .is_some_and(|observed_at_ms| observed_at_ms > 0 && observed_at_ms <= now_ms)
        })
}

fn artifact_market_evidence_is_current(
    artifact: &DeterministicExecutionArtifact,
    now_ms: i64,
) -> bool {
    artifact.legs.len() == 2
        && artifact.legs.iter().all(|leg| {
            leg.market_quality == Some(MarketDataQuality::Fresh)
                && leg.market_source == Some(MarketDataSourceKind::WsPush)
                && leg.market_observed_at_ms.is_some_and(|observed_at_ms| {
                    observed_at_ms > 0
                        && observed_at_ms <= now_ms
                        && now_ms.saturating_sub(observed_at_ms) <= HEDGE_PREVIEW_MARKET_MAX_AGE_MS
                })
        })
}

fn artifact_transfer_payload(
    artifact: &DeterministicExecutionArtifact,
    current_transfer: Option<&CandidateTransferStatus>,
) -> serde_json::Value {
    let relevant = transfer_is_relevant(artifact.strategy);
    let evidence = artifact
        .evidence
        .iter()
        .find(|row| row.key == TRANSFER_ROUTE_EVIDENCE_KEY);
    if relevant {
        if let Some(status) = current_transfer {
            let mut payload = candidate_transfer_payload(status);
            if let Some(object) = payload.as_object_mut() {
                object.insert("relevant".to_owned(), true.into());
                object.insert("required".to_owned(), true.into());
                object.insert("source".to_owned(), "current_registry".into());
                object.insert(
                    "artifactObservedAtMs".to_owned(),
                    evidence
                        .and_then(|row| row.observed_at_ms)
                        .map_or(serde_json::Value::Null, serde_json::Value::from),
                );
            }
            return payload;
        }
    }
    let (state, available, detail) = if !relevant {
        (
            "not_applicable",
            None,
            "当前策略不涉及跨场现货再平衡".to_owned(),
        )
    } else if let Some(evidence) = evidence {
        (
            if evidence.passed {
                "available"
            } else {
                "blocked"
            },
            Some(evidence.passed),
            evidence.detail.clone(),
        )
    } else {
        ("unknown", None, "确定性工件未绑定充提路径证据".to_owned())
    };
    serde_json::json!({
        "relevant": relevant,
        "required": relevant,
        "state": state,
        "available": available,
        "detail": detail,
        "observedAtMs": evidence.and_then(|row| row.observed_at_ms),
        "source": "execution_artifact",
    })
}

pub(super) const fn transfer_is_relevant(strategy: Option<StrategyKind>) -> bool {
    matches!(
        strategy,
        Some(StrategyKind::SpotCross | StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    )
}

fn artifact_message(
    artifact: &DeterministicExecutionArtifact,
    transfer: &serde_json::Value,
) -> String {
    let strategy = artifact
        .strategy
        .map_or("未知策略", |strategy| strategy.label_zh());
    let route = artifact_route(artifact);
    let transfer_detail = transfer
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("充提状态未绑定");
    format!(
        "预检通过的可执行机会\n{strategy} · {}\n{route}\n费后预期净收益 ${:+.4}\n充提：{transfer_detail}\n工件 {}",
        artifact.symbol, artifact.expected_net_edge_usd, artifact.artifact_id,
    )
}

fn artifact_route(artifact: &DeterministicExecutionArtifact) -> String {
    artifact
        .legs
        .iter()
        .map(|leg| {
            let role = match leg.role {
                HedgeLegRole::Long => "多腿",
                HedgeLegRole::Short => "空腿",
            };
            let side = match leg.side {
                OrderSide::Buy => "买入",
                OrderSide::Sell => "卖出",
            };
            format!("{role} {} {side} {}", leg.venue, leg.symbol)
        })
        .collect::<Vec<_>>()
        .join(" → ")
}

#[cfg(test)]
#[path = "artifact/tests.rs"]
mod tests;
