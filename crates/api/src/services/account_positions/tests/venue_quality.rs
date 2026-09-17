use super::*;

#[test]
fn position_field_quality_separates_real_and_estimated_maintenance_sources() {
    let rows = [
        ("Gate", 0.012),
        ("KuCoin", 0.011),
        ("Bitget", 0.01),
        ("Kraken", 0.0),
        ("Hyperliquid", 0.0),
    ]
    .into_iter()
    .map(|(exchange, maintenance_margin_ratio)| PositionInfo {
        exchange: exchange.to_owned(),
        maintenance_margin_ratio,
        ..position_row()
    })
    .collect::<Vec<_>>();
    let quality = position_field_quality(&rows, 10);
    let is_maintenance = |row: &AccountFieldQuality, venue: &str| {
        row.subject.venue.as_deref() == Some(venue) && row.field == "maintenanceMarginRatio"
    };

    for venue in ["KuCoin", "Bitget"] {
        assert!(!quality.iter().any(|row| is_maintenance(row, venue)));
    }
    assert!(quality.iter().any(|row| {
        is_maintenance(row, "Gate")
            && row.status == AccountFieldQualityStatus::Estimated
            && row.source == "gate_ws_legacy_maintenance_rate"
    }));
    for venue in ["Kraken", "Hyperliquid"] {
        assert!(quality.iter().any(|row| {
            is_maintenance(row, venue) && row.status == AccountFieldQualityStatus::Unknown
        }));
    }
}

#[test]
fn gate_maintenance_quality_tracks_actual_estimated_and_unknown_provenance() {
    let rows = vec![
        PositionInfo {
            exchange: "Gate".to_owned(),
            maintenance_margin_ratio: 0.012,
            margin_mode: Some("isolated".to_owned()),
            ..position_row()
        },
        PositionInfo {
            exchange: "Gate".to_owned(),
            maintenance_margin_ratio: 0.011,
            margin_mode: None,
            ..position_row()
        },
        PositionInfo {
            exchange: "Gate".to_owned(),
            maintenance_margin_ratio: 0.0,
            margin_mode: Some("cross".to_owned()),
            ..position_row()
        },
    ];
    let maintenance = position_field_quality(&rows, 10)
        .into_iter()
        .filter(|row| row.field == "maintenanceMarginRatio")
        .collect::<Vec<_>>();

    assert!(maintenance.iter().any(|row| {
        row.status == AccountFieldQualityStatus::Actual
            && row.source == "gate_rest_average_maintenance_rate"
            && row.problem.is_none()
    }));
    assert!(maintenance.iter().any(|row| {
        row.status == AccountFieldQualityStatus::Estimated
            && row.source == "gate_ws_legacy_maintenance_rate"
            && row.problem.is_some()
    }));
    assert!(maintenance.iter().any(|row| {
        row.status == AccountFieldQualityStatus::Unknown
            && row.source == "gate_maintenance_unknown"
            && row.problem.is_some()
    }));
}
