use super::*;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OrderListQuery {
    pub(super) limit: Option<usize>,
    pub(super) cursor: Option<String>,
    pub(super) state: Option<String>,
    pub(super) since_ms: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecutionLedgerListQuery {
    pub(super) limit: Option<usize>,
    pub(super) internal_order_id: Option<String>,
    pub(super) exchange_order_id: Option<String>,
    pub(super) hedge_group_id: Option<String>,
    pub(super) run_id: Option<String>,
    pub(super) ticket_id: Option<String>,
    pub(super) leg_role: Option<String>,
    pub(super) from_ms: Option<String>,
    pub(super) to_ms: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecutionRunListQuery {
    pub(super) run_id: Option<String>,
    pub(super) ticket_id: Option<String>,
    pub(super) opportunity_id: Option<String>,
}

pub(super) async fn list_orders(
    State(state): State<AppState>,
    Query(query): Query<OrderListQuery>,
) -> Json<ListEnvelope<OrderRecord>> {
    Json(order_list_envelope(&state, &query))
}

pub(super) async fn list_execution_ledger(
    State(state): State<AppState>,
    Query(query): Query<ExecutionLedgerListQuery>,
) -> Json<ListEnvelope<ExecutionLedgerEvent>> {
    Json(execution_ledger_envelope(&state, &query))
}

pub(super) async fn list_execution_runs(
    State(state): State<AppState>,
    Query(query): Query<ExecutionRunListQuery>,
) -> Json<ListEnvelope<ExecutionRun>> {
    Json(execution_run_envelope(&state, &query))
}

pub(super) async fn list_action_runs(
    State(state): State<AppState>,
) -> Json<shared_types::ActionRunEnvelope> {
    Json(action_runs::recent_envelope(&state))
}

pub(super) fn order_list_envelope(
    state: &AppState,
    query: &OrderListQuery,
) -> ListEnvelope<OrderRecord> {
    let mut problems = Vec::new();
    let limit = order_list_limit(query.limit, &mut problems);
    let offset = order_list_offset(query.cursor.as_deref(), &mut problems);
    let state_filter = order_list_state(query.state.as_deref(), &mut problems);
    let since_ms = order_list_since_ms(query.since_ms.as_deref(), &mut problems);
    let (rows, total_rows) =
        state
            .trading_service()
            .list_orders_page(offset, limit, state_filter, since_ms);
    let returned_count = rows.len();
    let next_offset = offset.saturating_add(returned_count);
    let has_more = next_offset < total_rows;
    let status = if problems.is_empty() {
        ListStatus::Fresh
    } else {
        ListStatus::Degraded
    };
    ListEnvelope::new(
        rows,
        ListPage {
            limit,
            max_limit: ORDER_LIST_MAX_LIMIT,
            start_offset: offset,
            returned_count,
            total_rows,
            has_more,
            next_cursor: has_more.then(|| next_offset.to_string()),
            ..ListPage::default()
        },
        status,
        ORDER_LIST_SOURCE,
        common::time::now_ms(),
        problems,
    )
}

pub(super) fn execution_ledger_envelope(
    state: &AppState,
    query: &ExecutionLedgerListQuery,
) -> ListEnvelope<ExecutionLedgerEvent> {
    let mut problems = Vec::new();
    let limit = execution_ledger_limit(query.limit, &mut problems);
    let from_ms = execution_ledger_timestamp(query.from_ms.as_deref(), "fromMs", &mut problems);
    let to_ms = execution_ledger_timestamp(query.to_ms.as_deref(), "toMs", &mut problems);
    let leg_role = execution_ledger_leg_role(query.leg_role.as_deref(), &mut problems);
    let rows = state
        .trading_service()
        .list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
            internal_order_id: clean_query_token(query.internal_order_id.as_deref()),
            exchange_order_id: clean_query_token(query.exchange_order_id.as_deref()),
            hedge_group_id: clean_query_token(query.hedge_group_id.as_deref()),
            run_id: clean_query_token(query.run_id.as_deref()),
            ticket_id: clean_query_token(query.ticket_id.as_deref()),
            leg_role,
            from_ms,
            to_ms,
            limit,
        });
    let returned_count = rows.len();
    let status = if problems.is_empty() {
        ListStatus::Fresh
    } else {
        ListStatus::Degraded
    };
    ListEnvelope::new(
        rows,
        ListPage {
            limit,
            max_limit: EXECUTION_LEDGER_LIST_MAX_LIMIT,
            start_offset: 0,
            returned_count,
            total_rows: returned_count,
            has_more: false,
            next_cursor: None,
            ..ListPage::default()
        },
        status,
        EXECUTION_LEDGER_LIST_SOURCE,
        common::time::now_ms(),
        problems,
    )
}

pub(super) fn execution_run_query(
    query: &ExecutionRunListQuery,
) -> execution_runs::ExecutionRunQuery {
    execution_runs::ExecutionRunQuery {
        run_id: clean_query_token(query.run_id.as_deref()),
        ticket_id: clean_query_token(query.ticket_id.as_deref()),
        opportunity_id: clean_query_token(query.opportunity_id.as_deref()),
    }
}

pub(super) fn execution_run_envelope(
    state: &AppState,
    query: &ExecutionRunListQuery,
) -> ListEnvelope<ExecutionRun> {
    let query = execution_run_query(query);
    let rows = if query.has_filter() {
        execution_runs::recent_by_query(state, &query)
    } else {
        execution_runs::recent(state)
    };
    let returned_count = rows.len();
    ListEnvelope::new(
        rows,
        ListPage {
            limit: EXECUTION_RUN_LIST_DEFAULT_LIMIT,
            max_limit: EXECUTION_RUN_LIST_MAX_LIMIT,
            start_offset: 0,
            returned_count,
            total_rows: returned_count,
            has_more: false,
            next_cursor: None,
            ..ListPage::default()
        },
        ListStatus::Fresh,
        EXECUTION_RUN_LIST_SOURCE,
        common::time::now_ms(),
        Vec::new(),
    )
}
