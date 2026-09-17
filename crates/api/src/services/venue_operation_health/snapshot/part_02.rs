fn resolve_completed_account_refetches(
    snapshots: &mut [PrivateWsRuntimeHealth],
    balance_cache: &[AccountCacheSnapshot],
    position_cache: &[AccountCacheSnapshot],
    now_ms: i64,
) {
    for snapshot in snapshots {
        let Some(dirty) = snapshot.account_dirty.as_ref() else {
            continue;
        };
        let dirty_at_ms = snapshot
            .last_problem_at_ms
            .unwrap_or(snapshot.observed_at_ms);
        let Some((completed_at_ms, rows)) =
            account_refetch_completion(dirty, dirty_at_ms, balance_cache, position_cache)
        else {
            continue;
        };
        snapshot.status = VenueOperationStatus::Ok;
        snapshot.message = format!(
            "私有 WS 账户变更已由有界 REST 补拉同步：venue={}; scope={}",
            dirty.venue,
            dirty.scope.as_str()
        );
        snapshot.requested = Some(1);
        snapshot.rows = Some(rows);
        snapshot.freshness_ms = Some(now_ms.saturating_sub(completed_at_ms));
        snapshot.error = None;
        snapshot.account_dirty = None;
        snapshot.observed_at_ms = completed_at_ms;
    }
}

fn account_refetch_completion(
    dirty: &crate::trading_service::private_ws_events::PrivateAccountDirty,
    dirty_at_ms: i64,
    balance_cache: &[AccountCacheSnapshot],
    position_cache: &[AccountCacheSnapshot],
) -> Option<(i64, u64)> {
    let mut completed_at_ms = dirty_at_ms;
    let mut rows = 0_u64;
    if dirty.scope.invalidates_balances() {
        let snapshot = fresh_account_cache_after(balance_cache, &dirty.venue, dirty_at_ms)?;
        completed_at_ms = completed_at_ms.max(snapshot.observed_at_ms);
        rows = rows.saturating_add(snapshot.rows);
    }
    if dirty.scope.invalidates_positions() {
        let snapshot = fresh_account_cache_after(position_cache, &dirty.venue, dirty_at_ms)?;
        completed_at_ms = completed_at_ms.max(snapshot.observed_at_ms);
        rows = rows.saturating_add(snapshot.rows);
    }
    Some((completed_at_ms, rows))
}

fn fresh_account_cache_after<'a>(
    snapshots: &'a [AccountCacheSnapshot],
    venue: &str,
    dirty_at_ms: i64,
) -> Option<&'a AccountCacheSnapshot> {
    let venue = normalized_venue_name(venue);
    snapshots.iter().find(|snapshot| {
        normalized_venue_name(&snapshot.venue) == venue
            && snapshot.quality == AccountCacheQuality::Fresh
            && snapshot.observed_at_ms >= dirty_at_ms
    })
}

fn private_ws_runtime_rows(
    credentials: &[VenueCredentialStatus],
    snapshots: Vec<PrivateWsRuntimeHealth>,
    orders: &[OrderRecord],
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    let ws_venues = exchange::trading_ws_venues().venues;
    let mut by_key = snapshots
        .into_iter()
        .map(|snapshot| {
            (
                (
                    normalized_venue_name(&snapshot.venue),
                    snapshot.operation.to_owned(),
                ),
                snapshot,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::with_capacity(credentials.len().saturating_mul(PRIVATE_WS_OPS.len()));
    for venue in credentials {
        let venue_key = normalized_venue_name(&venue.venue);
        for operation in PRIVATE_WS_OPS {
            let key = (venue_key.clone(), operation.to_owned());
            let row = match by_key.remove(&key) {
                Some(snapshot) => private_ws_runtime_row(snapshot, Some(venue), &ws_venues),
                None => private_ws_missing_row(venue, operation, observed_at_ms, &ws_venues),
            };
            rows.push(row);
        }
    }
    rows.extend(
        by_key
            .into_values()
            .map(|snapshot| private_ws_runtime_row(snapshot, None, &ws_venues)),
    );
    apply_order_stream_probes(&mut rows, credentials, orders, observed_at_ms, &ws_venues);
    rows
}

fn overlay_order_stream_readiness_from_live_proof(
    rows: &mut [VenueOperationHealth],
    proofs: &[LiveOrderProofRuntimeHealth],
) {
    for proof in proofs {
        let Some(cancel_proof) = credential_bound_private_ws_cancel_proof(proof) else {
            continue;
        };
        let venue = normalized_venue_name(&proof.venue);
        let Some((observed_at_ms, freshness_ms)) = current_private_ws_readiness(rows, &venue)
        else {
            continue;
        };
        let Some(row) = rows.iter_mut().find(|row| {
            normalized_venue_name(&row.venue) == venue
                && row.operation == OP_PRIVATE_WS_ORDER_STREAM
                && row.status == VenueOperationStatus::Unknown
                && row.supported == Some(true)
                && row.configured == Some(true)
        }) else {
            continue;
        };

        row.status = VenueOperationStatus::Ok;
        row.message =
            "私有 WS 当前会话已连接并确认订阅；同凭证订单事件已有持久化证明，等待本次会话新事件"
                .to_owned();
        row.requested = Some(1);
        row.rows = Some(1);
        row.freshness_ms = freshness_ms;
        row.retry_after_ms = None;
        row.error = None;
        row.problem = None;
        row.observed_at_ms = observed_at_ms;
        if let Some(evidence) = row.evidence.as_mut() {
            evidence
                .request_context
                .push("stream_readiness=current_session_plus_credential_bound_event_proof".to_owned());
            evidence.request_context.push(format!(
                "historical_order_event_source={}",
                cancel_proof.source
            ));
            evidence.request_context.push(format!(
                "historical_order_event_checked_at_ms={}",
                cancel_proof.checked_at_ms
            ));
            evidence.request_context.push(format!(
                "historical_order_event_internal_order_id={}",
                cancel_proof.internal_order_id
            ));
        }
    }
}

fn credential_bound_private_ws_cancel_proof(
    proof: &LiveOrderProofRuntimeHealth,
) -> Option<&LiveOrderProofSample> {
    (proof.status == VenueOperationStatus::Ok)
        .then_some(proof.cancel_finality.as_ref())
        .flatten()
        .filter(|sample| sample.source.starts_with("private_ws"))
}

fn current_private_ws_readiness(
    rows: &[VenueOperationHealth],
    venue: &str,
) -> Option<(i64, Option<i64>)> {
    let ready_row = |operation: &str| {
        rows.iter().find(|row| {
            normalized_venue_name(&row.venue) == venue
                && row.operation == operation
                && row.status == VenueOperationStatus::Ok
        })
    };
    let session = ready_row(OP_PRIVATE_WS_SESSION)?;
    let subscription = ready_row(OP_PRIVATE_WS_SUBSCRIBE)?;
    let observed_at_ms = session.observed_at_ms.max(subscription.observed_at_ms);
    let freshness_ms = [session.freshness_ms, subscription.freshness_ms]
        .into_iter()
        .flatten()
        .min();
    Some((observed_at_ms, freshness_ms))
}

fn order_write_runtime_rows(
    credentials: &[VenueCredentialStatus],
    snapshots: Vec<LiveOrderProofRuntimeHealth>,
    observed_at_ms: i64,
) -> Vec<VenueOperationHealth> {
    let mut by_venue = snapshots
        .into_iter()
        .map(|snapshot| (normalized_venue_name(&snapshot.venue), snapshot))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::with_capacity(credentials.len().saturating_add(by_venue.len()));
    for venue in credentials {
        let snapshot = by_venue.remove(&normalized_venue_name(&venue.venue));
        rows.push(order_write_runtime_row_for_credential(
            venue,
            snapshot,
            observed_at_ms,
        ));
    }
    rows.extend(
        by_venue
            .into_values()
            .map(|snapshot| order_write_runtime_row(&snapshot, None)),
    );
    rows
}

fn order_write_runtime_row_for_credential(
    venue: &VenueCredentialStatus,
    snapshot: Option<LiveOrderProofRuntimeHealth>,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    if !venue.live_write || !credentials_configured(venue) {
        return credential_row(venue, OP_ORDER_WRITE, venue.live_write, observed_at_ms);
    }
    match snapshot {
        Some(snapshot) => order_write_runtime_row(&snapshot, Some(venue)),
        None => order_write_missing_live_proof_row(venue, observed_at_ms),
    }
}

fn order_write_runtime_row(
    snapshot: &LiveOrderProofRuntimeHealth,
    credential: Option<&VenueCredentialStatus>,
) -> VenueOperationHealth {
    let snapshot = order_write_capability_snapshot(snapshot);
    let message = snapshot.message.clone();
    let problem = live_order_proof_problem(&snapshot, &message);
    VenueOperationHealth {
        venue: snapshot.venue.clone(),
        operation: OP_ORDER_WRITE.to_owned(),
        status: snapshot.status,
        source: SOURCE_LIVE_ORDER_PROOF_RUNTIME.to_owned(),
        message,
        supported: credential.map(|venue| venue.live_write),
        configured: credential.map(credentials_configured),
        requested: snapshot.requested,
        rows: snapshot.rows,
        freshness_ms: snapshot.freshness_ms,
        retry_after_ms: snapshot.retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: snapshot.error.clone(),
        evidence: Some(live_order_proof_evidence(&snapshot)),
        problem,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

fn order_write_capability_snapshot(
    snapshot: &LiveOrderProofRuntimeHealth,
) -> LiveOrderProofRuntimeHealth {
    let mut projected = snapshot.clone();
    projected.requested = Some(1);
    projected.rows = Some(u64::from(snapshot.place_proof.is_some()));

    if let Some(problem) = current_submit_problem(snapshot) {
        projected.status = VenueOperationStatus::Blocked;
        projected.message = format!("live 下单远程证明失败：{}", problem.message);
        projected.request_id = problem.request_id.clone();
        projected.retry_after_ms = problem.retry_after_ms;
        projected.error = Some(problem.message.clone());
        projected.observed_at_ms = problem.observed_at_ms;
        projected.freshness_ms = projected_freshness_ms(snapshot, problem.observed_at_ms);
        return projected;
    }

    let Some(place) = snapshot.place_proof.as_ref() else {
        projected.status = VenueOperationStatus::Unknown;
        projected.message = "尚未取得 live 下单 ack 远程证明样本".to_owned();
        projected.request_id = None;
        projected.retry_after_ms = None;
        projected.error = None;
        return projected;
    };

    projected.status = VenueOperationStatus::Ok;
    projected.message =
        "live 下单 ack 已证明；撤单请求与终态证据保留在详情，不阻断下单能力".to_owned();
    projected.request_id = place.request_id.clone();
    projected.retry_after_ms = None;
    projected.error = None;
    projected.observed_at_ms = place.checked_at_ms;
    projected.freshness_ms = projected_freshness_ms(snapshot, place.checked_at_ms);
    projected
}

fn current_submit_problem(
    snapshot: &LiveOrderProofRuntimeHealth,
) -> Option<&crate::services::live_order_proof_health::LiveOrderProofProblem> {
    snapshot.last_problem.as_ref().filter(|problem| {
        is_submit_problem_source(&problem.source)
            && snapshot
                .place_proof
                .as_ref()
                .is_none_or(|place| problem.observed_at_ms > place.checked_at_ms)
    })
}

fn is_submit_problem_source(source: &str) -> bool {
    matches!(source, "submit_order" | "submit_unwind_order")
        || source.ends_with(".submit_order")
        || source.ends_with(".submit_unwind_order")
}

fn projected_freshness_ms(
    snapshot: &LiveOrderProofRuntimeHealth,
    observed_at_ms: i64,
) -> Option<i64> {
    snapshot.freshness_ms.map(|freshness_ms| {
        snapshot
            .observed_at_ms
            .saturating_add(freshness_ms)
            .saturating_sub(observed_at_ms)
            .max(0)
    })
}

fn order_write_missing_live_proof_row(
    venue: &VenueCredentialStatus,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let snapshot = LiveOrderProofRuntimeHealth {
        venue: venue.venue.clone(),
        status: VenueOperationStatus::Unknown,
        message: "尚未取得 live 下单/撤单远程证明样本".to_owned(),
        request_id: None,
        requested: Some(2),
        rows: Some(0),
        freshness_ms: None,
        retry_after_ms: None,
        error: None,
        observed_at_ms,
        place_proof: None,
        cancel_request: None,
        cancel_finality: None,
        last_problem: None,
        place_ack_count: 0,
        cancel_requested_count: 0,
        cancel_finality_count: 0,
    };
    order_write_runtime_row(&snapshot, Some(venue))
}

fn overlay_order_write_runtime_rows(
    rows: &mut Vec<VenueOperationHealth>,
    runtime_rows: Vec<VenueOperationHealth>,
) {
    for runtime_row in runtime_rows {
        let venue_key = normalized_venue_name(&runtime_row.venue);
        if let Some(row) = rows.iter_mut().find(|row| {
            normalized_venue_name(&row.venue) == venue_key && row.operation == OP_ORDER_WRITE
        }) {
            *row = runtime_row;
        } else {
            rows.push(runtime_row);
        }
    }
}
