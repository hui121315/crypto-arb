use super::*;

mod account_summary;

pub(super) use account_summary::{
    account_equity_unknown_problem, account_equity_unknown_quality, account_summary_field_quality,
    account_summary_problems,
};

pub(super) fn account_operation_health_from_rows(
    rows: &[VenueOperationHealth],
) -> Vec<VenueOperationHealth> {
    rows.iter()
        .filter(|row| is_account_state_operation_row(row))
        .cloned()
        .collect()
}

fn is_account_state_operation_row(row: &VenueOperationHealth) -> bool {
    matches!(
        VenueOperationKind::parse(&row.operation),
        VenueOperationKind::CredentialProbeOpenOrdersRead
            | VenueOperationKind::CredentialProbeAccountModeRead
            | VenueOperationKind::CredentialProbeOrderPermission
            | VenueOperationKind::PrivateWsAccountStream
            | VenueOperationKind::PrivateWsOrderStream
    )
}

pub(super) fn child_problems(
    balances: &VenueBalanceEnvelope,
    positions: &VenuePositionEnvelope,
    open_orders: &VenueOpenOrdersEnvelope,
) -> Vec<ApiProblem> {
    balances
        .problems
        .iter()
        .chain(positions.problems.iter())
        .chain(open_orders.problems.iter())
        .cloned()
        .collect()
}

pub(super) fn child_operation_health(
    balances: &VenueBalanceEnvelope,
    positions: &VenuePositionEnvelope,
    open_orders: &VenueOpenOrdersEnvelope,
    account_operation_health: &[VenueOperationHealth],
) -> Vec<VenueOperationHealth> {
    let mut seen = BTreeSet::new();
    balances
        .operation_health
        .iter()
        .chain(positions.operation_health.iter())
        .chain(open_orders.operation_health.iter())
        .chain(account_operation_health.iter())
        .filter_map(|row| {
            let key = (
                normalized_venue_name(&row.venue),
                row.operation.clone(),
                row.source.clone(),
            );
            seen.insert(key).then(|| row.clone())
        })
        .collect()
}

pub(super) fn child_field_quality(
    balances: &VenueBalanceEnvelope,
    positions: &VenuePositionEnvelope,
    open_orders: &VenueOpenOrdersEnvelope,
) -> Vec<AccountFieldQuality> {
    balances
        .field_quality
        .iter()
        .chain(positions.field_quality.iter())
        .chain(open_orders.field_quality.iter())
        .cloned()
        .collect()
}

pub(super) fn child_account_bindings(
    balances: &VenueBalanceEnvelope,
    positions: &VenuePositionEnvelope,
    open_orders: &VenueOpenOrdersEnvelope,
) -> Vec<AccountBindingEvidence> {
    let mut seen = BTreeSet::new();
    balances
        .account_bindings
        .iter()
        .chain(positions.account_bindings.iter())
        .chain(open_orders.account_bindings.iter())
        .filter_map(|row| {
            let key = normalized_venue_name(&row.venue);
            seen.insert(key).then(|| row.clone())
        })
        .collect()
}

pub(super) fn apply_account_scopes(
    rows: &mut [AccountFieldQuality],
    bindings: &[AccountBindingEvidence],
) {
    for row in rows {
        let Some(venue) = row.subject.venue.as_deref() else {
            continue;
        };
        let normalized = normalized_venue_name(venue);
        let scope = bindings.iter().find_map(|binding| {
            (binding.status == shared_types::AccountBindingStatus::Verified
                && normalized_venue_name(&binding.venue) == normalized)
                .then_some(binding.account_scope.as_deref())
                .flatten()
        });
        if let Some(scope) = scope {
            row.subject.account_scope = Some(scope.to_owned());
        }
    }
}

pub(super) fn account_operation_field_quality(
    operation_health: &[VenueOperationHealth],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    operation_health
        .iter()
        .filter_map(|row| kucoin_classic_futures_scope_quality(row, observed_at_ms))
        .collect()
}

fn kucoin_classic_futures_scope_quality(
    row: &VenueOperationHealth,
    observed_at_ms: i64,
) -> Option<AccountFieldQuality> {
    let is_kucoin = normalized_venue_name(&row.venue) == "kucoin";
    let is_account_mode = VenueOperationKind::parse(&row.operation)
        == VenueOperationKind::CredentialProbeAccountModeRead;
    let has_verified_read = row.configured != Some(false) && row.status == VenueOperationStatus::Ok;
    (is_kucoin && is_account_mode && has_verified_read).then(|| {
        AccountFieldQuality::new(
            AccountFieldSubject::account(&row.venue),
            "classicFuturesPrivateReadScope",
            AccountFieldQualityStatus::Estimated,
            row.source.clone(),
            Some(row.observed_at_ms.max(observed_at_ms)),
        )
        .with_problem(kucoin_classic_futures_scope_problem(row, observed_at_ms))
    })
}

fn kucoin_classic_futures_scope_problem(
    row: &VenueOperationHealth,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::ACCOUNT_FIELD_UNKNOWN,
        "KuCoin account snapshot uses Classic Futures private read scope; UTA wallet scope requires separate UTA evidence",
    )
    .with_source(row.source.clone());
    problem.details = Some(serde_json::json!({
        "venue": row.venue.as_str(),
        "operation": row.operation.as_str(),
        "field": "classicFuturesPrivateReadScope",
        "source": row.source.as_str(),
        "message": row.message.as_str(),
        "classicFuturesEndpoint": "/api/v1/account-overview",
        "utaAccountOverviewEndpoint": "/api/ua/v1/unified/account/overview",
        "utaCurrencyAssetsEndpoint": "/api/ua/v1/unified/account/balance",
        "utaProductionBoundary": "KuCoin official docs mark UTA API under active development and not for production/live trading",
        "observedAtMs": observed_at_ms,
    }));
    problem
}

pub(super) fn account_state_status(
    balance_status: ListStatus,
    position_status: ListStatus,
    open_order_status: ListStatus,
    problems: &[ApiProblem],
    operation_health: &[VenueOperationHealth],
    field_quality: &[AccountFieldQuality],
) -> ListStatus {
    if balance_status == ListStatus::Degraded
        || position_status == ListStatus::Degraded
        || open_order_status == ListStatus::Degraded
        || !problems.is_empty()
        || operation_health
            .iter()
            .any(account_quality::account_data_operation_degrades_snapshot)
        || account_quality::account_fields_degrade_snapshot(field_quality)
    {
        ListStatus::Degraded
    } else {
        ListStatus::Fresh
    }
}

pub(super) fn is_unknown_equity_quality(row: &AccountFieldQuality) -> bool {
    row.field == EQUITY_FIELD && row.status == AccountFieldQualityStatus::Unknown
}
