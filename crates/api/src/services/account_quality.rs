use shared_types::{
    AccountFieldQuality, AccountFieldQualityStatus, VenueOperationHealth, VenueOperationStatus,
    OP_BALANCE, OP_POSITIONS,
};

const OP_OPEN_ORDERS: &str = "open_orders";

pub(crate) fn account_fields_degrade_snapshot(rows: &[AccountFieldQuality]) -> bool {
    rows.iter().any(account_field_degrades_snapshot)
}

fn account_field_degrades_snapshot(row: &AccountFieldQuality) -> bool {
    if row.status == AccountFieldQualityStatus::Actual {
        return false;
    }
    if row.status == AccountFieldQualityStatus::Invalid {
        return true;
    }
    !is_row_level_diagnostic_field(&row.field)
}

fn is_row_level_diagnostic_field(field: &str) -> bool {
    matches!(
        field,
        "withdrawableBalance"
            | "classicFuturesPrivateReadScope"
            | "liquidationPrice"
            | "liquidationDistancePct"
            | "maintenanceMarginRatio"
            | "margin"
            | "positionMode"
            | "marginMode"
            | "riskRate"
            | "availablePosition"
            | "frozenPosition"
            | "fundingRate8h"
            | "nextFundingMs"
    )
}

pub(crate) fn account_data_operation_degrades_snapshot(row: &VenueOperationHealth) -> bool {
    row.configured != Some(false)
        && matches!(
            row.status,
            VenueOperationStatus::Warn
                | VenueOperationStatus::Blocked
                | VenueOperationStatus::Unknown
        )
        && matches!(
            row.operation.as_str(),
            OP_BALANCE | OP_POSITIONS | OP_OPEN_ORDERS
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{AccountFieldSubject, VenueOperationHealth};

    fn quality(field: &str, status: AccountFieldQualityStatus) -> AccountFieldQuality {
        AccountFieldQuality::new(
            AccountFieldSubject::position("binance", "SOL", "long"),
            field,
            status,
            "test",
            Some(1),
        )
    }

    fn health(operation: &str, status: VenueOperationStatus) -> VenueOperationHealth {
        VenueOperationHealth {
            venue: "binance".to_owned(),
            operation: operation.to_owned(),
            status,
            source: "test".to_owned(),
            message: "test".to_owned(),
            supported: Some(true),
            configured: Some(true),
            requested: None,
            rows: None,
            freshness_ms: None,
            retry_after_ms: None,
            latency_ms: None,
            latency_p95_ms: None,
            error: None,
            evidence: None,
            problem: None,
            observed_at_ms: 1,
        }
    }

    #[test]
    fn row_level_missing_and_estimated_fields_do_not_degrade_snapshot() {
        let rows = [
            quality("maintenanceMarginRatio", AccountFieldQualityStatus::Unknown),
            quality("margin", AccountFieldQualityStatus::Estimated),
            quality("nextFundingMs", AccountFieldQualityStatus::Missing),
        ];

        assert!(!account_fields_degrade_snapshot(&rows));
    }

    #[test]
    fn invalid_or_core_fields_still_degrade_snapshot() {
        assert!(account_fields_degrade_snapshot(&[quality(
            "maintenanceMarginRatio",
            AccountFieldQualityStatus::Invalid,
        )]));
        assert!(account_fields_degrade_snapshot(&[quality(
            "markPrice",
            AccountFieldQualityStatus::Estimated,
        )]));
    }

    #[test]
    fn only_configured_current_data_operations_degrade_snapshot() {
        assert!(account_data_operation_degrades_snapshot(&health(
            OP_POSITIONS,
            VenueOperationStatus::Warn,
        )));
        assert!(!account_data_operation_degrades_snapshot(&health(
            "private_ws_order_stream",
            VenueOperationStatus::Warn,
        )));
        let mut unconfigured = health(OP_POSITIONS, VenueOperationStatus::Blocked);
        unconfigured.configured = Some(false);
        assert!(!account_data_operation_degrades_snapshot(&unconfigured));
    }
}
