use super::*;

pub(super) async fn query(
    client: &tokio_postgres::Client,
    from_ms: i64,
    to_ms: i64,
    hot_close_runs: &[CloseRun],
) -> Result<SqlRealizedWindow, String> {
    if to_ms <= from_ms {
        return Ok(SqlRealizedWindow::default());
    }
    let mut runs = BTreeMap::new();
    for run in query_sql_realized_close_runs(client, to_ms)
        .await?
        .into_iter()
        .chain(hot_close_runs.iter().cloned())
    {
        upsert_latest_close_run(&mut runs, run);
    }
    let close_runs = runs.into_values().collect::<Vec<_>>();
    let limit = SQL_LEDGER_REPLAY_LIMIT + 1;
    let seeds = client
        .query(
            "SELECT internal_order_id FROM order_events \
         WHERE occurred_at_ms >= $1 AND occurred_at_ms < $2 \
         AND event_type IN ('fill_snapshot', 'fill_event') \
         GROUP BY internal_order_id ORDER BY internal_order_id LIMIT $3",
            &[&from_ms, &to_ms, &limit],
        )
        .await
        .map_err(|error| format!("realized window seeds failed: {error}"))?;
    check_limit(seeds.len())?;
    let mut ids = seeds
        .iter()
        .map(|row| row.get::<_, String>("internal_order_id"))
        .collect::<BTreeSet<_>>();
    let keys = crate::ledger::close_window_link_keys(&close_runs, from_ms, to_ms);
    if !keys.is_empty() {
        let (run_ids, ticket_ids): (Vec<_>, Vec<_>) = keys.into_iter().unzip();
        let linked = client
            .query(
                "SELECT DISTINCT internal_order_id FROM fills \
             WHERE (run_id, ticket_id) IN (SELECT * FROM unnest($1::text[], $2::text[])) \
             AND occurred_at_ms < $3 ORDER BY internal_order_id LIMIT $4",
                &[&run_ids, &ticket_ids, &to_ms, &limit],
            )
            .await
            .map_err(|error| format!("realized linked opening query failed: {error}"))?;
        check_limit(linked.len())?;
        ids.extend(
            linked
                .iter()
                .map(|row| row.get::<_, String>("internal_order_id")),
        );
    }
    if ids.is_empty() {
        return Ok(SqlRealizedWindow::default());
    }
    // Recover each selected group's earlier fills and costs by the indexed order IDs.
    let groups = ids
        .iter()
        .map(|id| crate::ledger::hedge_group_id(id))
        .collect::<BTreeSet<_>>();
    for group in groups {
        for suffix in ["", "-long", "-short", "-unwind"] {
            ids.insert(format!("{group}{suffix}"));
        }
    }
    check_limit(ids.len())?;
    let ids = ids.into_iter().collect::<Vec<_>>();
    let rows = client.query(
        "SELECT payload FROM order_events WHERE internal_order_id = ANY($1) \
         AND occurred_at_ms < $2 \
         AND event_type IN ('fill_snapshot', 'fill_event', 'fee_snapshot', 'funding_payment', 'slippage', 'orderbook_evidence') \
         ORDER BY occurred_at_ms DESC, id DESC LIMIT $3",
        &[&ids, &to_ms, &limit],
    ).await.map_err(|error| format!("realized linked history query failed: {error}"))?;
    check_limit(rows.len())?;
    let events = rows
        .iter()
        .map(sql_event_payload_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let events = ExecutionLedger::from_events(events).realized_window_events_with_close_runs(
        from_ms,
        to_ms,
        &close_runs,
    );
    let order_snapshots = query_sql_realized_order_snapshots(client, &events).await?;
    let keys = realized_close_run_link_keys(&events);
    let close_runs = close_runs
        .into_iter()
        .filter(|run| close_run_matches_keys(run, &keys))
        .collect();
    Ok(SqlRealizedWindow {
        events,
        order_snapshots,
        close_runs,
    })
}

pub(super) fn check_limit(count: usize) -> Result<(), String> {
    if count > SQL_LEDGER_REPLAY_LIMIT as usize {
        return Err(
            "realized history exceeds the bounded query limit; incomplete totals were not returned"
                .into(),
        );
    }
    Ok(())
}
