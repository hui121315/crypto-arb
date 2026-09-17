use super::super::*;
use super::fixtures::*;

#[test]
fn account_mode_guard_records_kucoin_scope() {
    let long = plan("kucoin", "BTCUSDTM", Vec::new());
    let short = plan("gate", "BTC_USDT", Vec::new());
    let checks = [
        AccountModeCheck::new(&long, Ok(Some(account_mode("kucoin", "hedge")))),
        AccountModeCheck::new(&short, Ok(Some(account_mode("gate", "one_way")))),
    ];

    let guard = account_mode_guard(&checks).unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Passed);
    assert_eq!(outcome.scope.venues, vec!["kucoin", "gate"]);
    assert_eq!(outcome.scope.symbols, vec!["BTCUSDTM", "BTC_USDT"]);
    assert_eq!(
        outcome.scope.account_modes,
        vec!["kucoin_position_mode:hedge", "gate_position_mode:one_way"]
    );
    assert_eq!(
        outcome.scope.operations,
        vec![HedgePreflightOperation::AccountMode]
    );
    assert_eq!(outcome.freshness_ms, Some(8));
}

#[test]
fn account_mode_label_includes_scope_when_present() {
    let long = plan("kucoin", "BTCUSDTM", Vec::new());
    let mode = VenueAccountModeInfo {
        venue: "kucoin".to_owned(),
        mode: "hedge".to_owned(),
        source: "test".to_owned(),
        checked_at_ms: 1,
        freshness_ms: Some(8),
        account_scope: Some("classic_futures".to_owned()),
    };
    let checks = [AccountModeCheck::new(&long, Ok(Some(mode)))];

    let guard = account_mode_guard(&checks).unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert_eq!(
        outcome.scope.account_modes,
        vec!["kucoin_position_mode:hedge·classic_futures"]
    );
}

#[test]
fn account_mode_guard_blocks_unreadable_mode() {
    let long = plan("kucoin", "BTCUSDTM", Vec::new());
    let check = AccountModeCheck::new(&long, Err("401 auth failed".to_owned()));

    let guard = account_mode_guard(&[check]).unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
    assert_eq!(outcome.observed_venues, Vec::<String>::new());
    assert!(outcome
        .error
        .unwrap_or_default()
        .contains("kucoin BTCUSDTM 账户模式不可读"));
}

#[test]
fn account_mode_guard_blocks_unconfirmed_mode() {
    let long = plan("bybit", "BTCUSDT", Vec::new());
    let check = AccountModeCheck::new(&long, Ok(Some(account_mode("bybit", "unknown"))));

    let guard = account_mode_guard(&[check]).unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
    assert!(outcome
        .error
        .unwrap_or_default()
        .contains("bybit BTCUSDT 账户模式未确认"));
}

#[test]
fn account_mode_guard_blocks_empty_mode() {
    let long = plan("kucoin", "BTCUSDTM", Vec::new());
    let check = AccountModeCheck::new(&long, Ok(Some(account_mode("kucoin", "  "))));

    let guard = account_mode_guard(&[check]).unwrap_or_else(missing_guard);

    assert!(!guard.passed);
    assert!(guard
        .preflight_outcome
        .unwrap_or_default()
        .error
        .unwrap_or_default()
        .contains("账户模式未确认"));
}

#[test]
fn account_mode_guard_skips_empty_checks() {
    assert!(account_mode_guard(&[]).is_none());
}
