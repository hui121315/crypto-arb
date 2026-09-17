use super::*;

#[test]
fn selector_requires_side_when_rows_are_ambiguous() {
    let rows = vec![position(PositionSide::Long), position(PositionSide::Short)];
    let ambiguous = select_position(&rows, "binance", "MUUSDT", None);
    assert!(matches!(ambiguous, Err(AppError::BadRequest(_))));

    let selected =
        select_position(&rows, "binance", "MUUSDT", Some(PositionSide::Short)).map(|row| row.side);
    assert!(matches!(selected, Ok(PositionSide::Short)));
}

#[test]
fn selector_returns_paired_position_rows() -> Result<(), AppError> {
    let mut long = position(PositionSide::Long);
    let mut short = position(PositionSide::Short);
    short.venue = "OKX".into();
    long.paired_with = Some("OKX@MUUSDT".into());
    short.paired_with = Some("Binance@MUUSDT".into());
    long.pair_evidence = Some(pair_evidence(&long, &short));
    short.pair_evidence = Some(pair_evidence(&short, &long));
    let rows = vec![long, short];

    let (selected, pair) =
        select_pair_positions(&rows, "binance", "MUUSDT", Some(PositionSide::Long))?;

    assert_eq!(selected.venue, "Binance");
    assert_eq!(pair.venue, "OKX");
    assert_eq!(pair.side, PositionSide::Short);
    Ok(())
}

#[test]
fn selector_rejects_legacy_pair_string_without_evidence() {
    let mut long = position(PositionSide::Long);
    let mut short = position(PositionSide::Short);
    short.venue = "OKX".into();
    long.paired_with = Some("OKX@MUUSDT".into());
    short.paired_with = Some("Binance@MUUSDT".into());
    let rows = vec![long, short];

    let result = select_pair_positions(&rows, "binance", "MUUSDT", Some(PositionSide::Long));

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[test]
fn selector_normalizes_builder_venue_case() -> Result<(), AppError> {
    let mut row = position(PositionSide::Long);
    row.venue = "Hyperliquid:XYZ".into();
    let rows = vec![row];

    let selected = select_position(
        &rows,
        " hyperliquid:xyz ",
        "MUUSDT",
        Some(PositionSide::Long),
    )?;

    assert_eq!(selected.venue, "Hyperliquid:XYZ");
    Ok(())
}

#[test]
fn selector_accepts_native_symbol_for_canonical_position() -> Result<(), AppError> {
    let mut row = position(PositionSide::Long);
    row.symbol = "MU".into();
    let rows = vec![row];

    let selected = select_position(&rows, "binance", "MUUSDT", Some(PositionSide::Long))?;

    assert_eq!(selected.symbol, "MU");
    Ok(())
}

#[test]
fn selector_rejects_unpaired_position_rows() {
    let rows = vec![position(PositionSide::Long)];
    let result = select_pair_positions(&rows, "binance", "MUUSDT", Some(PositionSide::Long));

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}
