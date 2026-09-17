use shared_types::{
    ExecutedTrade, FundingDiffRow, HistoryBackendStatus, HistoryPage, HistoryResponse,
    ListEnvelope, ListPage, ListStatus, LiveOrderState, MarginMode, MissReason, MissedOpportunity,
    OrderIntent, OrderRecord, OrderSide, OrderSource, OrderType, OrderUpdateSource,
    ReviewDataSource, ReviewEnvelope, ReviewLedgerStatus, ReviewPnlEvidence, RiskDecision,
    StrategyKind, TimeInForce, VenueOrderIdentity,
};

const SOURCE: &str = "payload_budget_contract";
const OBSERVED_AT_MS: i64 = 1_780_185_600_000;
const TOTAL_ROWS: usize = 1_000;
const PAGE_ROWS: usize = 100;
const ORDER_LIST_MAX_BYTES: usize = 180 * 1024;
const REVIEW_EXECUTED_MAX_BYTES: usize = 360 * 1024;
const REVIEW_MISSED_MAX_BYTES: usize = 96 * 1024;
const HISTORY_FUNDING_DIFF_MAX_BYTES: usize = 96 * 1024;

#[test]
fn list_api_1000_row_payloads_fit_byte_budget() -> serde_json::Result<()> {
    let orders = order_list_envelope(PAGE_ROWS);
    assert_page_contract(&orders.page, TOTAL_ROWS);
    assert_payload_budget(&orders, ORDER_LIST_MAX_BYTES)?;

    let executed = review_executed_envelope(PAGE_ROWS);
    assert_page_contract(&executed.page, TOTAL_ROWS);
    assert_payload_budget(&executed, REVIEW_EXECUTED_MAX_BYTES)?;

    let missed = review_missed_envelope(PAGE_ROWS);
    assert_page_contract(&missed.page, TOTAL_ROWS);
    assert_payload_budget(&missed, REVIEW_MISSED_MAX_BYTES)?;

    let history = history_funding_diff_response(PAGE_ROWS);
    assert_history_page_contract(&history);
    assert_payload_budget(&history, HISTORY_FUNDING_DIFF_MAX_BYTES)?;

    Ok(())
}

fn assert_payload_budget<T: serde::Serialize>(
    payload: &T,
    max_bytes: usize,
) -> serde_json::Result<()> {
    let bytes = serde_json::to_vec(payload)?;
    assert!(
        bytes.len() <= max_bytes,
        "payload is {} bytes; budget is {} bytes",
        bytes.len(),
        max_bytes
    );
    Ok(())
}

fn assert_page_contract(page: &ListPage, total_rows: usize) {
    assert_eq!(page.limit, PAGE_ROWS);
    assert_eq!(page.max_limit, PAGE_ROWS);
    assert_eq!(page.returned_count, PAGE_ROWS);
    assert_eq!(page.total_rows, total_rows);
    assert!(page.has_more);
    assert_eq!(page.next_cursor.as_deref(), Some("v1:100"));
}

fn assert_history_page_contract(response: &HistoryResponse<FundingDiffRow>) {
    assert_eq!(response.count, PAGE_ROWS);
    assert_eq!(
        response.page.as_ref().map(|page| page.limit),
        Some(PAGE_ROWS)
    );
    assert_eq!(
        response.page.as_ref().map(|page| page.max_limit),
        Some(TOTAL_ROWS)
    );
    assert_eq!(
        response.page.as_ref().map(|page| page.returned_count),
        Some(PAGE_ROWS)
    );
    assert_eq!(response.page.as_ref().map(|page| page.has_more), Some(true));
    assert_eq!(
        response
            .page
            .as_ref()
            .and_then(|page| page.next_cursor.as_deref()),
        Some("v1:100")
    );
}

fn order_list_envelope(rows: usize) -> ListEnvelope<OrderRecord> {
    ListEnvelope::new(
        (0..rows).map(order_record).collect(),
        list_page(),
        ListStatus::Fresh,
        SOURCE,
        OBSERVED_AT_MS,
        Vec::new(),
    )
}

fn review_missed_envelope(rows: usize) -> ReviewEnvelope<MissedOpportunity> {
    ReviewEnvelope::new(
        (0..rows).map(missed_row).collect(),
        OBSERVED_AT_MS,
        7,
        ReviewDataSource::MissedOpportunityStore,
        Some(ReviewLedgerStatus::NoCompleteRows),
        Vec::new(),
    )
    .with_page(list_page(), ListStatus::Fresh, Vec::new())
}

fn review_executed_envelope(rows: usize) -> ReviewEnvelope<ExecutedTrade> {
    ReviewEnvelope::new(
        (0..rows).map(executed_row).collect(),
        OBSERVED_AT_MS,
        7,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::LedgerBacked),
        Vec::new(),
    )
    .with_page(list_page(), ListStatus::Fresh, Vec::new())
}

fn history_funding_diff_response(rows: usize) -> HistoryResponse<FundingDiffRow> {
    HistoryResponse {
        count: rows,
        rows: (0..rows).map(funding_diff_row).collect(),
        page: Some(HistoryPage {
            limit: PAGE_ROWS,
            max_limit: TOTAL_ROWS,
            returned_count: PAGE_ROWS,
            has_more: true,
            next_cursor: Some("v1:100".into()),
        }),
        row_cap: Some(shared_types::RowCapEvidence::lower_bound(
            PAGE_ROWS,
            PAGE_ROWS,
            PAGE_ROWS + 1,
            SOURCE,
        )),
        source: SOURCE.into(),
        observed_at_ms: OBSERVED_AT_MS,
        latest_at_ms: Some(OBSERVED_AT_MS),
        freshness_ms: Some(250),
        problem: None,
        retry_after_ms: None,
        problems: Vec::new(),
        backend_status: HistoryBackendStatus::default(),
        storage_health: None,
    }
}

fn list_page() -> ListPage {
    ListPage {
        limit: PAGE_ROWS,
        max_limit: PAGE_ROWS,
        start_offset: 0,
        returned_count: PAGE_ROWS,
        total_rows: TOTAL_ROWS,
        has_more: true,
        next_cursor: Some("v1:100".into()),
        ..ListPage::default()
    }
}

fn order_record(idx: usize) -> OrderRecord {
    let intent = OrderIntent {
        id: format!("order-{idx}"),
        source: OrderSource::Strategy,
        strategy: Some(StrategyKind::PerpCross),
        mode: shared_types::ExecutionMode::Live,
        exchange: "binance".into(),
        symbol: format!("SYM{idx}USDT"),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0 + idx as f64,
        price: Some(100.0 + idx as f64),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: true,
        margin_mode: MarginMode::Cross,
        leverage: 3.0,
        client_order_id: format!("cl-{idx}"),
        client_order_id_policy: None,
        created_at_ms: OBSERVED_AT_MS + idx as i64,
    };
    let identity = VenueOrderIdentity::from_intent(&intent);
    OrderRecord {
        intent,
        state: LiveOrderState::Accepted,
        risk: Some(RiskDecision::allow(1_000.0)),
        identity,
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some(format!("ex-{idx}")),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: OBSERVED_AT_MS + idx as i64,
    }
}

fn missed_row(idx: usize) -> MissedOpportunity {
    MissedOpportunity {
        id: format!("missed-{idx}"),
        opportunity_id: format!("perp-cross-{idx}"),
        strategy: StrategyKind::PerpCross,
        symbol: format!("SYM{idx}"),
        detected_at_ms: OBSERVED_AT_MS + idx as i64,
        expected_pnl_usd: 12.5 + idx as f64,
        reason: MissReason::PriceMoved,
        detail: "settlement window moved before execution confirmation".into(),
    }
}

fn executed_row(idx: usize) -> ExecutedTrade {
    ExecutedTrade {
        id: format!("executed-{idx}"),
        strategy: StrategyKind::PerpCross,
        symbol: format!("SYM{idx}"),
        long_venue: "binance".into(),
        short_venue: "okx".into(),
        opened_at_ms: OBSERVED_AT_MS + idx as i64,
        closed_at_ms: Some(OBSERVED_AT_MS + idx as i64 + 3_600_000),
        holding_minutes: Some(60),
        gross_pnl_usd: 18.0,
        fee_usd: 2.0,
        funding_usd: 4.0,
        slippage_usd: 1.0,
        net_pnl_usd: 15.0,
        evidence: ReviewPnlEvidence::default(),
        actual_fields: Vec::new(),
        estimated_fields: Vec::new(),
        missing_fields: Vec::new(),
        long_orders: vec![order_record(idx * 2)],
        short_orders: vec![order_record(idx * 2 + 1)],
    }
}

fn funding_diff_row(idx: usize) -> FundingDiffRow {
    FundingDiffRow {
        occurred_at_ms: OBSERVED_AT_MS + idx as i64,
        symbol: format!("SYM{idx}"),
        long_exchange: "binance".into(),
        short_exchange: "okx".into(),
        long_rate_8h: -0.0001,
        short_rate_8h: 0.0002,
        gross_diff_bps: 3.0,
        long_next_funding_ms: OBSERVED_AT_MS + 3_600_000,
        short_next_funding_ms: OBSERVED_AT_MS + 3_600_000,
        window_alignment_minutes: 0,
        long_interval_hours: 8,
        short_interval_hours: 8,
        min_volume_24h: 1_000_000.0,
    }
}
