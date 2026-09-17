fn record_remote_missing(
    state: &AppState,
    target: &PendingOrderTarget,
    outcome: &mut RunFinalityOutcome,
) {
    let checked_at_ms = common::time::now_ms();
    let problem = finality_remote_missing_problem(target, checked_at_ms);
    outcome.record_remote_missing(target, &problem, checked_at_ms);
    project_run_finality_problem(state, target, &problem, checked_at_ms, outcome);
    debug!(
        order_id = %target.raw_order_id,
        internal_order_id = %target.internal_order_id,
        source = ?target.source,
        "run finality order query returned no remote order"
    );
}

fn record_refresh_failure(
    state: &AppState,
    target: &PendingOrderTarget,
    error: &trading::TradingError,
    outcome: &mut RunFinalityOutcome,
) {
    let checked_at_ms = common::time::now_ms();
    let problem = finality_refresh_failure_problem(target, error, checked_at_ms);
    outcome.record_refresh_failure(target, &problem, checked_at_ms, error.to_string());
    project_run_finality_problem(state, target, &problem, checked_at_ms, outcome);
    warn!(
        %error,
        order_id = %target.raw_order_id,
        internal_order_id = %target.internal_order_id,
        source = ?target.source,
        "run finality order refresh failed"
    );
}

fn project_run_finality_problem(
    state: &AppState,
    target: &PendingOrderTarget,
    problem: &ApiProblem,
    checked_at_ms: i64,
    outcome: &mut RunFinalityOutcome,
) {
    project_execution_finality_problem(state, target, problem, checked_at_ms, outcome);
    project_close_finality_problem(state, target, problem, checked_at_ms, outcome);
}

fn project_execution_finality_problem(
    state: &AppState,
    target: &PendingOrderTarget,
    problem: &ApiProblem,
    checked_at_ms: i64,
    outcome: &mut RunFinalityOutcome,
) {
    for run in crate::services::execution_runs::project_finality_problem(
        state,
        &target.raw_order_id,
        problem,
        checked_at_ms,
    ) {
        if let Err(error) = crate::services::ws_publish::publish_execution_run_event(
            state,
            EXECUTION_RUN_EVENT,
            &run,
        ) {
            outcome.record_publish_failure(target, error.to_string());
            warn!(
                %error,
                order_id = %target.raw_order_id,
                internal_order_id = %target.internal_order_id,
                source = ?target.source,
                "run finality problem publish failed"
            );
        }
    }
}

fn project_close_finality_problem(
    state: &AppState,
    target: &PendingOrderTarget,
    problem: &ApiProblem,
    checked_at_ms: i64,
    outcome: &mut RunFinalityOutcome,
) {
    for run in crate::services::close_runs::project_finality_problem(
        state,
        &target.raw_order_id,
        problem,
        checked_at_ms,
    ) {
        if let Err(error) =
            crate::services::ws_publish::publish_close_run_event(state, CLOSE_RUN_EVENT, &run)
        {
            outcome.record_publish_failure(target, error.to_string());
            warn!(
                %error,
                order_id = %target.raw_order_id,
                internal_order_id = %target.internal_order_id,
                source = ?target.source,
                "close run finality problem publish failed"
            );
        }
    }
}

fn local_order_record(state: &AppState, order_id: &str) -> Option<OrderRecord> {
    state.trading_service().get_order(order_id).or_else(|| {
        state
            .trading_service()
            .get_order_by_client_order_id(order_id)
            .or_else(|| {
                state
                    .trading_service()
                    .get_order_by_exchange_order_id(order_id)
            })
    })
}

fn project_unsubmitted_execution_leg(
    state: &AppState,
    order_ref: &PendingOrderRef,
    outcome: &mut RunFinalityOutcome,
) -> bool {
    let checked_at_ms = common::time::now_ms();
    let projected = crate::services::execution_runs::project_unsubmitted_leg_failure(
        state,
        &order_ref.order_id,
        checked_at_ms,
    );
    for (run, venue) in &projected {
        let target = PendingOrderTarget {
            raw_order_id: order_ref.order_id.clone(),
            internal_order_id: order_ref.order_id.clone(),
            venue: venue.clone(),
            source: order_ref.source,
            state: LiveOrderState::Created,
        };
        outcome.record_scanned(&target);
        match crate::services::ws_publish::publish_execution_run_event(
            state,
            EXECUTION_RUN_EVENT,
            run,
        ) {
            Ok(()) => outcome.record_refreshed(&target),
            Err(error) => outcome.record_publish_failure(&target, error.to_string()),
        }
    }
    !projected.is_empty()
}

fn publish_refreshed_order(
    state: &AppState,
    record: &shared_types::OrderRecord,
    target: &PendingOrderTarget,
    outcome: &mut RunFinalityOutcome,
) {
    match crate::services::ws_publish::publish_order_event(state, ORDER_EVENT, record) {
        Ok(()) => outcome.record_refreshed(target),
        Err(error) => {
            outcome.record_publish_failure(target, error.to_string());
            warn!(
                %error,
                order_id = %target.raw_order_id,
                internal_order_id = %target.internal_order_id,
                source = ?target.source,
                "run finality publish failed"
            );
        }
    }
}

fn publish_terminal_order(
    state: &AppState,
    record: &shared_types::OrderRecord,
    target: &PendingOrderTarget,
    outcome: &mut RunFinalityOutcome,
) {
    match crate::services::ws_publish::publish_order_event(state, ORDER_EVENT, record) {
        Ok(()) => outcome.record_skipped_terminal(target),
        Err(error) => {
            outcome.record_publish_failure(target, error.to_string());
            warn!(
                %error,
                order_id = %target.raw_order_id,
                internal_order_id = %target.internal_order_id,
                source = ?target.source,
                "run finality terminal projection failed"
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
