use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealizedPnlFieldQuality {
    pub actual: Vec<ReviewPnlField>,
    pub estimated: Vec<ReviewPnlField>,
    pub missing: Vec<ReviewPnlField>,
}

pub fn realized_pnl_field_quality(row: &RealizedPnlRow) -> RealizedPnlFieldQuality {
    let open_fill_quality =
        event_set_quality(&row.evidence, &row.evidence.fill_event_ids, fill_quality);
    let gross = row
        .close_price_quality
        .map_or(ExecutionLedgerQuality::Missing, |close_price_quality| {
            combined_quality([open_fill_quality, close_price_quality])
        });
    let fee = covered_event_quality(
        &row.evidence,
        &row.evidence.fee_event_ids,
        &row.evidence.fill_event_ids,
        fee_quality,
    );
    let funding = funding_field_quality(&row.evidence);
    let slippage = slippage_quality(&row.evidence);
    let close_run_cost_missing = row
        .evidence
        .close_run_evidence
        .iter()
        .any(close_run_cost_missing);
    let net = if close_run_cost_missing {
        ExecutionLedgerQuality::Missing
    } else {
        combined_quality([gross, fee, funding, slippage])
    };
    let mut fields = RealizedPnlFieldQuality::default();
    for (field, quality) in [
        (ReviewPnlField::Gross, gross),
        (ReviewPnlField::Fee, fee),
        (ReviewPnlField::Funding, funding),
        (ReviewPnlField::Slippage, slippage),
        (ReviewPnlField::Net, net),
    ] {
        match quality {
            ExecutionLedgerQuality::Actual => fields.actual.push(field),
            ExecutionLedgerQuality::Estimated => fields.estimated.push(field),
            ExecutionLedgerQuality::Missing => fields.missing.push(field),
        }
    }
    fields
}

fn funding_field_quality(evidence: &ReviewPnlEvidence) -> ExecutionLedgerQuality {
    let ledger_quality = event_set_quality(
        evidence,
        &evidence.funding_event_ids,
        funding_payload_quality,
    );
    if ledger_quality != ExecutionLedgerQuality::Missing {
        return ledger_quality;
    }
    if !evidence.close_run_evidence.is_empty()
        && evidence.close_run_evidence.iter().all(|close| {
            close.cost_reconciliation.as_ref().is_some_and(|cost| {
                cost.funding_usd.is_some_and(f64::is_finite) && !cost.funding_event_ids.is_empty()
            })
        })
    {
        ExecutionLedgerQuality::Actual
    } else {
        ExecutionLedgerQuality::Missing
    }
}

fn covered_event_quality(
    evidence: &ReviewPnlEvidence,
    field_event_ids: &[String],
    required_event_ids: &[String],
    quality: fn(&ReviewLedgerPayloadEvidence) -> Option<ExecutionLedgerQuality>,
) -> ExecutionLedgerQuality {
    let required_orders = event_order_ids(evidence, required_event_ids, fill_quality);
    let covered_orders = event_order_ids(evidence, field_event_ids, quality);
    if required_orders.is_empty() || !required_orders.is_subset(&covered_orders) {
        return ExecutionLedgerQuality::Missing;
    }
    event_set_quality(evidence, field_event_ids, quality)
}

fn slippage_quality(evidence: &ReviewPnlEvidence) -> ExecutionLedgerQuality {
    let required_orders = event_order_ids(evidence, &evidence.fill_event_ids, fill_quality);
    let mut covered_orders = event_order_ids(
        evidence,
        &evidence.slippage_event_ids,
        slippage_payload_quality,
    );
    covered_orders.extend(event_order_ids(
        evidence,
        &evidence.estimated_slippage_fill_event_ids,
        fill_quality,
    ));
    if required_orders.is_empty() || !required_orders.is_subset(&covered_orders) {
        return ExecutionLedgerQuality::Missing;
    }
    let durable = if evidence.slippage_event_ids.is_empty() {
        ExecutionLedgerQuality::Actual
    } else {
        event_set_quality(
            evidence,
            &evidence.slippage_event_ids,
            slippage_payload_quality,
        )
    };
    if durable == ExecutionLedgerQuality::Missing {
        return durable;
    }
    if evidence.estimated_slippage_fill_event_ids.is_empty() {
        durable
    } else {
        ExecutionLedgerQuality::Estimated
    }
}

fn event_order_ids(
    evidence: &ReviewPnlEvidence,
    event_ids: &[String],
    quality: fn(&ReviewLedgerPayloadEvidence) -> Option<ExecutionLedgerQuality>,
) -> BTreeSet<String> {
    event_ids
        .iter()
        .filter_map(|event_id| review_event(evidence, event_id))
        .filter(|event| quality(&event.payload).is_some())
        .map(|event| event.order.identity.internal_order_id.clone())
        .collect()
}

fn event_set_quality(
    evidence: &ReviewPnlEvidence,
    event_ids: &[String],
    quality: fn(&ReviewLedgerPayloadEvidence) -> Option<ExecutionLedgerQuality>,
) -> ExecutionLedgerQuality {
    if event_ids.is_empty() {
        return ExecutionLedgerQuality::Missing;
    }
    let qualities = event_ids.iter().map(|event_id| {
        review_event(evidence, event_id)
            .and_then(|event| quality(&event.payload))
            .unwrap_or(ExecutionLedgerQuality::Missing)
    });
    combined_quality(qualities)
}

fn review_event<'a>(
    evidence: &'a ReviewPnlEvidence,
    event_id: &str,
) -> Option<&'a ReviewLedgerEventEvidence> {
    evidence
        .ledger_events
        .iter()
        .find(|event| event.event_id == event_id)
}

fn combined_quality(
    qualities: impl IntoIterator<Item = ExecutionLedgerQuality>,
) -> ExecutionLedgerQuality {
    let mut combined = ExecutionLedgerQuality::Actual;
    for quality in qualities {
        match quality {
            ExecutionLedgerQuality::Missing => return ExecutionLedgerQuality::Missing,
            ExecutionLedgerQuality::Estimated => combined = ExecutionLedgerQuality::Estimated,
            ExecutionLedgerQuality::Actual => {}
        }
    }
    combined
}

fn fill_quality(payload: &ReviewLedgerPayloadEvidence) -> Option<ExecutionLedgerQuality> {
    match payload {
        ReviewLedgerPayloadEvidence::Fill { quality, .. } => Some(*quality),
        _ => None,
    }
}

fn fee_quality(payload: &ReviewLedgerPayloadEvidence) -> Option<ExecutionLedgerQuality> {
    match payload {
        ReviewLedgerPayloadEvidence::Fill { fee: Some(fee), .. } => Some(fee.quality),
        ReviewLedgerPayloadEvidence::Fee { quality, .. } => Some(*quality),
        _ => None,
    }
}

fn funding_payload_quality(
    payload: &ReviewLedgerPayloadEvidence,
) -> Option<ExecutionLedgerQuality> {
    match payload {
        ReviewLedgerPayloadEvidence::Funding { quality, .. } => Some(*quality),
        _ => None,
    }
}

fn slippage_payload_quality(
    payload: &ReviewLedgerPayloadEvidence,
) -> Option<ExecutionLedgerQuality> {
    match payload {
        ReviewLedgerPayloadEvidence::Slippage { quality, .. } => Some(*quality),
        _ => None,
    }
}
