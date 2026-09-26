use shared_types::{ExecutedTrade, ExecutionFillConfidence, ReviewPnlField, StrategyKind};

pub(super) struct PerformanceAggregate<'a> {
    pub(super) rows: Vec<&'a ExecutedTrade>,
    pub(super) total_trades_30d: u32,
    pub(super) trades_30d: u32,
    pub(super) actual_trades_30d: u32,
    pub(super) estimated_trades_30d: u32,
    pub(super) skipped_trades_30d: u32,
    pub(super) partial_evidence_trades_30d: u32,
    pub(super) lowest_fill_confidence: Option<ExecutionFillConfidence>,
    pub(super) profitable_trades_30d: u32,
    pub(super) losing_trades_30d: u32,
    pub(super) break_even_trades_30d: u32,
    pub(super) gross: f64,
    pub(super) net: f64,
    pub(super) gross_profit_30d_usd: f64,
    pub(super) gross_loss_30d_usd: f64,
    pub(super) profit_factor: Option<f64>,
    pub(super) actual_net_pnl_30d_usd: f64,
    pub(super) estimated_net_pnl_30d_usd: f64,
}

impl<'a> PerformanceAggregate<'a> {
    pub(super) fn from_trades(trades: &'a [ExecutedTrade], kind: StrategyKind) -> Self {
        Self::from_rows(trades.iter().filter(|trade| trade.strategy == kind))
    }

    pub(super) fn from_rows(trades: impl Iterator<Item = &'a ExecutedTrade>) -> Self {
        let all_rows = trades.collect::<Vec<_>>();
        let usable_rows = all_rows
            .iter()
            .copied()
            .filter(|trade| has_usable_net_evidence(trade))
            .collect::<Vec<_>>();
        let rows = usable_rows
            .iter()
            .copied()
            .filter(|trade| trade.actual_fields.contains(&ReviewPnlField::Net))
            .collect::<Vec<_>>();
        let trades_30d = usable_rows.len() as u32;
        let total_trades_30d = all_rows.len() as u32;
        let skipped_trades_30d = total_trades_30d.saturating_sub(trades_30d);
        let actual_trades_30d = rows.len() as u32;
        let estimated_trades_30d = usable_rows
            .iter()
            .filter(|trade| {
                !trade.actual_fields.contains(&ReviewPnlField::Net)
                    && trade.estimated_fields.contains(&ReviewPnlField::Net)
            })
            .count() as u32;
        let partial_evidence_trades_30d = usable_rows
            .iter()
            .filter(|trade| has_partial_metric_evidence(trade))
            .count() as u32;
        let lowest_fill_confidence = lowest_fill_confidence(&usable_rows);
        let gross = rows.iter().map(|trade| trade.gross_pnl_usd).sum::<f64>();
        let net = rows.iter().map(|trade| trade.net_pnl_usd).sum::<f64>();
        let actual_net_pnl_30d_usd = net;
        let estimated_net_pnl_30d_usd = usable_rows
            .iter()
            .filter(|trade| {
                !trade.actual_fields.contains(&ReviewPnlField::Net)
                    && trade.estimated_fields.contains(&ReviewPnlField::Net)
            })
            .map(|trade| trade.net_pnl_usd)
            .sum::<f64>();
        let profitable_trades_30d = rows
            .iter()
            .filter(|trade| trade.net_pnl_usd > f64::EPSILON)
            .count() as u32;
        let losing_trades_30d = rows
            .iter()
            .filter(|trade| trade.net_pnl_usd < -f64::EPSILON)
            .count() as u32;
        let break_even_trades_30d = actual_trades_30d
            .saturating_sub(profitable_trades_30d)
            .saturating_sub(losing_trades_30d);
        let gross_profit_30d_usd = rows
            .iter()
            .map(|trade| trade.net_pnl_usd.max(0.0))
            .sum::<f64>();
        let gross_loss_30d_usd = rows
            .iter()
            .map(|trade| (-trade.net_pnl_usd).max(0.0))
            .sum::<f64>();
        let profit_factor = (gross_loss_30d_usd > f64::EPSILON)
            .then_some(gross_profit_30d_usd / gross_loss_30d_usd);

        Self {
            rows,
            total_trades_30d,
            trades_30d,
            actual_trades_30d,
            estimated_trades_30d,
            skipped_trades_30d,
            partial_evidence_trades_30d,
            lowest_fill_confidence,
            profitable_trades_30d,
            losing_trades_30d,
            break_even_trades_30d,
            gross,
            net,
            gross_profit_30d_usd,
            gross_loss_30d_usd,
            profit_factor,
            actual_net_pnl_30d_usd,
            estimated_net_pnl_30d_usd,
        }
    }
}

fn has_usable_net_evidence(trade: &ExecutedTrade) -> bool {
    !trade.missing_fields.contains(&ReviewPnlField::Net)
        && (trade.actual_fields.contains(&ReviewPnlField::Net)
            || trade.estimated_fields.contains(&ReviewPnlField::Net))
}

fn has_partial_metric_evidence(trade: &ExecutedTrade) -> bool {
    !trade.estimated_fields.is_empty() || !trade.missing_fields.is_empty()
}

fn lowest_fill_confidence(rows: &[&ExecutedTrade]) -> Option<ExecutionFillConfidence> {
    rows.iter()
        .filter_map(|trade| trade.evidence.fill_confidence)
        .min_by(|left, right| left.score().total_cmp(&right.score()))
}
