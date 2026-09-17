use shared_types::{
    normalized_venue_name, venue_family, AccountBindingEvidence, AccountBindingStatus,
    AccountEquityScope, VenueAccountSummary,
};

const HYPERLIQUID: &str = "hyperliquid";
const HYPERLIQUID_SPOT: &str = "hyperliquid:spot";

pub(crate) fn equity_coverage_venue(
    venue: &str,
    bindings: &[AccountBindingEvidence],
    summaries: &[VenueAccountSummary],
) -> String {
    if hyperliquid_uses_consolidated_equity(venue, bindings, summaries) {
        HYPERLIQUID.to_owned()
    } else {
        normalized_venue_name(venue)
    }
}

pub(crate) fn summary_covers_nav(
    summary: &VenueAccountSummary,
    bindings: &[AccountBindingEvidence],
    summaries: &[VenueAccountSummary],
) -> bool {
    if !hyperliquid_uses_consolidated_equity(&summary.venue, bindings, summaries) {
        return true;
    }
    summary.equity_scope == AccountEquityScope::Unified
        || (normalized_venue_name(&summary.venue) == HYPERLIQUID_SPOT
            && summary.equity_scope == AccountEquityScope::Spot)
}

fn hyperliquid_uses_consolidated_equity(
    venue: &str,
    bindings: &[AccountBindingEvidence],
    summaries: &[VenueAccountSummary],
) -> bool {
    if normalized_venue_name(venue_family(venue)) != HYPERLIQUID {
        return false;
    }
    let verified_binding = bindings.iter().any(|binding| {
        binding.status == AccountBindingStatus::Verified
            && normalized_venue_name(venue_family(&binding.venue)) == HYPERLIQUID
            && binding
                .account_scope
                .as_deref()
                .is_some_and(is_consolidated_hyperliquid_scope)
    });
    verified_binding
        || summaries.iter().any(|summary| {
            normalized_venue_name(&summary.venue) == HYPERLIQUID_SPOT
                && is_consolidated_hyperliquid_scope(&summary.account_type)
        })
}

fn is_consolidated_hyperliquid_scope(scope: &str) -> bool {
    matches!(
        scope.trim().to_ascii_lowercase().as_str(),
        "unifiedaccount" | "portfoliomargin"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(scope: &str) -> AccountBindingEvidence {
        AccountBindingEvidence {
            venue: HYPERLIQUID.to_owned(),
            account_scope: Some(scope.to_owned()),
            status: AccountBindingStatus::Verified,
            source: "userAbstraction".to_owned(),
            checked_at_ms: Some(1),
            freshness_ms: Some(0),
            credential_fingerprint: None,
            problem: None,
        }
    }

    fn summary(venue: &str, equity_scope: AccountEquityScope) -> VenueAccountSummary {
        VenueAccountSummary {
            venue: venue.to_owned(),
            account_type: "test".to_owned(),
            equity_scope,
            total_equity_usd: 1.0,
            total_available_balance_usd: 1.0,
            withdrawable_balance_usd: Some(1.0),
            total_initial_margin_usd: 0.0,
            total_maintenance_margin_usd: 0.0,
            account_im_rate: 0.0,
            account_mm_rate: 0.0,
            source: "test".to_owned(),
            observed_at_ms: 1,
            freshness_ms: Some(0),
            problem: None,
        }
    }

    #[test]
    fn unified_hyperliquid_uses_one_family_coverage_key() {
        let bindings = [binding("unifiedAccount")];
        let summaries = [summary(HYPERLIQUID_SPOT, AccountEquityScope::Unified)];

        assert_eq!(
            equity_coverage_venue("hyperliquid:xyz", &bindings, &summaries),
            HYPERLIQUID
        );
        assert!(!summary_covers_nav(
            &summary(HYPERLIQUID, AccountEquityScope::Perpetuals),
            &bindings,
            &summaries
        ));
        assert!(summary_covers_nav(
            &summary(HYPERLIQUID_SPOT, AccountEquityScope::Unified),
            &bindings,
            &summaries
        ));
    }

    #[test]
    fn standard_hyperliquid_keeps_independent_dex_scopes() {
        let bindings = [binding("default")];
        let summaries = [summary("hyperliquid:xyz", AccountEquityScope::Perpetuals)];

        assert_eq!(
            equity_coverage_venue("hyperliquid:xyz", &bindings, &summaries),
            "hyperliquid:xyz"
        );
        assert!(summary_covers_nav(
            &summary("hyperliquid:xyz", AccountEquityScope::Perpetuals),
            &bindings,
            &summaries
        ));
    }

    #[test]
    fn runtime_unified_spot_summary_survives_missing_validation_cache() {
        let mut unified = summary(HYPERLIQUID_SPOT, AccountEquityScope::Spot);
        unified.account_type = "unifiedAccount".to_owned();
        let summaries = [unified];

        assert_eq!(
            equity_coverage_venue("hyperliquid:xyz", &[], &summaries),
            HYPERLIQUID
        );
        assert!(summary_covers_nav(&summaries[0], &[], &summaries));
    }
}
