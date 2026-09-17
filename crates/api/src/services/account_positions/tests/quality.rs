use super::*;

#[test]
fn position_field_quality_marks_missing_risk_fields() {
    let rows = vec![PositionInfo {
        margin: 0.0,
        ..position_row()
    }];
    let quality = position_field_quality(&rows, 10);

    assert!(quality.iter().any(|row| {
        row.field == "liquidationPrice" && row.status == AccountFieldQualityStatus::Missing
    }));
    assert!(quality.iter().any(|row| {
        row.field == "liquidationDistancePct" && row.status == AccountFieldQualityStatus::Missing
    }));
    assert!(quality.iter().any(|row| {
        row.field == "maintenanceMarginRatio" && row.status == AccountFieldQualityStatus::Unknown
    }));
    assert!(quality.iter().any(|row| {
        row.field == "margin"
            && row.status == AccountFieldQualityStatus::Estimated
            && row.source == "position_notional_over_leverage_estimate"
    }));
    assert!(quality.iter().all(|row| {
        row.problem
            .as_ref()
            .is_some_and(|problem| problem.code == codes::POSITION_FIELD_UNAVAILABLE)
    }));
    assert_eq!(position_status(&[], &[], &quality), ListStatus::Fresh);
}

#[test]
fn position_field_quality_marks_price_and_margin_fallbacks_as_estimated() {
    let row = PositionInfo {
        mark_price: 0.0,
        entry_price: 10.0,
        margin: 0.0,
        leverage: 2.0,
        ..position_row()
    };
    let quality = position_field_quality(&[row], 10);

    assert!(quality.iter().any(|item| {
        item.field == "markPrice"
            && item.status == AccountFieldQualityStatus::Estimated
            && item.source == "position_entry_price_fallback"
    }));
    assert!(quality.iter().any(|item| {
        item.field == "margin"
            && item.status == AccountFieldQualityStatus::Estimated
            && item.source == "position_notional_over_leverage_estimate"
    }));
    assert_eq!(position_status(&[], &[], &quality), ListStatus::Degraded);
}

#[test]
fn liquidation_distance_quality_distinguishes_exchange_estimate_and_unavailable() {
    let rows = vec![
        PositionInfo {
            exchange: "Binance".to_owned(),
            liquidation_price: Some(8.0),
            liquidation_distance_pct: Some(20.0),
            ..position_row()
        },
        PositionInfo {
            exchange: "OKX".to_owned(),
            liquidation_price: Some(8.0),
            liquidation_distance_pct: None,
            ..position_row()
        },
        PositionInfo {
            exchange: "Bybit".to_owned(),
            liquidation_price: None,
            liquidation_distance_pct: None,
            ..position_row()
        },
    ];
    let quality = position_field_quality(&rows, 10);

    assert!(quality.iter().any(|item| {
        item.subject.venue.as_deref() == Some("Binance")
            && item.field == "liquidationDistancePct"
            && item.status == AccountFieldQualityStatus::Actual
            && item.source == "binance_position_liquidation_distance"
    }));
    assert!(quality.iter().any(|item| {
        item.subject.venue.as_deref() == Some("OKX")
            && item.field == "liquidationDistancePct"
            && item.status == AccountFieldQualityStatus::Estimated
            && item.source == "position_liquidation_price_derived_distance"
    }));
    assert!(quality.iter().any(|item| {
        item.subject.venue.as_deref() == Some("Bybit")
            && item.field == "liquidationDistancePct"
            && item.status == AccountFieldQualityStatus::Missing
    }));
}

#[test]
fn crossed_liquidation_distance_remains_actual_risk_evidence() {
    let row = PositionInfo {
        exchange: "Binance".to_owned(),
        liquidation_price: Some(12.0),
        liquidation_distance_pct: Some(-1.0),
        ..position_row()
    };

    let quality = position_field_quality(&[row], 10);

    assert!(quality.iter().any(|item| {
        item.field == "liquidationDistancePct"
            && item.status == AccountFieldQualityStatus::Actual
            && item.source == "binance_position_liquidation_distance"
    }));
}

#[test]
fn binance_documented_zero_liquidation_price_is_verified_not_missing() {
    let row = PositionInfo {
        exchange: "Binance".to_owned(),
        liquidation_price: Some(0.0),
        liquidation_distance_pct: None,
        ..position_row()
    };

    let quality = position_field_quality(&[row], 10);

    assert!(quality.iter().any(|item| {
        item.field == "liquidationPrice"
            && item.status == AccountFieldQualityStatus::Actual
            && item.source == "binance_position_liquidation_price_zero"
            && item.problem.is_none()
    }));
    assert!(quality.iter().any(|item| {
        item.field == "liquidationDistancePct"
            && item.status == AccountFieldQualityStatus::Actual
            && item.source == "binance_position_liquidation_price_zero_no_distance"
            && item.problem.is_none()
    }));
}
