use super::*;

pub(super) fn account_mode_blockers(checks: &[AccountModeCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter_map(account_mode_blocker)
        .fold(Vec::new(), push_unique)
}

pub(super) fn account_mode_blocker(check: &AccountModeCheck<'_>) -> Option<String> {
    match check.account_mode.as_ref() {
        Ok(Some(info)) if account_mode_is_unconfirmed(&info.mode) => Some(format!(
            "{} {} 账户模式未确认: {}",
            check.plan.exchange,
            check.plan.symbol,
            unconfirmed_mode_label(&info.mode)
        )),
        Ok(Some(_)) => None,
        Ok(None) => Some(format!(
            "{} {} 未暴露账户模式证据",
            check.plan.exchange, check.plan.symbol
        )),
        Err(error) => Some(format!(
            "{} {} 账户模式不可读: {error}",
            check.plan.exchange, check.plan.symbol
        )),
    }
}

/// Hedge/position mode 必须是已确认的具体模式；空值或 unknown/unconfirmed 等
/// 占位语义按 fail-closed 阻断，避免在仓位方向未确认时下单（PR-B Hedge Mode guard）。
pub(super) fn account_mode_is_unconfirmed(mode: &str) -> bool {
    let normalized = mode.trim().to_ascii_lowercase();
    normalized.is_empty()
        || matches!(
            normalized.as_str(),
            "unknown" | "unconfirmed" | "unspecified" | "unset" | "none" | "null"
        )
}

pub(super) fn unconfirmed_mode_label(mode: &str) -> String {
    let trimmed = mode.trim();
    if trimmed.is_empty() {
        "<empty>".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(super) fn account_mode_scope(checks: &[AccountModeCheck<'_>]) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: account_mode_venues(checks),
        symbols: account_mode_symbols(checks),
        account_modes: account_mode_labels(checks),
        operations: vec![HedgePreflightOperation::AccountMode],
    }
}

pub(super) fn account_mode_venues(checks: &[AccountModeCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .map(|check| normalized_venue_name(&check.plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn observed_account_mode_venues(checks: &[AccountModeCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter_map(successful_account_mode)
        .map(|info| normalized_venue_name(&info.venue))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn account_mode_symbols(checks: &[AccountModeCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .map(|check| check.plan.symbol.clone())
        .fold(Vec::new(), push_unique)
}

pub(super) fn account_mode_labels(checks: &[AccountModeCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter_map(successful_account_mode)
        .map(account_mode_label)
        .fold(Vec::new(), push_unique)
}

fn account_mode_label(info: &VenueAccountModeInfo) -> String {
    let kind = "position_mode";
    let base = format!(
        "{}_{kind}:{}",
        normalized_venue_name(&info.venue),
        info.mode
    );
    match info.account_scope.as_deref() {
        Some(scope) if !scope.is_empty() => format!("{base}·{scope}"),
        _ => base,
    }
}

pub(super) fn max_account_mode_freshness(checks: &[AccountModeCheck<'_>]) -> Option<u64> {
    checks
        .iter()
        .filter_map(successful_account_mode)
        .filter_map(|info| info.freshness_ms)
        .max()
}

fn successful_account_mode<'a>(
    check: &'a AccountModeCheck<'_>,
) -> Option<&'a VenueAccountModeInfo> {
    match &check.account_mode {
        Ok(info) => info.as_ref(),
        Err(_) => None,
    }
}
