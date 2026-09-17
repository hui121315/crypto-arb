use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubject,
    AccountFieldSubjectKind, VenueAssetValuation, VenueBalanceInfo, VenueOperationHealth,
    VenueOperationStatus,
};

pub(super) fn health_row(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status,
        source: "account_balance_runtime".to_owned(),
        message: "sample".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: None,
        rows: Some(1),
        freshness_ms: Some(250),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1,
    }
}

pub(super) fn account_quality(
    kind: AccountFieldSubjectKind,
    venue: &str,
    currency: Option<&str>,
    field: &str,
    status: AccountFieldQualityStatus,
) -> AccountFieldQuality {
    let subject = match kind {
        AccountFieldSubjectKind::Account => AccountFieldSubject::account(venue),
        AccountFieldSubjectKind::Balance => {
            AccountFieldSubject::balance(venue, currency.unwrap_or("USDT"))
        }
        AccountFieldSubjectKind::Position => {
            AccountFieldSubject::position(venue, "BTCUSDT", "long")
        }
        AccountFieldSubjectKind::OpenOrder => {
            AccountFieldSubject::open_order(venue, "o-1", "BTCUSDT", "buy")
        }
    };
    AccountFieldQuality::new(subject, field, status, "account_state_runtime", Some(1))
}

pub(super) fn account_data_health(venue: &str, currency: &str, source: &str) -> AccountDataHealth {
    let mut health =
        AccountDataHealth::new(AccountFieldSubject::balance(venue, currency), source, 1_000);
    health.freshness_ms = Some(500);
    health.last_success_ms = Some(500);
    health
}

pub(super) fn balance(venue: &str, currency: &str, total: f64) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: venue.to_owned(),
        currency: currency.to_owned(),
        total,
        available: total,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }
}

pub(super) fn valuation(venue: &str, currency: &str, usd_value: f64) -> VenueAssetValuation {
    VenueAssetValuation {
        venue: venue.to_owned(),
        currency: currency.to_owned(),
        usd_value,
        source: "official.account.usdValue".to_owned(),
        observed_at_ms: 1,
    }
}
