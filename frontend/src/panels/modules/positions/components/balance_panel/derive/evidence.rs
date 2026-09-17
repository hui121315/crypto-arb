use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubjectKind,
    VenueBalanceInfo, VenueOperationHealth, VenueOperationStatus,
};

pub(crate) fn balance_health_rows(rows: Vec<VenueOperationHealth>) -> Vec<VenueOperationHealth> {
    rows.into_iter()
        .filter(|row| {
            row.operation == "balance" || row.operation == "credential_probe:balance_read"
        })
        .collect()
}

pub(crate) fn balance_field_quality_rows(
    rows: Vec<AccountFieldQuality>,
) -> Vec<AccountFieldQuality> {
    rows.into_iter()
        .filter(|row| {
            matches!(
                row.subject.kind,
                AccountFieldSubjectKind::Account | AccountFieldSubjectKind::Balance
            )
        })
        .filter(|row| row.status != AccountFieldQualityStatus::Actual)
        .collect()
}

pub(crate) fn account_level_quality_rows(rows: &[AccountFieldQuality]) -> Vec<AccountFieldQuality> {
    rows.iter()
        .filter(|row| row.subject.kind == AccountFieldSubjectKind::Account)
        .cloned()
        .collect()
}

pub(crate) fn balance_quality_for_row(
    row: &VenueBalanceInfo,
    field_quality: &[AccountFieldQuality],
) -> Vec<AccountFieldQuality> {
    field_quality
        .iter()
        .filter(|quality| balance_quality_matches_row(quality, row))
        .cloned()
        .collect()
}

fn balance_quality_matches_row(quality: &AccountFieldQuality, row: &VenueBalanceInfo) -> bool {
    if quality.subject.kind != AccountFieldSubjectKind::Balance {
        return false;
    }
    let venue_match = quality
        .subject
        .venue
        .as_deref()
        .map(shared_types::normalized_venue_name)
        == Some(shared_types::normalized_venue_name(&row.venue));
    let currency_match = quality
        .subject
        .currency
        .as_deref()
        .map(|currency| currency.trim().to_ascii_uppercase())
        == Some(row.currency.trim().to_ascii_uppercase());
    venue_match && currency_match && quality.status != AccountFieldQualityStatus::Actual
}

pub(crate) fn balance_row_health_for_row(
    row: &VenueBalanceInfo,
    health: &[AccountDataHealth],
) -> Vec<AccountDataHealth> {
    health
        .iter()
        .filter(|item| balance_health_matches_row(item, row))
        .cloned()
        .collect()
}

fn balance_health_matches_row(health: &AccountDataHealth, row: &VenueBalanceInfo) -> bool {
    if health.subject.kind != AccountFieldSubjectKind::Balance {
        return false;
    }
    let venue_match = health
        .subject
        .venue
        .as_deref()
        .map(shared_types::normalized_venue_name)
        == Some(shared_types::normalized_venue_name(&row.venue));
    let currency_match = health
        .subject
        .currency
        .as_deref()
        .map(|currency| currency.trim().to_ascii_uppercase())
        == Some(row.currency.trim().to_ascii_uppercase());
    venue_match && currency_match
}

pub(crate) fn balance_evidence_title(row: &VenueOperationHealth) -> String {
    let mut parts = vec![
        row.venue.clone(),
        row.operation.clone(),
        balance_status_label(row.status).to_owned(),
        row.source.clone(),
        row.message.clone(),
    ];
    if let Some(freshness_ms) = row.freshness_ms {
        parts.push(format!("freshness {}", duration_label(freshness_ms)));
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        parts.push(format!("retry {}", duration_label(retry_after_ms as i64)));
    }
    if let Some(error) = row.error.as_deref() {
        parts.push(error.to_owned());
    }
    parts.join(" · ")
}

pub(crate) fn account_quality_title(row: &AccountFieldQuality) -> String {
    let mut parts = vec![
        account_quality_subject(row),
        row.field.clone(),
        account_quality_status_label(row.status).to_owned(),
        row.source.clone(),
    ];
    if let Some(problem) = row.problem.as_ref() {
        parts.push(problem.code.clone());
        parts.push(problem.message.clone());
        if let Some(request_id) = problem.request_id.as_ref() {
            parts.push(request_id.clone());
        }
    }
    parts.join(" · ")
}

pub(crate) fn balance_data_health_title(row: &AccountDataHealth) -> String {
    let mut parts = vec![account_data_health_subject(row), row.source.clone()];
    if let Some(freshness_ms) = row.freshness_ms {
        parts.push(format!("freshness {}", duration_label(freshness_ms)));
    }
    if let Some(last_success_ms) = row.last_success_ms {
        parts.push(format!("last success {last_success_ms}"));
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        parts.push(format!("retry {}", duration_label(retry_after_ms as i64)));
    }
    if let Some(request_id) = row.request_id.as_ref() {
        parts.push(format!("request {request_id}"));
    }
    if let Some(problem) = row.last_error.as_ref() {
        parts.push(problem.code.clone());
        parts.push(problem.message.clone());
    }
    parts.join(" · ")
}

pub(crate) fn account_quality_subject(row: &AccountFieldQuality) -> String {
    let venue = row.subject.venue.as_deref().unwrap_or("账户");
    match row.subject.kind {
        AccountFieldSubjectKind::Account => venue.to_owned(),
        AccountFieldSubjectKind::Balance => match row.subject.currency.as_deref() {
            Some(currency) => format!("{venue} {currency}"),
            None => venue.to_owned(),
        },
        AccountFieldSubjectKind::Position => match row.subject.symbol.as_deref() {
            Some(symbol) => format!("{venue} {symbol}"),
            None => venue.to_owned(),
        },
        AccountFieldSubjectKind::OpenOrder => match (
            row.subject.symbol.as_deref(),
            row.subject.order_id.as_deref(),
        ) {
            (Some(symbol), Some(order_id)) => format!("{venue} {symbol} {order_id}"),
            (Some(symbol), None) => format!("{venue} {symbol}"),
            _ => venue.to_owned(),
        },
    }
}

pub(crate) fn account_data_health_subject(row: &AccountDataHealth) -> String {
    let venue = row.subject.venue.as_deref().unwrap_or("账户");
    match row.subject.currency.as_deref() {
        Some(currency) => format!("{venue} {currency}"),
        None => venue.to_owned(),
    }
}

pub(crate) fn account_field_label(field: &str) -> String {
    match field {
        "equity" => "权益".to_owned(),
        "available" => "可用".to_owned(),
        "total" => "总额".to_owned(),
        "margin" => "保证金".to_owned(),
        "frozen" => "占用".to_owned(),
        "classicFuturesPrivateReadScope" => "Classic Futures 读法".to_owned(),
        _ => field.to_owned(),
    }
}

pub(crate) fn balance_operation_label(operation: &str) -> &'static str {
    match operation {
        "balance" => "余额缓存",
        "credential_probe:balance_read" => "凭证验证",
        _ => "余额证据",
    }
}

pub(crate) fn balance_status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "OK",
        VenueOperationStatus::Warn => "WARN",
        VenueOperationStatus::Blocked => "BLOCK",
        VenueOperationStatus::Unknown => "UNKNOWN",
        VenueOperationStatus::Unsupported => "UNSUPPORTED",
    }
}

pub(crate) fn account_quality_status_label(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "OK",
        AccountFieldQualityStatus::Estimated => "EST",
        AccountFieldQualityStatus::Unknown => "UNKNOWN",
        AccountFieldQualityStatus::Invalid => "INVALID",
        AccountFieldQualityStatus::Missing => "MISSING",
    }
}

pub(crate) fn balance_status_class(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "ok",
        VenueOperationStatus::Warn => "warn",
        VenueOperationStatus::Blocked => "blocked",
        VenueOperationStatus::Unknown => "unknown",
        VenueOperationStatus::Unsupported => "unsupported",
    }
}

pub(crate) fn account_quality_status_class(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "ok",
        AccountFieldQualityStatus::Estimated => "warn",
        AccountFieldQualityStatus::Unknown => "unknown",
        AccountFieldQualityStatus::Invalid | AccountFieldQualityStatus::Missing => "blocked",
    }
}

pub(crate) fn balance_data_health_class(row: &AccountDataHealth) -> &'static str {
    if row.last_error.is_some() {
        "blocked"
    } else if row.freshness_ms.is_some() {
        "ok"
    } else {
        "unknown"
    }
}

pub(crate) fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let secs = ms / 1_000;
    if secs < 60 {
        return format!("{secs}s");
    }
    format!("{}m", secs / 60)
}
