use super::{
    normalized_venue_name, VenueOperationHealth, VenueOperationKind, VenueOperationStatus,
    VenueRuntimeHealth, VenueRuntimeOperation, VenueRuntimeOperationHealth,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueRuntimeHealthSnapshot {
    pub venues: Vec<VenueRuntimeHealth>,
    pub generated_at_ms: i64,
    pub venue_count: usize,
    pub operation_count: usize,
    pub currently_usable_count: usize,
    pub attention_count: usize,
}

impl VenueRuntimeHealthSnapshot {
    pub fn from_operation_rows(rows: &[VenueOperationHealth], generated_at_ms: i64) -> Self {
        let mut grouped = BTreeMap::<String, VenueCandidates>::new();
        for row in rows {
            let venue = normalized_venue_name(&row.venue);
            if venue.is_empty() {
                continue;
            }
            for operation in mapped_operations(&row.operation).into_iter().flatten() {
                grouped
                    .entry(venue.clone())
                    .or_default()
                    .insert(operation, row);
            }
        }

        let venues = grouped
            .into_iter()
            .map(|(venue, candidates)| candidates.project(venue, generated_at_ms))
            .collect::<Vec<_>>();
        let operation_count = venues.iter().map(|venue| venue.operations().count()).sum();
        let currently_usable_count = venues
            .iter()
            .flat_map(|venue| venue.operations())
            .filter(|operation| operation.currently_usable)
            .count();
        let attention_count = operation_count - currently_usable_count;
        Self {
            venue_count: venues.len(),
            venues,
            generated_at_ms,
            operation_count,
            currently_usable_count,
            attention_count,
        }
    }
}

#[derive(Default)]
struct VenueCandidates {
    by_operation: BTreeMap<(VenueRuntimeOperation, String), VenueOperationHealth>,
}

impl VenueCandidates {
    fn insert(&mut self, operation: VenueRuntimeOperation, row: &VenueOperationHealth) {
        let key = (operation, row.operation.trim().to_ascii_lowercase());
        match self.by_operation.get_mut(&key) {
            Some(current) if duplicate_candidate_cmp(row, current).is_gt() => {
                *current = row.clone();
            }
            Some(_) => {}
            None => {
                self.by_operation.insert(key, row.clone());
            }
        }
    }

    fn project(self, venue: String, generated_at_ms: i64) -> VenueRuntimeHealth {
        let mut selected = BTreeMap::<VenueRuntimeOperation, VenueOperationHealth>::new();
        for ((operation, _), row) in self.by_operation {
            match selected.get_mut(&operation) {
                Some(current) if slot_candidate_cmp(&row, current, operation).is_gt() => {
                    *current = row;
                }
                Some(_) => {}
                None => {
                    selected.insert(operation, row);
                }
            }
        }

        let mut health = VenueRuntimeHealth::new(venue, generated_at_ms);
        for (operation, row) in selected {
            health.set_operation(VenueRuntimeOperationHealth::from_operation_health(
                operation, &row,
            ));
        }
        health
    }
}

fn mapped_operations(operation: &str) -> [Option<VenueRuntimeOperation>; 2] {
    use VenueOperationKind as Kind;
    use VenueRuntimeOperation as Runtime;

    match Kind::parse(operation) {
        Kind::RestOrderbooks
        | Kind::RestFundingRates
        | Kind::RestIndexCompositions
        | Kind::RestInstrumentSpecs
        | Kind::RestMetadata
        | Kind::RestPerpTickers
        | Kind::RestSpotTicks
        | Kind::RestFundingFallback
        | Kind::RestTickerFallback => [Some(Runtime::PublicRest), None],
        Kind::WsFunding
        | Kind::WsFundingSubscribe
        | Kind::WsFundingSnapshot
        | Kind::WsTicker
        | Kind::WsTickerSubscribe
        | Kind::WsTickerSnapshot
        | Kind::WsSpotSnapshot => [Some(Runtime::PublicWs), None],
        Kind::PrivateRead | Kind::CredentialProbeAccountModeRead => {
            [Some(Runtime::PrivateRest), None]
        }
        Kind::PrivateWsSession | Kind::PrivateWsSubscribe | Kind::PrivateWsAccountStream => {
            [Some(Runtime::PrivateWs), None]
        }
        Kind::Balance | Kind::CredentialProbeBalanceRead => [Some(Runtime::Balance), None],
        Kind::Positions | Kind::CredentialProbePositionsRead => [Some(Runtime::Positions), None],
        Kind::CredentialProbeOpenOrdersRead => [Some(Runtime::OpenOrders), None],
        Kind::OrderWrite => [Some(Runtime::PlaceOrder), Some(Runtime::CancelOrder)],
        Kind::PrivateWsOrderStream => [Some(Runtime::OrderStream), None],
        Kind::OrderFinality | Kind::OrderReconciliation => [Some(Runtime::Finality), None],
        Kind::CredentialProbeOrderPermission
        | Kind::AppWsBroadcast
        | Kind::HttpRest
        | Kind::HostGate
        | Kind::RateLimiter
        | Kind::OpportunitySnapshot
        | Kind::WatchlistPrewarm
        | Kind::BackgroundTasks
        | Kind::BackgroundTask
        | Kind::StorageAuditLog
        | Kind::StorageHistory
        | Kind::StoragePortfolioNav
        | Kind::StorageExecutionLedger
        | Kind::StorageOrderSnapshot
        | Kind::StorageTradingSqlMigrations
        | Kind::StorageTradingSqlLedger
        | Kind::StorageWatchlistAlerts
        | Kind::Unknown => [None, None],
    }
}

fn duplicate_candidate_cmp(
    candidate: &VenueOperationHealth,
    current: &VenueOperationHealth,
) -> Ordering {
    candidate
        .observed_at_ms
        .cmp(&current.observed_at_ms)
        .then_with(|| status_severity(candidate.status).cmp(&status_severity(current.status)))
        .then_with(|| evidence_rank(candidate).cmp(&evidence_rank(current)))
        .then_with(|| deterministic_row_cmp(candidate, current))
}

fn slot_candidate_cmp(
    candidate: &VenueOperationHealth,
    current: &VenueOperationHealth,
    operation: VenueRuntimeOperation,
) -> Ordering {
    status_severity(candidate.status)
        .cmp(&status_severity(current.status))
        .then_with(|| {
            let candidate =
                VenueRuntimeOperationHealth::from_operation_health(operation, candidate);
            let current = VenueRuntimeOperationHealth::from_operation_health(operation, current);
            (!candidate.currently_usable).cmp(&(!current.currently_usable))
        })
        .then_with(|| candidate.observed_at_ms.cmp(&current.observed_at_ms))
        .then_with(|| evidence_rank(candidate).cmp(&evidence_rank(current)))
        .then_with(|| deterministic_row_cmp(candidate, current))
}

const fn status_severity(status: VenueOperationStatus) -> u8 {
    match status {
        VenueOperationStatus::Ok => 0,
        VenueOperationStatus::Warn => 1,
        VenueOperationStatus::Unknown => 2,
        VenueOperationStatus::Unsupported => 3,
        VenueOperationStatus::Blocked => 4,
    }
}

fn evidence_rank(row: &VenueOperationHealth) -> u8 {
    u8::from(row.evidence.is_some())
        + u8::from(row.error.is_some())
        + u8::from(row.problem.is_some()).saturating_mul(2)
        + u8::from(
            row.evidence
                .as_ref()
                .and_then(|evidence| evidence.request_id.as_ref())
                .or_else(|| {
                    row.problem
                        .as_ref()
                        .and_then(|problem| problem.request_id.as_ref())
                })
                .is_some(),
        )
}

fn deterministic_row_cmp(
    candidate: &VenueOperationHealth,
    current: &VenueOperationHealth,
) -> Ordering {
    current
        .operation
        .cmp(&candidate.operation)
        .then_with(|| current.source.cmp(&candidate.source))
        .then_with(|| current.message.cmp(&candidate.message))
}
