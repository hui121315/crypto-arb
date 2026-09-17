use super::*;

pub(crate) fn positions_evidence_guard(
    mode: ExecutionMode,
    evidence: &HedgePreviewPositionsEvidence,
    positions: &[PositionInfo],
    intents: &[&shared_types::OrderIntent],
) -> shared_types::ExecutionGuard {
    let live_evidence_required = mode == ExecutionMode::Live;
    let blockers = kucoin_position_intent_blockers(positions, intents);
    let passed =
        !live_evidence_required || (evidence.status == ListStatus::Fresh && blockers.is_empty());
    let detail = positions_evidence_detail(mode, evidence, &blockers, passed);
    shared_types::ExecutionGuard {
        key: "positions_evidence".to_owned(),
        label: "持仓/强平证据".to_owned(),
        passed,
        detail: detail.clone(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: if passed {
                HedgePreflightStatus::Passed
            } else {
                HedgePreflightStatus::Blocked
            },
            checked_at_ms: evidence.observed_at_ms,
            scope: if live_evidence_required {
                positions_evidence_scope(evidence)
            } else {
                HedgePreflightScope {
                    operations: vec![HedgePreflightOperation::Positions],
                    ..HedgePreflightScope::default()
                }
            },
            observed_venues: if live_evidence_required {
                positions_observed_venues(evidence)
            } else {
                Vec::new()
            },
            balance_rows: Vec::new(),
            source: Some(evidence.source.clone()),
            freshness_ms: if live_evidence_required {
                positions_freshness_ms(evidence)
            } else {
                None
            },
            retry_after_ms: if live_evidence_required {
                evidence.retry_after_ms
            } else {
                None
            },
            request_id: if live_evidence_required {
                evidence.request_id.clone()
            } else {
                None
            },
            problems: if live_evidence_required {
                evidence.problems.clone()
            } else {
                Vec::new()
            },
            field_quality: if live_evidence_required {
                evidence.field_quality.clone()
            } else {
                Vec::new()
            },
            row_health: if live_evidence_required {
                evidence.row_health.clone()
            } else {
                Vec::new()
            },
            error: (!passed).then_some(detail),
        }),
    }
}

fn positions_evidence_detail(
    mode: ExecutionMode,
    evidence: &HedgePreviewPositionsEvidence,
    blockers: &[String],
    passed: bool,
) -> String {
    if mode != ExecutionMode::Live {
        return "模拟模式：不要求交易所私有持仓/强平证据".into();
    }
    if passed {
        return format!(
            "通过 · {} 行持仓 · source {}",
            evidence.row_count, evidence.source
        );
    }
    let mut parts = vec![format!("持仓/强平证据阻断: status {:?}", evidence.status)];
    if !evidence.problems.is_empty() {
        parts.push(format!("{} 个问题", evidence.problems.len()));
    }
    if !evidence.field_quality.is_empty() {
        parts.push(format!("{} 个字段缺证据", evidence.field_quality.len()));
    }
    if operation_health_needs_attention(&evidence.operation_health) {
        parts.push("operation health degraded".into());
    }
    parts.extend(blockers.iter().cloned());
    parts.join("; ")
}

fn kucoin_position_intent_blockers(
    positions: &[PositionInfo],
    intents: &[&shared_types::OrderIntent],
) -> Vec<String> {
    intents
        .iter()
        .filter(|intent| {
            intent.mode == shared_types::ExecutionMode::Live
                && venue_family(&intent.exchange) == "kucoin"
        })
        .flat_map(|intent| kucoin_intent_blockers(positions, intent))
        .fold(Vec::new(), |mut blockers, blocker| {
            if !blockers.contains(&blocker) {
                blockers.push(blocker);
            }
            blockers
        })
}

fn kucoin_intent_blockers(
    positions: &[PositionInfo],
    intent: &shared_types::OrderIntent,
) -> Vec<String> {
    positions
        .iter()
        .filter(|position| {
            venue_family(&position.exchange) == "kucoin"
                && position.symbol.eq_ignore_ascii_case(&intent.symbol)
        })
        .flat_map(|position| kucoin_position_blockers(position, intent))
        .collect()
}

fn kucoin_position_blockers(
    position: &PositionInfo,
    intent: &shared_types::OrderIntent,
) -> Vec<String> {
    let scope = format!("KuCoin {} 当前持仓", intent.symbol);
    let mut blockers = Vec::with_capacity(2);
    if !position.leverage.is_finite() || position.leverage <= 0.0 {
        blockers.push(format!("{scope} 缺少有效 leverage 证据"));
    } else if (position.leverage - intent.leverage).abs() > 1e-9 {
        blockers.push(format!(
            "{scope} leverage={} 与票据 leverage={} 冲突",
            position.leverage, intent.leverage
        ));
    }
    match position.margin_mode.as_deref().map(str::trim) {
        Some(mode) if margin_mode_matches(mode, intent.margin_mode) => {}
        Some(mode) if !mode.is_empty() => blockers.push(format!(
            "{scope} marginMode={mode} 与票据 marginMode={} 冲突",
            intent_margin_mode(intent.margin_mode)
        )),
        _ => blockers.push(format!("{scope} 缺少 marginMode 证据")),
    }
    blockers
}

fn margin_mode_matches(mode: &str, expected: shared_types::MarginMode) -> bool {
    mode.eq_ignore_ascii_case(intent_margin_mode(expected))
}

fn intent_margin_mode(mode: shared_types::MarginMode) -> &'static str {
    match mode {
        shared_types::MarginMode::Cross => "cross",
        shared_types::MarginMode::Isolated => "isolated",
    }
}

fn operation_health_needs_attention(rows: &[VenueOperationHealth]) -> bool {
    rows.iter().any(|row| {
        matches!(
            row.status,
            VenueOperationStatus::Warn
                | VenueOperationStatus::Blocked
                | VenueOperationStatus::Unknown
        )
    })
}

fn positions_evidence_scope(evidence: &HedgePreviewPositionsEvidence) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: positions_observed_venues(evidence),
        symbols: positions_symbols(evidence),
        account_modes: Vec::new(),
        operations: vec![HedgePreflightOperation::Positions],
    }
}

fn positions_observed_venues(evidence: &HedgePreviewPositionsEvidence) -> Vec<String> {
    let mut venues = Vec::new();
    for venue in evidence
        .operation_health
        .iter()
        .map(|row| row.venue.as_str())
    {
        push_unique_string(&mut venues, venue);
    }
    for quality in &evidence.field_quality {
        if let Some(venue) = quality.subject.venue.as_deref() {
            push_unique_string(&mut venues, venue);
        }
    }
    for health in &evidence.row_health {
        if let Some(venue) = health.subject.venue.as_deref() {
            push_unique_string(&mut venues, venue);
        }
    }
    for binding in &evidence.account_bindings {
        push_unique_string(&mut venues, &binding.venue);
    }
    venues
}

fn positions_symbols(evidence: &HedgePreviewPositionsEvidence) -> Vec<String> {
    let mut symbols = Vec::new();
    for quality in &evidence.field_quality {
        if let Some(symbol) = quality.subject.symbol.as_deref() {
            push_unique_string(&mut symbols, symbol);
        }
    }
    for health in &evidence.row_health {
        if let Some(symbol) = health.subject.symbol.as_deref() {
            push_unique_string(&mut symbols, symbol);
        }
    }
    symbols
}

fn positions_freshness_ms(evidence: &HedgePreviewPositionsEvidence) -> Option<u64> {
    evidence
        .operation_health
        .iter()
        .filter_map(|row| row.freshness_ms.and_then(i64_to_u64))
        .min()
}

fn i64_to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

fn push_unique_string(values: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}
