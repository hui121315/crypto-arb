#[test]
fn kucoin_funding_symbols_are_live_windowed_deduplicated_and_bounded() -> ExchangeResult<()> {
    let service = TradingService::new_mock();
    service
        .position_cache
        .replace("kucoin", service.account_cache_epoch(), Vec::new());
    seed_filled_order(&service, "kucoin-a", "kucoin", "ethusdtm", 110);
    seed_filled_order(&service, "kucoin-b", "KUCOIN:UM", "BTC", 120);
    seed_filled_order(&service, "kucoin-c", "kucoin", "ETHUSDTM", 130);
    seed_filled_order(&service, "outside", "kucoin", "SOL", 99);
    seed_filled_order(&service, "other", "binance", "DOGE", 140);

    let symbols = service.kucoin_funding_symbols(Some(100), Some(200))?;

    assert_eq!(symbols, vec!["BTC", "ETHUSDTM"]);
    Ok(())
}

#[test]
fn kucoin_funding_symbols_include_old_non_reduce_anchor_when_cache_is_missing() -> ExchangeResult<()>
{
    let service = TradingService::new_mock();
    seed_filled_order(&service, "startup-anchor", "kucoin", "OLD", 10);

    let symbols = service.kucoin_funding_symbols(Some(100), Some(200))?;

    assert_eq!(symbols, vec!["OLD"]);
    Ok(())
}

#[test]
fn kucoin_funding_symbols_include_fresh_position_for_old_filled_anchor() -> ExchangeResult<()> {
    let service = TradingService::new_mock();
    seed_filled_order(&service, "old-kucoin", "kucoin", "OLD", 10);
    service.position_cache.replace(
        "kucoin",
        service.account_cache_epoch(),
        vec![position_row("OLD")],
    );

    let symbols = service.kucoin_funding_symbols(Some(100), Some(200))?;

    assert_eq!(symbols, vec!["OLD"]);
    Ok(())
}

#[tokio::test]
async fn kucoin_funding_symbol_overflow_is_reported_without_partial_fetch() {
    let service = TradingService::new_mock();
    for index in 0..=live_adapters::KUCOIN_FUNDING_SYMBOL_LIMIT {
        seed_filled_order(
            &service,
            &format!("startup-{index}"),
            "kucoin",
            &format!("K{index}"),
            50,
        );
    }
    let credentials = AdapterCredentials {
        kucoin_live: Some(("key".into(), "secret".into(), "passphrase".into())),
        ..AdapterCredentials::default()
    };

    let batch = service
        .ingest_configured_private_funding_payments(credentials, Some(100), Some(200))
        .await;

    assert!(batch.ledger_events.is_empty());
    assert!(batch
        .report
        .fetch_error
        .as_deref()
        .is_some_and(|error| error.contains("exceeds bounded fanout limit 32")));
}

fn seed_filled_order(
    service: &TradingService,
    id: &str,
    exchange: &str,
    symbol: &str,
    timestamp_ms: i64,
) {
    let intent = OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::Live,
        exchange: exchange.into(),
        symbol: symbol.into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: timestamp_ms,
    };
    service.journal.insert_created(intent, timestamp_ms);
    let _ = service
        .journal
        .mark_risk_checked(id, RiskDecision::allow(100.0), timestamp_ms);
    let _ = service.journal.mark_submitted(id, timestamp_ms);
    let _ = service.journal.apply_ack(&OrderAck {
        internal_order_id: id.into(),
        exchange_order_id: Some(format!("exchange-{id}")),
        client_order_id: format!("client-{id}"),
        identity_update: Default::default(),
        state: LiveOrderState::Filled,
        accepted_at_ms: timestamp_ms,
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(100.0),
        filled_fee: Some(0.01),
    });
}

fn position_row(symbol: &str) -> PositionInfo {
    PositionInfo {
        symbol: symbol.into(),
        exchange: "kucoin".into(),
        side: "long".into(),
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 101.0,
        unrealized_pnl: 1.0,
        leverage: 2.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 50.0,
        maintenance_margin_ratio: 0.01,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}
