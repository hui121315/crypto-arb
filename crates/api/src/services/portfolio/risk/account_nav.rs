use super::*;

#[derive(Debug, Clone)]
pub(in crate::services::portfolio) struct AccountNav {
    pub(in crate::services::portfolio) value: f64,
    pub(in crate::services::portfolio) evidence: PortfolioNavEvidence,
}

pub(in crate::services::portfolio) fn account_nav(
    account_state: &AccountStateSnapshot,
    now_ms: i64,
) -> AccountNav {
    let account_summaries = &account_state.balances.account_summaries;
    let summaries = account_summaries
        .iter()
        .filter(|row| {
            account_equity_scope::summary_covers_nav(
                row,
                &account_state.account_bindings,
                account_summaries,
            )
        })
        .collect::<Vec<_>>();
    let covered_venues = summaries
        .iter()
        .map(|row| {
            account_equity_scope::equity_coverage_venue(
                &row.venue,
                &account_state.account_bindings,
                account_summaries,
            )
        })
        .collect::<BTreeSet<_>>();
    let mut missing_venues = account_state
        .field_quality
        .iter()
        .filter(|row| row.field == "equity" && row.status != AccountFieldQualityStatus::Actual)
        .filter_map(|row| row.subject.venue.as_deref())
        .map(|venue| {
            account_equity_scope::equity_coverage_venue(
                venue,
                &account_state.account_bindings,
                account_summaries,
            )
        })
        .collect::<BTreeSet<_>>();
    for row in &summaries {
        if row.problem.is_some()
            || row.equity_scope == AccountEquityScope::Unknown
            || !row.total_equity_usd.is_finite()
            || row.total_equity_usd < 0.0
        {
            missing_venues.insert(account_equity_scope::equity_coverage_venue(
                &row.venue,
                &account_state.account_bindings,
                account_summaries,
            ));
        }
    }
    if summaries.is_empty() && missing_venues.is_empty() {
        missing_venues.insert("account_state".to_owned());
    }
    let covered_venues = covered_venues.into_iter().collect::<Vec<_>>();
    let missing_venues = missing_venues.into_iter().collect::<Vec<_>>();
    if missing_venues.is_empty() {
        return AccountNav {
            value: summaries.iter().map(|row| row.total_equity_usd).sum(),
            evidence: PortfolioNavEvidence {
                status: AccountFieldQualityStatus::Actual,
                source: "account_state.account_summaries.total_equity_usd".to_owned(),
                observed_at_ms: now_ms,
                breakdown: PortfolioNavBreakdown::default(),
                covered_venues,
                missing_venues,
                problem: None,
            },
        };
    }
    let problem = nav_missing_problem(&covered_venues, &missing_venues, now_ms);
    AccountNav {
        value: 0.0,
        evidence: PortfolioNavEvidence {
            status: AccountFieldQualityStatus::Missing,
            source: "account_state.account_summaries.total_equity_usd".to_owned(),
            observed_at_ms: now_ms,
            breakdown: PortfolioNavBreakdown::default(),
            covered_venues,
            missing_venues,
            problem: Some(problem),
        },
    }
}

fn nav_missing_problem(covered: &[String], missing: &[String], now_ms: i64) -> ApiProblem {
    let mut problem = ApiProblem::new(
        shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN,
        "portfolio NAV is unavailable because account-level equity coverage is incomplete",
    )
    .with_source("account_state.account_summaries");
    problem.details = Some(serde_json::json!({
        "operation": "portfolio_nav",
        "field": "equity",
        "coveredVenues": covered,
        "missingVenues": missing,
        "observedAtMs": now_ms,
    }));
    problem
}
