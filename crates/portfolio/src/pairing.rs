use shared_types::{PositionPairEvidence, PositionRow, PositionSide};

pub fn pair_positions(rows: &mut [PositionRow], evidence: &[PositionPairEvidence]) {
    clear_pairing(rows);
    for item in evidence {
        if !has_reciprocal_evidence(evidence, item) {
            continue;
        }
        if !has_partner_row(rows, item) {
            continue;
        }
        let Some(index) = unique_leg_index(rows, item) else {
            continue;
        };
        if rows[index].pair_evidence.is_some() {
            continue;
        }
        rows[index].paired_with = Some(format!("{}@{}", item.partner_venue, item.partner_symbol));
        rows[index].pair_evidence = Some(item.clone());
    }
}

fn has_reciprocal_evidence(rows: &[PositionPairEvidence], item: &PositionPairEvidence) -> bool {
    rows.iter().any(|candidate| {
        candidate.source == item.source
            && candidate.run_id == item.run_id
            && shared_types::venue_names_equal(&candidate.venue, &item.partner_venue)
            && candidate.symbol.eq_ignore_ascii_case(&item.partner_symbol)
            && candidate.side == item.partner_side
            && shared_types::venue_names_equal(&candidate.partner_venue, &item.venue)
            && candidate.partner_symbol.eq_ignore_ascii_case(&item.symbol)
            && candidate.partner_side == item.side
    })
}

fn clear_pairing(rows: &mut [PositionRow]) {
    for row in rows {
        row.paired_with = None;
        row.pair_evidence = None;
    }
}

fn has_partner_row(rows: &[PositionRow], evidence: &PositionPairEvidence) -> bool {
    rows.iter().any(|row| {
        row_matches(
            row,
            &evidence.partner_venue,
            &evidence.partner_symbol,
            evidence.partner_side,
            evidence.partner_filled_quantity,
        )
    })
}

fn unique_leg_index(rows: &[PositionRow], evidence: &PositionPairEvidence) -> Option<usize> {
    let mut matches = rows.iter().enumerate().filter_map(|(index, row)| {
        row_matches(
            row,
            &evidence.venue,
            &evidence.symbol,
            evidence.side,
            evidence.leg_filled_quantity,
        )
        .then_some(index)
    });
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn row_matches(
    row: &PositionRow,
    venue: &str,
    symbol: &str,
    side: PositionSide,
    quantity: f64,
) -> bool {
    shared_types::venue_names_equal(&row.venue, venue)
        && row.symbol.eq_ignore_ascii_case(symbol)
        && row.side == side
        && quantity_matches(row.quantity, quantity)
}

fn quantity_matches(row_quantity: f64, evidence_quantity: f64) -> bool {
    if !row_quantity.is_finite() || !evidence_quantity.is_finite() {
        return false;
    }
    let tolerance = (evidence_quantity.abs() * 1e-9).max(1e-9);
    (row_quantity.abs() - evidence_quantity.abs()).abs() <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::PositionPairEvidenceSource;

    #[test]
    fn does_not_pair_opposite_sides_without_evidence() {
        let mut rows = vec![
            row("OKX", PositionSide::Long),
            row("Hyperliquid", PositionSide::Short),
        ];

        pair_positions(&mut rows, &[]);

        assert!(rows.iter().all(|row| row.paired_with.is_none()));
        assert!(rows.iter().all(|row| row.pair_evidence.is_none()));
    }

    #[test]
    fn applies_execution_run_pair_evidence() {
        let mut rows = vec![
            row("OKX", PositionSide::Long),
            row("Hyperliquid", PositionSide::Short),
        ];

        pair_positions(&mut rows, &pair_evidence());

        assert_eq!(rows[0].paired_with.as_deref(), Some("Hyperliquid@BTC"));
        assert_eq!(rows[1].paired_with.as_deref(), Some("OKX@BTC"));
        assert_eq!(
            rows[0]
                .pair_evidence
                .as_ref()
                .map(|item| item.run_id.as_str()),
            Some("run-1")
        );
        assert_eq!(
            rows[1]
                .pair_evidence
                .as_ref()
                .map(|item| item.ticket_id.as_str()),
            Some("ticket-1")
        );
    }

    #[test]
    fn does_not_apply_one_sided_evidence() {
        let mut rows = vec![
            row("OKX", PositionSide::Long),
            row("Hyperliquid", PositionSide::Short),
        ];
        let evidence = [evidence(
            "OKX",
            PositionSide::Long,
            "Hyperliquid",
            PositionSide::Short,
        )];

        pair_positions(&mut rows, &evidence);

        assert!(rows.iter().all(|row| row.paired_with.is_none()));
        assert!(rows.iter().all(|row| row.pair_evidence.is_none()));
    }

    fn row(venue: &str, side: PositionSide) -> PositionRow {
        PositionRow {
            venue: venue.into(),
            symbol: "BTC".into(),
            origin: Default::default(),
            side,
            quantity: 1.0,
            entry_price: 100.0,
            mark_price: 100.0,
            leverage: 2.0,
            unrealized_pnl_usd: 0.0,
            liquidation_price: None,
            liquidation_distance_pct: None,
            next_funding_ms: None,
            funding_rate_8h: 0.0,
            funding_rate_verified: true,
            maintenance_margin_ratio: 0.05,
            pair_evidence: None,
            paired_with: None,
            margin_usd: 50.0,
            severity: shared_types::PositionSeverity::Ok,
            seconds_until_funding: None,
        }
    }

    fn pair_evidence() -> [PositionPairEvidence; 2] {
        [
            evidence(
                "OKX",
                PositionSide::Long,
                "Hyperliquid",
                PositionSide::Short,
            ),
            evidence(
                "Hyperliquid",
                PositionSide::Short,
                "OKX",
                PositionSide::Long,
            ),
        ]
    }

    fn evidence(
        venue: &str,
        side: PositionSide,
        partner_venue: &str,
        partner_side: PositionSide,
    ) -> PositionPairEvidence {
        PositionPairEvidence {
            source: PositionPairEvidenceSource::ExecutionRun,
            run_id: "run-1".into(),
            ticket_id: "ticket-1".into(),
            opportunity_id: "opp-1".into(),
            venue: venue.into(),
            symbol: "BTC".into(),
            side,
            partner_venue: partner_venue.into(),
            partner_symbol: "BTC".into(),
            partner_side,
            leg_filled_quantity: 1.0,
            partner_filled_quantity: 1.0,
            matched_notional_usd: 100.0,
            updated_at_ms: 7,
        }
    }
}
