use super::*;

#[tokio::test]
async fn append_and_query_api_health_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    store
        .append_api_health(&[
            api_health_sample("binance", "/fapi/v1/premiumIndex", "ok", 1_000),
            api_health_sample("okx", "/api/v5/public/funding-rate", "rate_limited", 2_000),
            api_health_sample("binance", "/fapi/v1/ticker", "error", 3_000),
        ])
        .await?;

    let binance = store
        .query_api_health(ApiHealthQuery {
            exchange: Some("BINANCE".into()),
            limit: 10,
            ..ApiHealthQuery::default()
        })
        .await?;
    assert_eq!(binance.len(), 2);
    assert!(binance.iter().all(|row| row.exchange == "binance"));
    assert_eq!(binance[0].occurred_at_ms, 3_000, "newest first");

    let throttled = store
        .query_api_health(ApiHealthQuery {
            outcome: Some("rate_limited".into()),
            limit: 10,
            ..ApiHealthQuery::default()
        })
        .await?;
    assert_eq!(throttled.len(), 1);
    assert_eq!(throttled[0].exchange, "okx");
    assert_eq!(throttled[0].retry_after_ms, Some(750));
    assert_eq!(throttled[0].circuit_state.as_deref(), Some("half_open"));
    Ok(())
}

#[tokio::test]
async fn events_query_threads_full_correlation_chain() -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(10);
    store
        .append_events(&[
            ledger_event("evt-1", "execution", "submit_order", 1_000),
            ledger_event("evt-2", "execution", "cancel_order", 2_000),
        ])
        .await?;

    let cases: [(&str, EventQuery); 5] = [
        (
            "request_id",
            EventQuery {
                request_id: Some("req-evt-1".into()),
                limit: 10,
                ..EventQuery::default()
            },
        ),
        (
            "run_id",
            EventQuery {
                run_id: Some("run-evt-1".into()),
                limit: 10,
                ..EventQuery::default()
            },
        ),
        (
            "ticket_id",
            EventQuery {
                ticket_id: Some("ticket-evt-1".into()),
                limit: 10,
                ..EventQuery::default()
            },
        ),
        (
            "client_order_id",
            EventQuery {
                client_order_id: Some("client-evt-1".into()),
                limit: 10,
                ..EventQuery::default()
            },
        ),
        (
            "exchange_order_id",
            EventQuery {
                exchange_order_id: Some("exch-evt-1".into()),
                limit: 10,
                ..EventQuery::default()
            },
        ),
    ];
    for (field, query) in cases {
        let rows = store.query_events(query).await?;
        assert_eq!(rows.len(), 1, "{field} filter should isolate one event");
        assert_eq!(rows[0].event_id, "evt-1", "{field} filter matched event");
    }

    let by_action = store
        .query_events(EventQuery {
            action: Some("cancel_order".into()),
            limit: 10,
            ..EventQuery::default()
        })
        .await?;
    assert_eq!(by_action.len(), 1);
    assert_eq!(by_action[0].event_id, "evt-2");
    Ok(())
}

#[tokio::test]
async fn approximate_row_count_includes_observability_tables(
) -> Result<(), Box<dyn std::error::Error>> {
    let store = HistoryStore::new(100);
    store
        .append_api_health(&[api_health_sample("binance", "/fapi/v1/ticker", "ok", 1_000)])
        .await?;
    store
        .append_events(&[ledger_event("evt-1", "execution", "submit_order", 1_000)])
        .await?;
    assert_eq!(store.approximate_row_count().await, Some(2));
    Ok(())
}
