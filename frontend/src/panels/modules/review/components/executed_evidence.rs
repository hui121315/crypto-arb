use shared_types::{
    ExecutedTrade, ExecutionEnvironment, LiveOrderState, OrderRecord, OrderUpdateSource,
    ReviewPnlField,
};

use super::format::{fill_confidence_label, order_update_source_label};

pub(in crate::panels::modules::review) fn evidence_summary(row: &ExecutedTrade) -> String {
    [
        execution_environment_summary(row),
        fill_confidence_summary(row),
        event_count_summary(row),
        close_run_summary(row),
        order_finality_summary(row),
        field_quality_summary(row),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::review) enum ReviewExecutionEnvironment {
    Paper,
    Live,
    Mixed,
    Unknown,
}

impl ReviewExecutionEnvironment {
    pub(in crate::panels::modules::review) const fn label(self) -> &'static str {
        match self {
            Self::Paper => "模拟",
            Self::Live => "实盘",
            Self::Mixed => "环境混合",
            Self::Unknown => "环境未知",
        }
    }

    pub(in crate::panels::modules::review) const fn tone(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::Live => "live",
            Self::Mixed => "mixed",
            Self::Unknown => "unknown",
        }
    }
}

pub(in crate::panels::modules::review) fn review_execution_environment(
    row: &ExecutedTrade,
) -> ReviewExecutionEnvironment {
    let mut environments = row
        .long_orders
        .iter()
        .chain(row.short_orders.iter())
        .map(|order| order.intent.mode.environment());
    let Some(environment) = environments.next() else {
        return ReviewExecutionEnvironment::Unknown;
    };
    if environments.any(|candidate| candidate != environment) {
        ReviewExecutionEnvironment::Mixed
    } else {
        match environment {
            ExecutionEnvironment::Paper => ReviewExecutionEnvironment::Paper,
            ExecutionEnvironment::Live => ReviewExecutionEnvironment::Live,
        }
    }
}

fn execution_environment_summary(row: &ExecutedTrade) -> String {
    let environment = review_execution_environment(row);
    match environment {
        ReviewExecutionEnvironment::Unknown => "执行环境缺证据".to_owned(),
        ReviewExecutionEnvironment::Mixed => "执行环境冲突".to_owned(),
        ReviewExecutionEnvironment::Paper | ReviewExecutionEnvironment::Live => {
            format!("执行环境 {}", environment.label())
        }
    }
}

fn fill_confidence_summary(row: &ExecutedTrade) -> String {
    row.evidence
        .fill_confidence
        .map(fill_confidence_label)
        .unwrap_or("缺成交置信度")
        .to_owned()
}

fn event_count_summary(row: &ExecutedTrade) -> String {
    format!(
        "事件 fill:{} fee:{} funding:{} slip:{} book:{}",
        row.evidence.fill_event_ids.len(),
        row.evidence.fee_event_ids.len(),
        row.evidence.funding_event_ids.len(),
        row.evidence.slippage_event_ids.len(),
        row.evidence.orderbook_event_ids.len()
    )
}

fn close_run_summary(row: &ExecutedTrade) -> String {
    let count = row.evidence.close_run_evidence.len();
    if count == 0 {
        return "无 CloseRun 证据".to_owned();
    }
    let cost_events = row
        .evidence
        .close_run_evidence
        .iter()
        .filter_map(|evidence| evidence.cost_reconciliation.as_ref())
        .map(|reconciliation| reconciliation.evidence_event_ids.len())
        .sum::<usize>();
    format!("CloseRun:{count} cost:{cost_events}")
}

fn order_finality_summary(row: &ExecutedTrade) -> String {
    let orders = row
        .long_orders
        .iter()
        .chain(row.short_orders.iter())
        .collect::<Vec<_>>();
    if orders.is_empty() {
        return "无订单终态".to_owned();
    }
    let filled = orders
        .iter()
        .filter(|order| order.state == LiveOrderState::Filled)
        .count();
    let sources = order_source_summary(&orders);
    format!("终态 {filled}/{} Filled via {sources}", orders.len())
}

fn order_source_summary(orders: &[&OrderRecord]) -> String {
    let mut sources = Vec::<OrderUpdateSource>::new();
    for order in orders {
        if !sources.contains(&order.last_update_source) {
            sources.push(order.last_update_source);
        }
    }
    sources
        .into_iter()
        .map(order_update_source_label)
        .collect::<Vec<_>>()
        .join("/")
}

fn field_quality_summary(row: &ExecutedTrade) -> String {
    let (actual, estimated, missing) = [
        ReviewPnlField::Gross,
        ReviewPnlField::Fee,
        ReviewPnlField::Funding,
        ReviewPnlField::Slippage,
        ReviewPnlField::Net,
    ]
    .into_iter()
    .fold((0, 0, 0), |(actual, estimated, missing), field| {
        if row.missing_fields.contains(&field)
            || (!row.actual_fields.contains(&field) && !row.estimated_fields.contains(&field))
        {
            (actual, estimated, missing + 1)
        } else if row.estimated_fields.contains(&field) {
            (actual, estimated + 1, missing)
        } else {
            (actual + 1, estimated, missing)
        }
    });
    format!("已确认 {actual} · 估算 {estimated} · 缺证据 {missing}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        CloseRunCostReconciliation, CloseRunStatus, ExecutionFillConfidence, ExecutionMode,
        MarginMode, OrderIntent, OrderSide, OrderSource, OrderType, ReviewCloseRunEvidence,
        ReviewPnlEvidence, ReviewPnlField, StrategyKind, TimeInForce, VenueOrderIdentity,
    };

    #[test]
    fn evidence_summary_surfaces_confidence_events_and_sources() {
        let mut row = trade();
        row.evidence.fill_event_ids = vec!["fill-long".into(), "fill-short".into()];
        row.evidence.fee_event_ids = vec!["fee-long".into()];
        row.evidence.funding_event_ids = vec!["funding-long".into()];
        row.evidence.slippage_event_ids = vec!["slip-long".into(), "slip-short".into()];
        row.evidence.orderbook_event_ids = vec!["book-long".into(), "book-short".into()];
        row.evidence
            .record_close_run_evidence(ReviewCloseRunEvidence {
                close_run_id: "close-1".into(),
                status: CloseRunStatus::Compensated,
                run_id: "run-1".into(),
                ticket_id: "ticket-1".into(),
                opportunity_id: "opp-1".into(),
                matched_notional_usd: 100.0,
                unwind_status: None,
                compensation_attempt_count: 0,
                cost_reconciliation: Some(CloseRunCostReconciliation {
                    evidence_event_ids: vec!["funding-1".into(), "manual-1".into()],
                    ..CloseRunCostReconciliation::default()
                }),
            });
        row.evidence
            .record_fill_confidence(ExecutionFillConfidence::AdapterAck);
        row.long_orders = vec![order_record(
            LiveOrderState::Filled,
            OrderUpdateSource::PrivateWs,
        )];
        row.short_orders = vec![order_record(
            LiveOrderState::Filled,
            OrderUpdateSource::OrderQuery,
        )];

        let summary = evidence_summary(&row);

        assert!(summary.contains("仅 ACK 推定"));
        assert!(summary.contains("fill:2"));
        assert!(summary.contains("fee:1"));
        assert!(summary.contains("funding:1"));
        assert!(summary.contains("slip:2"));
        assert!(summary.contains("book:2"));
        assert!(summary.contains("CloseRun:1"));
        assert!(summary.contains("cost:2"));
        assert!(summary.contains("终态 2/2 Filled"));
        assert!(summary.contains("私有 WS"));
        assert!(summary.contains("订单回查"));
    }

    #[test]
    fn evidence_summary_counts_estimated_and_missing_fields() {
        let mut row = trade();
        row.actual_fields = vec![ReviewPnlField::Gross, ReviewPnlField::Net];
        row.estimated_fields = vec![ReviewPnlField::Fee];
        row.missing_fields = vec![ReviewPnlField::Funding, ReviewPnlField::Slippage];

        let summary = evidence_summary(&row);

        assert!(summary.contains("已确认 2"));
        assert!(summary.contains("估算 1"));
        assert!(summary.contains("缺证据 2"));
    }

    #[test]
    fn evidence_summary_counts_legacy_unclassified_fields_as_missing() {
        let summary = evidence_summary(&trade());

        assert!(summary.contains("执行环境缺证据"));
        assert!(summary.contains("已确认 0 · 估算 0 · 缺证据 5"));
    }

    #[test]
    fn evidence_summary_projects_legacy_modes_and_rejects_mixed_environments() {
        let mut row = trade();
        row.long_orders = vec![order_record_with_mode(ExecutionMode::DryRun)];
        row.short_orders = vec![order_record_with_mode(ExecutionMode::Testnet)];

        assert!(evidence_summary(&row).contains("执行环境 模拟"));

        row.short_orders = vec![order_record_with_mode(ExecutionMode::Live)];
        assert!(evidence_summary(&row).contains("执行环境冲突"));
    }

    fn trade() -> ExecutedTrade {
        ExecutedTrade {
            id: "hedge-1".into(),
            strategy: StrategyKind::PerpCross,
            symbol: "BTC".into(),
            long_venue: "binance".into(),
            short_venue: "okx".into(),
            opened_at_ms: 0,
            closed_at_ms: Some(1),
            holding_minutes: Some(1),
            gross_pnl_usd: 1.0,
            fee_usd: 0.1,
            funding_usd: 0.0,
            slippage_usd: 0.0,
            net_pnl_usd: 0.9,
            evidence: ReviewPnlEvidence::default(),
            actual_fields: Vec::new(),
            estimated_fields: Vec::new(),
            missing_fields: Vec::new(),
            long_orders: Vec::new(),
            short_orders: Vec::new(),
        }
    }

    fn order_record(state: LiveOrderState, source: OrderUpdateSource) -> OrderRecord {
        let mut order = order_record_with_mode(ExecutionMode::DryRun);
        order.state = state;
        order.last_update_source = source;
        order
    }

    fn order_record_with_mode(mode: ExecutionMode) -> OrderRecord {
        let intent = OrderIntent {
            id: "order-1".into(),
            source: OrderSource::ArbitragePreview,
            strategy: Some(StrategyKind::PerpCross),
            mode,
            exchange: "okx".into(),
            symbol: "BTCUSDT".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".into(),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        OrderRecord {
            identity: VenueOrderIdentity::from_intent(&intent),
            intent,
            state: LiveOrderState::Filled,
            risk: None,
            last_update_source: OrderUpdateSource::PrivateWs,
            exchange_order_id: Some("ex-1".into()),
            message: None,
            filled_quantity: Some(1.0),
            filled_price: Some(100.0),
            filled_fee: Some(0.1),
            updated_at_ms: 2,
        }
    }
}
