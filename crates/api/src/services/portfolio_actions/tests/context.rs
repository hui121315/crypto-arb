use super::*;

#[test]
fn close_request_context_requires_snapshot_version() {
    let context = CloseRequestContext::new(None, Some(1), Some(" close ".to_owned()));

    assert!(
        matches!(context, Err(AppError::Domain { code, .. }) if code == shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID)
    );
}

#[test]
fn validate_context_rejects_stale_snapshot() {
    let rows = vec![position(PositionSide::Long)];
    let context = close_context_for_test(1);

    let result = validate_close_context(&rows, 1, context);

    assert!(
        matches!(result, Err(AppError::Domain { code, .. }) if code == shared_types::problem::codes::CLOSE_RUN_STALE_SNAPSHOT)
    );
}

#[test]
fn validate_context_rejects_leg_count_mismatch() -> Result<(), AppError> {
    let rows = vec![position(PositionSide::Long)];
    let version = portfolio::positions_version(&rows);
    let context = CloseRequestContext::new(Some(version), Some(2), None)?;

    let result = validate_close_context(&rows, 1, context);

    assert!(
        matches!(result, Err(AppError::Domain { code, .. }) if code == shared_types::problem::codes::CLOSE_RUN_EXPECTED_LEG_MISMATCH)
    );
    Ok(())
}

#[test]
fn validate_context_trims_reason() -> Result<(), AppError> {
    let rows = vec![position(PositionSide::Long)];
    let version = portfolio::positions_version(&rows);
    let context = CloseRequestContext::new(Some(version), Some(1), Some(" close ".to_owned()))?;

    let validated = validate_close_context(&rows, 1, context)?;

    assert_eq!(validated.reason.as_deref(), Some("close"));
    Ok(())
}

#[test]
fn close_order_key_for_context_is_stable_for_same_action_key() {
    let row = position(PositionSide::Long);
    let context = close_context_with_key("portfolio-close:single:binance:MUUSDT:long:pos-1:1");

    let first = close_order_key_for_context(&row, &context, 0);
    let second = close_order_key_for_context(&row, &context, 0);

    assert_eq!(first, second);
    assert!(first.len() <= 36);
}

#[test]
fn close_order_key_for_context_changes_by_action_key_and_leg() {
    let row = position(PositionSide::Long);
    let first_context =
        close_context_with_key("portfolio-close:single:binance:MUUSDT:long:pos-1:1");
    let second_context =
        close_context_with_key("portfolio-close:single:binance:MUUSDT:long:pos-2:1");

    let first = close_order_key_for_context(&row, &first_context, 0);
    let second = close_order_key_for_context(&row, &second_context, 0);
    let second_leg = close_order_key_for_context(&row, &first_context, 1);

    assert_ne!(first, second);
    assert_ne!(first, second_leg);
}

#[test]
fn close_order_key_for_context_changes_for_same_header_across_close_scopes() {
    let row = position(PositionSide::Long);
    let header_key = "client-close-key";
    let single = close_context_with_key_and_scope(header_key, CloseRunScope::Single);
    let pair = close_context_with_key_and_scope(header_key, CloseRunScope::Pair);
    let all = close_context_with_key_and_scope(header_key, CloseRunScope::All);

    let single_key = close_order_key_for_context(&row, &single, 0);
    let pair_key = close_order_key_for_context(&row, &pair, 0);
    let all_key = close_order_key_for_context(&row, &all, 0);

    assert_ne!(single_key, pair_key);
    assert_ne!(single_key, all_key);
    assert_ne!(pair_key, all_key);
}
