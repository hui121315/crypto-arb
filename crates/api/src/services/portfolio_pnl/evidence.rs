use shared_types::{
    problem::codes, ApiProblem, CloseRunStatus, ExecutionLedgerQuality, PortfolioPnlEvidence,
    ReviewPnlField,
};
use std::collections::BTreeSet;

pub(super) const PNL_FIELDS: [ReviewPnlField; 5] = [
    ReviewPnlField::Gross,
    ReviewPnlField::Fee,
    ReviewPnlField::Funding,
    ReviewPnlField::Slippage,
    ReviewPnlField::Net,
];

pub(super) fn pnl_evidence(
    rows: &[&review_domain::RealizedPnlRow],
    source: &'static str,
    observed_at_ms: i64,
) -> PortfolioPnlEvidence {
    let field_quality = rows
        .iter()
        .map(|row| review_domain::realized_pnl_field_quality(row))
        .collect::<Vec<_>>();
    let mut actual_fields = Vec::new();
    let mut estimated_fields = Vec::new();
    let mut missing_fields = Vec::new();
    for field in PNL_FIELDS {
        match aggregate_field_quality(&field_quality, field) {
            ExecutionLedgerQuality::Actual => actual_fields.push(field),
            ExecutionLedgerQuality::Estimated => estimated_fields.push(field),
            ExecutionLedgerQuality::Missing => missing_fields.push(field),
        }
    }
    let quality = if missing_fields.is_empty() {
        if estimated_fields.is_empty() {
            ExecutionLedgerQuality::Actual
        } else {
            ExecutionLedgerQuality::Estimated
        }
    } else {
        ExecutionLedgerQuality::Missing
    };
    let close_run_ids = rows
        .iter()
        .flat_map(|row| row.evidence.close_run_evidence.iter())
        .map(|evidence| evidence.close_run_id.as_str())
        .collect::<BTreeSet<_>>();
    let unwind_run_ids = rows
        .iter()
        .flat_map(|row| row.evidence.close_run_evidence.iter())
        .filter(|evidence| {
            evidence.unwind_status.is_some()
                || matches!(
                    evidence.status,
                    CloseRunStatus::UnwindRequired
                        | CloseRunStatus::CompensationSubmitted
                        | CloseRunStatus::Compensated
                        | CloseRunStatus::CompensationFailed
                )
        })
        .map(|evidence| evidence.close_run_id.as_str())
        .collect::<BTreeSet<_>>();
    let problem = (quality != ExecutionLedgerQuality::Actual)
        .then(|| pnl_evidence_problem(source, observed_at_ms, &estimated_fields, &missing_fields));
    PortfolioPnlEvidence {
        quality,
        source: source.to_owned(),
        observed_at_ms,
        realized_group_count: bounded_count(rows.len()),
        close_run_count: bounded_count(close_run_ids.len()),
        unwind_run_count: bounded_count(unwind_run_ids.len()),
        actual_fields,
        estimated_fields,
        missing_fields,
        problem,
    }
}

fn aggregate_field_quality(
    rows: &[review_domain::RealizedPnlFieldQuality],
    field: ReviewPnlField,
) -> ExecutionLedgerQuality {
    rows.iter()
        .fold(ExecutionLedgerQuality::Actual, |quality, row| {
            let row_quality = if row.missing.contains(&field) {
                ExecutionLedgerQuality::Missing
            } else if row.estimated.contains(&field) {
                ExecutionLedgerQuality::Estimated
            } else {
                ExecutionLedgerQuality::Actual
            };
            combine_quality(quality, row_quality)
        })
}

fn combine_quality(
    left: ExecutionLedgerQuality,
    right: ExecutionLedgerQuality,
) -> ExecutionLedgerQuality {
    match (left, right) {
        (ExecutionLedgerQuality::Missing, _) | (_, ExecutionLedgerQuality::Missing) => {
            ExecutionLedgerQuality::Missing
        }
        (ExecutionLedgerQuality::Estimated, _) | (_, ExecutionLedgerQuality::Estimated) => {
            ExecutionLedgerQuality::Estimated
        }
        _ => ExecutionLedgerQuality::Actual,
    }
}

fn pnl_evidence_problem(
    source: &'static str,
    observed_at_ms: i64,
    estimated_fields: &[ReviewPnlField],
    missing_fields: &[ReviewPnlField],
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::PORTFOLIO_SNAPSHOT_DEGRADED,
        "portfolio PnL contains estimated or missing ledger fields",
    )
    .with_source(source);
    problem.details = Some(serde_json::json!({
        "operation": "portfolio_pnl",
        "source": source,
        "estimatedFields": estimated_fields,
        "missingFields": missing_fields,
        "observedAtMs": observed_at_ms,
    }));
    problem
}

fn bounded_count(count: usize) -> u32 {
    count.min(u32::MAX as usize) as u32
}
