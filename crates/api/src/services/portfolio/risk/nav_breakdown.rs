use super::*;

pub(super) fn nav_breakdown(
    rows: &[PositionRow],
    account_state: &AccountStateSnapshot,
    total_nav_usd: f64,
    nav_evidence: &PortfolioNavEvidence,
    now_ms: i64,
) -> PortfolioNavBreakdown {
    let wallet_equity = wallet_equity_component(total_nav_usd, nav_evidence, now_ms);
    let status = position_component_status(rows, account_state);
    let position_equity_value = finite_sum(
        rows.iter()
            .map(|row| row.margin_usd + row.unrealized_pnl_usd),
    );
    let unrealized_value = finite_sum(rows.iter().map(|row| row.unrealized_pnl_usd));
    let position_problem = position_component_problem(status, account_state, now_ms);
    let position_equity = PortfolioValueEvidence {
        value_usd: position_equity_value,
        status,
        source: "account_state.positions.margin_plus_unrealized_pnl".to_owned(),
        observed_at_ms: now_ms,
        problem: position_problem.clone(),
    };
    let unrealized_pnl = PortfolioValueEvidence {
        value_usd: unrealized_value,
        status,
        source: "account_state.positions.unrealized_pnl".to_owned(),
        observed_at_ms: now_ms,
        problem: position_problem,
    };
    let cash = cash_component(&wallet_equity, &position_equity, now_ms);
    PortfolioNavBreakdown {
        wallet_equity,
        position_equity,
        cash,
        unrealized_pnl,
    }
}

fn wallet_equity_component(
    total_nav_usd: f64,
    nav_evidence: &PortfolioNavEvidence,
    now_ms: i64,
) -> PortfolioValueEvidence {
    let value_usd = (nav_evidence.status == AccountFieldQualityStatus::Actual
        && total_nav_usd.is_finite())
    .then_some(total_nav_usd);
    PortfolioValueEvidence {
        value_usd,
        status: nav_evidence.status,
        source: nav_evidence.source.clone(),
        observed_at_ms: now_ms,
        problem: nav_evidence.problem.clone(),
    }
}

fn position_component_status(
    rows: &[PositionRow],
    account_state: &AccountStateSnapshot,
) -> AccountFieldQualityStatus {
    if !account_state.positions.problems.is_empty()
        || account_state.positions.operation_health.iter().any(|row| {
            matches!(
                row.status,
                VenueOperationStatus::Warn
                    | VenueOperationStatus::Blocked
                    | VenueOperationStatus::Unknown
            )
        })
        || account_state
            .positions
            .row_health
            .iter()
            .any(|row| row.last_error.is_some())
        || rows.iter().any(|row| {
            !row.margin_usd.is_finite()
                || !row.unrealized_pnl_usd.is_finite()
                || !row.mark_price.is_finite()
        })
    {
        return AccountFieldQualityStatus::Missing;
    }
    account_state
        .field_quality
        .iter()
        .filter(|row| matches!(row.field.as_str(), "markPrice" | "margin" | "unrealizedPnl"))
        .fold(AccountFieldQualityStatus::Actual, |status, row| {
            combine_component_status(status, row.status)
        })
}

fn combine_component_status(
    left: AccountFieldQualityStatus,
    right: AccountFieldQualityStatus,
) -> AccountFieldQualityStatus {
    match (left, right) {
        (AccountFieldQualityStatus::Missing, _)
        | (AccountFieldQualityStatus::Invalid, _)
        | (AccountFieldQualityStatus::Unknown, _)
        | (_, AccountFieldQualityStatus::Missing)
        | (_, AccountFieldQualityStatus::Invalid)
        | (_, AccountFieldQualityStatus::Unknown) => AccountFieldQualityStatus::Missing,
        (AccountFieldQualityStatus::Estimated, _) | (_, AccountFieldQualityStatus::Estimated) => {
            AccountFieldQualityStatus::Estimated
        }
        _ => AccountFieldQualityStatus::Actual,
    }
}

fn position_component_problem(
    status: AccountFieldQualityStatus,
    account_state: &AccountStateSnapshot,
    now_ms: i64,
) -> Option<ApiProblem> {
    (status != AccountFieldQualityStatus::Actual).then(|| {
        let mut problem = ApiProblem::new(
            shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN,
            if status == AccountFieldQualityStatus::Estimated {
                "position equity contains estimated mark or margin inputs"
            } else {
                "position equity coverage is incomplete"
            },
        )
        .with_source("account_state.positions");
        problem.details = Some(serde_json::json!({
            "operation": "portfolio_nav_components",
            "field": "position_equity",
            "status": status,
            "source": account_state.positions.source.as_str(),
            "observedAtMs": now_ms,
        }));
        problem
    })
}

fn cash_component(
    wallet_equity: &PortfolioValueEvidence,
    position_equity: &PortfolioValueEvidence,
    now_ms: i64,
) -> PortfolioValueEvidence {
    let value_usd = match (wallet_equity.value_usd, position_equity.value_usd) {
        (Some(wallet), Some(position))
            if position_equity.status != AccountFieldQualityStatus::Missing =>
        {
            let cash = wallet - position;
            cash.is_finite().then_some(cash)
        }
        _ => None,
    };
    let status = if value_usd.is_some() {
        AccountFieldQualityStatus::Estimated
    } else {
        AccountFieldQualityStatus::Missing
    };
    let mut problem = ApiProblem::new(
        shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN,
        if status == AccountFieldQualityStatus::Estimated {
            "cash is an estimated residual of wallet and position equity"
        } else {
            "cash is unavailable without complete wallet and position equity"
        },
    )
    .with_source("wallet_equity_minus_position_equity");
    problem.details = Some(serde_json::json!({
        "operation": "portfolio_nav_components",
        "field": "cash",
        "status": status,
        "observedAtMs": now_ms,
    }));
    PortfolioValueEvidence {
        value_usd,
        status,
        source: "wallet_equity_minus_position_equity".to_owned(),
        observed_at_ms: now_ms,
        problem: Some(problem),
    }
}

fn finite_sum(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    values
        .into_iter()
        .try_fold(0.0, |sum, value| value.is_finite().then_some(sum + value))
}
