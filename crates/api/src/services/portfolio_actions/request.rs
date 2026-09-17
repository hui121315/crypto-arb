use super::*;

pub(super) fn required_snapshot_version(value: Option<String>) -> Result<String, AppError> {
    let Some(version) = value.map(|value| value.trim().to_owned()) else {
        return Err(close_request_error("close snapshot_version is required"));
    };
    if version.is_empty() {
        return Err(close_request_error("close snapshot_version is required"));
    }
    Ok(version)
}

pub(super) fn required_expected_leg_count(value: Option<usize>) -> Result<usize, AppError> {
    value.ok_or_else(|| close_request_error("close expected_leg_count is required"))
}

pub(super) fn normalize_reason(value: Option<String>) -> Result<Option<String>, AppError> {
    let Some(reason) = value.map(|value| value.trim().to_owned()) else {
        return Ok(None);
    };
    if reason.is_empty() {
        return Ok(None);
    }
    if reason.chars().count() > CLOSE_REASON_MAX_CHARS {
        return Err(close_request_error("close reason is too long"));
    }
    Ok(Some(reason))
}

fn close_request_error(message: &'static str) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
        message,
    )
}

pub(super) fn validate_close_context(
    rows: &[PositionRow],
    actual_leg_count: usize,
    context: CloseRequestContext,
) -> Result<CloseRequestContext, AppError> {
    validate_snapshot_version(rows, &context.snapshot_version)?;
    validate_expected_leg_count(context.expected_leg_count, actual_leg_count)?;
    Ok(context)
}

fn validate_snapshot_version(rows: &[PositionRow], expected: &str) -> Result<(), AppError> {
    let actual = portfolio::positions_version(rows);
    if actual == expected {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::CONFLICT,
        shared_types::problem::codes::CLOSE_RUN_STALE_SNAPSHOT,
        "portfolio snapshot changed; refresh before closing positions",
    )
    .with_details(json!({
        "expectedSnapshotVersion": expected,
        "currentSnapshotVersion": actual,
    })))
}

fn validate_expected_leg_count(expected: usize, actual: usize) -> Result<(), AppError> {
    if expected == actual {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::CONFLICT,
        shared_types::problem::codes::CLOSE_RUN_EXPECTED_LEG_MISMATCH,
        "close request expected leg count does not match current positions",
    )
    .with_details(json!({
        "expectedLegCount": expected,
        "actualLegCount": actual,
    })))
}

pub(super) fn select_position<'a>(
    rows: &'a [PositionRow],
    venue: &str,
    symbol: &str,
    side: Option<PositionSide>,
) -> Result<&'a PositionRow, AppError> {
    if let Some(row) = unique_position_match(
        rows.iter()
            .filter(|row| position_exact_matches(row, venue, symbol, side)),
    )? {
        return Ok(row);
    }
    let row = unique_position_match(
        rows.iter()
            .filter(|row| position_matches(row, venue, symbol, side)),
    )?;
    let Some(row) = row else {
        return Err(AppError::NotFound(format!("position: {venue}/{symbol}")));
    };
    Ok(row)
}

fn unique_position_match<'a>(
    mut matches: impl Iterator<Item = &'a PositionRow>,
) -> Result<Option<&'a PositionRow>, AppError> {
    let row = matches.next();
    if row.is_some() && matches.next().is_some() {
        return Err(AppError::BadRequest(
            "position selector matches multiple rows; side is required".into(),
        ));
    }
    Ok(row)
}

pub(super) fn select_pair_positions<'a>(
    rows: &'a [PositionRow],
    venue: &str,
    symbol: &str,
    side: Option<PositionSide>,
) -> Result<(&'a PositionRow, &'a PositionRow), AppError> {
    let row = select_position(rows, venue, symbol, side)?;
    let pair = select_paired_position(rows, row)?;
    Ok((row, pair))
}

fn select_paired_position<'a>(
    rows: &'a [PositionRow],
    row: &PositionRow,
) -> Result<&'a PositionRow, AppError> {
    let pair = row.pair_evidence.as_ref().ok_or_else(|| {
        AppError::BadRequest(format!(
            "position has no execution-run pair evidence: {}/{}",
            row.venue, row.symbol
        ))
    })?;
    let paired = select_position(
        rows,
        &pair.partner_venue,
        &pair.partner_symbol,
        Some(pair.partner_side),
    )?;
    validate_reciprocal_pair(row, paired)?;
    Ok(paired)
}

fn validate_reciprocal_pair(row: &PositionRow, paired: &PositionRow) -> Result<(), AppError> {
    let Some(left) = row.pair_evidence.as_ref() else {
        return Err(AppError::BadRequest("missing pair evidence".into()));
    };
    let Some(right) = paired.pair_evidence.as_ref() else {
        return Err(AppError::BadRequest(
            "paired position has no reciprocal evidence".into(),
        ));
    };
    if left.run_id == right.run_id
        && shared_types::venue_names_equal(&right.partner_venue, &row.venue)
        && position_symbols_equal(&right.partner_symbol, &row.symbol)
        && right.partner_side == row.side
    {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "paired position evidence is not reciprocal".into(),
        ))
    }
}

fn position_matches(
    row: &PositionRow,
    venue: &str,
    symbol: &str,
    side: Option<PositionSide>,
) -> bool {
    shared_types::venue_names_equal(&row.venue, venue)
        && position_symbols_equal(&row.symbol, symbol)
        && side.is_none_or(|value| value == row.side)
}

fn position_exact_matches(
    row: &PositionRow,
    venue: &str,
    symbol: &str,
    side: Option<PositionSide>,
) -> bool {
    shared_types::venue_names_equal(&row.venue, venue)
        && row.symbol.trim().eq_ignore_ascii_case(symbol.trim())
        && side.is_none_or(|value| value == row.side)
}

fn position_symbols_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
        || exchange::strip_common_suffixes(left)
            .eq_ignore_ascii_case(&exchange::strip_common_suffixes(right))
}
