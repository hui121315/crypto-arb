use super::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct RuntimeHealthSelection {
    pub(super) rows: Vec<VenueOperationHealth>,
    pub(super) total: usize,
    pub(super) attention: usize,
    pub(super) trading_evidence: TradingRuntimeEvidence,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct TradingRuntimeEvidence {
    pub(super) order_permission: Option<VenueOperationHealth>,
    pub(super) order_write: Option<VenueOperationHealth>,
    pub(super) private_order_stream: Option<VenueOperationHealth>,
    pub(super) order_finality: Option<VenueOperationHealth>,
}

pub(super) fn selected_runtime_health_selection(
    snapshot: Option<VenueOperationHealthSnapshot>,
    venue_id: &str,
) -> RuntimeHealthSelection {
    snapshot
        .map(|snapshot| selected_runtime_health_rows(snapshot, venue_id))
        .unwrap_or_default()
}

pub(super) fn selected_runtime_health_rows(
    snapshot: VenueOperationHealthSnapshot,
    venue_id: &str,
) -> RuntimeHealthSelection {
    let mut rows = snapshot
        .rows
        .into_iter()
        .filter(|row| runtime_row_matches_selected_venue(&row.venue, venue_id))
        .collect::<Vec<_>>();
    rows.sort_by(runtime_health_order);
    let trading_evidence = TradingRuntimeEvidence::from_rows(&rows);
    let total = rows.len();
    let attention = rows.iter().filter(|row| !row.is_currently_usable()).count();
    RuntimeHealthSelection {
        rows,
        total,
        attention,
        trading_evidence,
    }
}

impl TradingRuntimeEvidence {
    fn from_rows(rows: &[VenueOperationHealth]) -> Self {
        Self {
            order_permission: runtime_row_for_kind(
                rows,
                VenueOperationKind::CredentialProbeOrderPermission,
            ),
            order_write: runtime_row_for_kind(rows, VenueOperationKind::OrderWrite),
            private_order_stream: runtime_row_for_kind(
                rows,
                VenueOperationKind::PrivateWsOrderStream,
            ),
            order_finality: runtime_row_for_kind(rows, VenueOperationKind::OrderFinality),
        }
    }
}

pub(super) fn trading_runtime_rows(
    evidence: &TradingRuntimeEvidence,
) -> [Option<&VenueOperationHealth>; 4] {
    [
        evidence.order_permission.as_ref(),
        evidence.order_write.as_ref(),
        evidence.private_order_stream.as_ref(),
        evidence.order_finality.as_ref(),
    ]
}

pub(super) fn trading_runtime_ready_count(evidence: &TradingRuntimeEvidence) -> usize {
    trading_runtime_rows(evidence)
        .into_iter()
        .flatten()
        .filter(|row| row.is_currently_usable())
        .count()
}

pub(super) fn trading_runtime_attention_count(evidence: &TradingRuntimeEvidence) -> usize {
    trading_runtime_rows(evidence)
        .len()
        .saturating_sub(trading_runtime_ready_count(evidence))
}

fn runtime_row_for_kind(
    rows: &[VenueOperationHealth],
    kind: VenueOperationKind,
) -> Option<VenueOperationHealth> {
    rows.iter()
        .find(|row| VenueOperationKind::parse(&row.operation) == kind)
        .cloned()
}

pub(super) fn runtime_selection_dataset_key(
    venue_id: &str,
    selection: &RuntimeHealthSelection,
) -> String {
    let mut key = format!(
        "venue={venue_id};rows={};attention={}",
        selection.total, selection.attention
    );
    for row in &selection.rows {
        key.push('|');
        key.push_str(&row.venue);
        key.push(':');
        key.push_str(&row.operation);
        key.push(':');
        key.push_str(operation_status_key(row.status));
        key.push(':');
        key.push_str(&row.source);
    }
    key
}

pub(super) fn operation_status_key(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "ok",
        VenueOperationStatus::Warn => "warn",
        VenueOperationStatus::Blocked => "blocked",
        VenueOperationStatus::Unknown => "unknown",
        VenueOperationStatus::Unsupported => "unsupported",
    }
}

pub(super) fn runtime_row_matches_selected_venue(row_venue: &str, venue_id: &str) -> bool {
    let selected = normalized_venue_name(venue_id);
    if selected.is_empty() {
        return false;
    }
    let row = normalized_venue_name(row_venue);
    if row == selected {
        return true;
    }
    if selected.contains(':') {
        return false;
    }
    normalized_venue_name(venue_family(row_venue)) == selected
}

pub(super) fn runtime_health_order(
    left: &VenueOperationHealth,
    right: &VenueOperationHealth,
) -> std::cmp::Ordering {
    operation_status_rank(right.status)
        .cmp(&operation_status_rank(left.status))
        .then_with(|| left.venue.cmp(&right.venue))
        .then_with(|| left.operation.cmp(&right.operation))
}

pub(super) fn operation_status_rank(status: VenueOperationStatus) -> u8 {
    match status {
        VenueOperationStatus::Blocked => 5,
        VenueOperationStatus::Warn => 4,
        VenueOperationStatus::Unknown => 3,
        VenueOperationStatus::Unsupported => 2,
        VenueOperationStatus::Ok => 1,
    }
}

pub(super) fn runtime_header_summary(
    venue_id: &str,
    total: usize,
    attention: usize,
    visible: usize,
    generated_at_ms: i64,
) -> String {
    if total == 0 {
        return format!("{venue_id} 暂无运行态记录 · 生成 {generated_at_ms}");
    }
    format!("{venue_id} · {attention}/{total} 需关注 · 显示 {visible} 条 · 生成 {generated_at_ms}")
}

pub(super) fn runtime_summary_status(total: usize, attention: usize) -> &'static str {
    match (total, attention) {
        (0, _) => "待证据",
        (_, 0) => "正常",
        _ => "需关注",
    }
}

pub(super) fn runtime_summary_class(total: usize, attention: usize) -> &'static str {
    match (total, attention) {
        (0, _) => "status-pill pending",
        (_, 0) => "status-pill ready",
        _ => "status-pill blocked",
    }
}

pub(super) fn selected_credential_status(
    response: Option<VenueCredentialsResponse>,
    venue_id: &str,
) -> Option<VenueCredentialStatus> {
    response?
        .venues
        .into_iter()
        .find(|venue| venue.venue == venue_id)
}

pub(super) fn selected_credential_fields(
    response: Option<VenueCredentialsResponse>,
    venue_id: &str,
) -> Vec<VenueCredentialField> {
    selected_credential_status(response, venue_id)
        .map(|row| row.fields)
        .unwrap_or_default()
}

pub(super) fn credential_fields_dataset_key(
    venue_id: &str,
    fields: &[VenueCredentialField],
) -> String {
    let mut key = format!("venue={venue_id};fields={}", fields.len());
    for field in fields {
        key.push('|');
        key.push_str(&field.key);
        key.push(':');
        key.push_str(bool_key(field.configured));
        key.push(':');
        key.push_str(bool_key(field.secret));
        key.push(':');
        key.push_str(bool_key(field.required));
        key.push(':');
        key.push_str(credential_field_source_key(field.source));
        key.push(':');
        key.push_str(&field.env_key);
    }
    key
}

pub(super) fn ws_venue_from_response(
    response: shared_types::ExchangeWsVenuesResponse,
    venue_id: &str,
) -> Option<ExchangeWsVenue> {
    response
        .venues
        .into_iter()
        .find(|venue| venue.venue == venue_id)
}

fn bool_key(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

fn credential_field_source_key(source: shared_types::VenueCredentialFieldSource) -> &'static str {
    match source {
        shared_types::VenueCredentialFieldSource::Missing => "missing",
        shared_types::VenueCredentialFieldSource::Environment => "environment",
        shared_types::VenueCredentialFieldSource::EnvFile => "env_file",
        shared_types::VenueCredentialFieldSource::Keychain => "keychain",
        shared_types::VenueCredentialFieldSource::Runtime => "runtime",
    }
}
