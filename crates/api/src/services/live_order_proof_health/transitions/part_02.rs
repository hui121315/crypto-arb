fn problem_is_newer_than_latest_proof(row: &LiveOrderProofRuntimeHealth) -> bool {
    let Some(problem) = row.last_problem.as_ref() else {
        return false;
    };
    latest_proof_at(row)
        .map(|checked_at_ms| problem.observed_at_ms > checked_at_ms)
        .unwrap_or(true)
}

fn latest_proof_at(row: &LiveOrderProofRuntimeHealth) -> Option<i64> {
    [
        sample_checked_at(&row.place_proof),
        sample_checked_at(&row.cancel_request),
        sample_checked_at(&row.cancel_finality),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn is_live_adapter_ack(record: &OrderRecord) -> bool {
    record.intent.mode == ExecutionMode::Live
        && record.last_update_source == OrderUpdateSource::AdapterAck
}

fn place_ack_state(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Filled
            | LiveOrderState::CancelRequested
            | LiveOrderState::Cancelled
    )
}

fn record_sample(record: &OrderRecord, source: &str) -> LiveOrderProofSample {
    let identity = record.identity_snapshot();
    let transport = &identity.transport_metadata;
    LiveOrderProofSample {
        venue: record.intent.exchange.clone(),
        symbol: record.intent.symbol.clone(),
        internal_order_id: record.intent.id.clone(),
        exchange_order_id: record
            .exchange_order_id
            .clone()
            .or_else(|| identity.exchange_order_id.clone()),
        client_order_id: Some(identity.public_client_order_id),
        source: source.to_owned(),
        checked_at_ms: record.updated_at_ms,
        request_id: common::request_id::current(),
        native_transport: transport.native_transport.clone(),
        native_request_id: transport.native_request_id.clone(),
        native_response_id: transport.native_response_id.clone(),
    }
}

fn incomplete_message(row: &LiveOrderProofRuntimeHealth) -> String {
    if row
        .place_proof
        .as_ref()
        .zip(row.cancel_finality.as_ref())
        .is_some_and(|(place, cancel)| !samples_match_order_identity(place, cancel))
    {
        return "live 下单 ack 与撤单终态样本订单身份不一致，仍缺同一订单的远程证明闭环".to_owned();
    }
    match (
        row.place_proof.is_some(),
        row.cancel_request.is_some(),
        row.cancel_finality.is_some(),
    ) {
        (true, true, false) => "已有 live 下单 ack 与撤单请求样本，仍缺撤单终态远程证明".to_owned(),
        (true, false, false) => {
            "已有 live 下单 ack 样本，仍缺撤单请求与撤单终态远程证明".to_owned()
        }
        (false, true, true) => "已有撤单闭环样本，仍缺对应 live 下单 ack 远程证明".to_owned(),
        (false, true, false) => {
            "已有撤单请求样本，仍缺 live 下单 ack 与撤单终态远程证明".to_owned()
        }
        (false, false, true) => "已有撤单终态样本，仍缺对应 live 下单 ack 远程证明".to_owned(),
        (true, false, true) => "live 下单 ack 与撤单终态已完成，撤单请求样本未记录".to_owned(),
        _ => "尚未取得完整 live 下单/撤单远程证明样本".to_owned(),
    }
}

fn completed_proof_rows(row: &LiveOrderProofRuntimeHealth) -> u64 {
    if has_complete_remote_proof(row) {
        LIVE_ORDER_PROOF_EXPECTED_STEPS
    } else {
        u64::from(has_any_sample(row))
    }
}

fn has_any_sample(row: &LiveOrderProofRuntimeHealth) -> bool {
    row.place_proof.is_some() || row.cancel_request.is_some() || row.cancel_finality.is_some()
}

fn has_complete_remote_proof(row: &LiveOrderProofRuntimeHealth) -> bool {
    matched_complete_proof_at(row).is_some()
}

fn should_preserve_completed_pair(
    row: &LiveOrderProofRuntimeHealth,
    event: LiveOrderProofEvent,
    sample: &LiveOrderProofSample,
) -> bool {
    if !has_complete_remote_proof(row) || problem_is_newer_than_latest_proof(row) {
        return false;
    }
    let matches_place = row
        .place_proof
        .as_ref()
        .is_some_and(|place| samples_match_order_identity(place, sample));
    let matches_cancel = row
        .cancel_finality
        .as_ref()
        .is_some_and(|cancel| samples_match_order_identity(sample, cancel));
    match event {
        LiveOrderProofEvent::PlaceAck => !matches_cancel,
        LiveOrderProofEvent::CancelRequested => !(matches_place || matches_cancel),
        LiveOrderProofEvent::CancelFinality => !matches_place,
    }
}

fn matched_complete_proof_at(row: &LiveOrderProofRuntimeHealth) -> Option<i64> {
    let place = row.place_proof.as_ref()?;
    let cancel = row.cancel_finality.as_ref()?;
    samples_match_order_identity(place, cancel)
        .then(|| place.checked_at_ms.max(cancel.checked_at_ms))
}

fn samples_match_order_identity(
    place: &LiveOrderProofSample,
    cancel: &LiveOrderProofSample,
) -> bool {
    normalized_venue_name(&place.venue) == normalized_venue_name(&cancel.venue)
        && place.symbol.eq_ignore_ascii_case(&cancel.symbol)
        && samples_share_non_empty_identity(place, cancel)
}

fn samples_share_non_empty_identity(
    place: &LiveOrderProofSample,
    cancel: &LiveOrderProofSample,
) -> bool {
    let identities = [
        required_identity_status(&place.internal_order_id, &cancel.internal_order_id),
        optional_identity_status(&place.exchange_order_id, &cancel.exchange_order_id),
        optional_identity_status(&place.client_order_id, &cancel.client_order_id),
    ];
    identities.contains(&IdentityStatus::Match) && !identities.contains(&IdentityStatus::Conflict)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentityStatus {
    Absent,
    Match,
    Conflict,
}

fn required_identity_status(left: &str, right: &str) -> IdentityStatus {
    if same_non_empty(left, right) {
        IdentityStatus::Match
    } else {
        IdentityStatus::Conflict
    }
}

fn optional_identity_status(left: &Option<String>, right: &Option<String>) -> IdentityStatus {
    match (left.as_deref(), right.as_deref()) {
        (None, None) => IdentityStatus::Absent,
        (Some(left), Some(right)) if same_non_empty(left, right) => IdentityStatus::Match,
        _ => IdentityStatus::Conflict,
    }
}

fn same_non_empty(left: &str, right: &str) -> bool {
    !left.trim().is_empty() && left == right
}

fn latest_observed_at(row: &LiveOrderProofRuntimeHealth) -> Option<i64> {
    [
        sample_checked_at(&row.place_proof),
        sample_checked_at(&row.cancel_request),
        sample_checked_at(&row.cancel_finality),
        row.last_problem
            .as_ref()
            .map(|problem| problem.observed_at_ms),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn latest_request_id(row: &LiveOrderProofRuntimeHealth) -> Option<String> {
    let sample_request_id = [
        sample_request_id(&row.cancel_finality),
        sample_request_id(&row.place_proof),
        sample_request_id(&row.cancel_request),
    ]
    .into_iter()
    .flatten()
    .next();
    if sample_request_id.is_some() {
        return sample_request_id;
    }
    if matched_complete_proof_at(row).is_some() && !problem_is_newer_than_latest_proof(row) {
        return None;
    }
    row.last_problem
        .as_ref()
        .and_then(|problem| problem.request_id.clone())
}

fn sample_checked_at(sample: &Option<LiveOrderProofSample>) -> Option<i64> {
    sample.as_ref().map(|sample| sample.checked_at_ms)
}

fn sample_request_id(sample: &Option<LiveOrderProofSample>) -> Option<String> {
    sample.as_ref().and_then(|sample| sample.request_id.clone())
}

fn with_freshness(
    mut row: LiveOrderProofRuntimeHealth,
    now_ms: i64,
) -> LiveOrderProofRuntimeHealth {
    row.freshness_ms = Some(now_ms.saturating_sub(row.observed_at_ms));
    row
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
