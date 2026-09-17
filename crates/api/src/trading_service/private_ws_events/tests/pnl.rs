use super::*;

#[tokio::test]
async fn private_fill_delta_flows_to_review_pnl_without_cumulative_double_count() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent("hedge-1-long", "c-long", OrderSide::Buy),
        "e-long",
    );
    seed_accepted_order(
        &service,
        arbitrage_intent("hedge-1-short", "c-short", OrderSide::Sell),
        "e-short",
    );

    mark_all_seeded_orders_filled(&service, 5).await;

    for delta in [
        private_fill("e-long", "long-a", 0.4, 100.0, 0.04, 10),
        private_fill("e-long", "long-b", 0.6, 100.0, 0.06, 11),
        private_fill("e-short", "short-a", 1.0, 103.0, 0.10, 12),
    ] {
        let outcome = service
            .apply_private_ws_event(PrivateWsEvent::Fill(delta))
            .await;
        assert!(outcome.ledger_updated);
    }

    let ledger = service.list_execution_ledger_events();
    assert!(ledger.iter().any(|event| {
        event.event_type == ExecutionLedgerEventType::FillSnapshot
            && event.order.identity.internal_order_id == "hedge-1-long"
    }));
    assert_eq!(
        ledger
            .iter()
            .filter(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
            .count(),
        3
    );

    let rows = review_domain::realized_pnl_by_group(&service.list_orders(), &ledger, 0, 1_000);
    let row = rows.get("hedge-1").expect("review pnl row");

    assert_close(row.buy_notional_usd, 100.0);
    assert_close(row.sell_notional_usd, 103.0);
    assert_close(row.price_pnl_usd, 3.0);
    assert_close(row.fee_usd, 0.20);
    assert_eq!(
        row.evidence.fill_event_ids,
        [
            "fill_event:hedge-1-long:private_ws:long-a",
            "fill_event:hedge-1-long:private_ws:long-b",
            "fill_event:hedge-1-short:private_ws:short-a"
        ]
    );
}

#[tokio::test]
async fn private_fill_delta_flows_to_portfolio_today_pnl() -> anyhow::Result<()> {
    let state = isolated_app_state().await?;
    let service = state.trading_service();
    seed_accepted_order(
        service,
        arbitrage_intent("hedge-2-long", "c2-long", OrderSide::Buy),
        "e2-long",
    );
    seed_accepted_order(
        service,
        arbitrage_intent("hedge-2-short", "c2-short", OrderSide::Sell),
        "e2-short",
    );
    mark_all_seeded_orders_filled(service, 5).await;

    for delta in [
        private_fill("e2-long", "pnl-long-a", 0.5, 100.0, 0.05, 10),
        private_fill("e2-long", "pnl-long-b", 0.5, 100.0, 0.05, 11),
        private_fill("e2-short", "pnl-short-a", 1.0, 103.0, 0.10, 12),
    ] {
        let outcome = service
            .apply_private_ws_event(PrivateWsEvent::Fill(delta))
            .await;
        assert!(outcome.ledger_updated);
    }

    let pnl = crate::services::portfolio_pnl::today(&state, 1_000).await;

    assert_close(pnl.realized_pnl_usd, 2.8);
    assert_eq!(pnl.funding_usd, 0.0);
    assert_close(pnl.fee_rebate_usd, -0.20);
    Ok(())
}

#[tokio::test]
async fn private_funding_delta_flows_to_review_and_portfolio_pnl() -> anyhow::Result<()> {
    let state = isolated_app_state().await?;
    let service = state.trading_service();
    seed_accepted_order(
        service,
        arbitrage_intent_on(
            "hedge-3-long",
            "c3-long",
            OrderSide::Buy,
            "hyperliquid",
            "BTC-USDC",
        ),
        "e3-long",
    );
    seed_accepted_order(
        service,
        arbitrage_intent_on(
            "hedge-3-short",
            "c3-short",
            OrderSide::Sell,
            "okx",
            "BTC-USDT",
        ),
        "e3-short",
    );
    mark_all_seeded_orders_filled(service, 5).await;
    for delta in [
        private_fill_on(FillFixture {
            venue: "hyperliquid",
            exchange_order_id: "e3-long",
            venue_event_id: "funding-long-a",
            quantity: 1.0,
            price: 100.0,
            fee_amount: 0.05,
            occurred_at_ms: 10,
        }),
        private_fill_on(FillFixture {
            venue: "okx",
            exchange_order_id: "e3-short",
            venue_event_id: "funding-short-a",
            quantity: 1.0,
            price: 103.0,
            fee_amount: 0.15,
            occurred_at_ms: 11,
        }),
    ] {
        let outcome = service
            .apply_private_ws_event(PrivateWsEvent::Fill(delta))
            .await;
        assert!(outcome.ledger_updated);
    }

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Funding(PrivateFundingDelta {
            venue: "hyperliquid".into(),
            venue_event_id: "hl-funding:BTC:12".into(),
            coin: "BTC".into(),
            amount: -0.12,
            currency: "USDC".into(),
            occurred_at_ms: 12,
        }))
        .await;

    assert!(outcome.ledger_updated);
    assert!(outcome.account_cache_dirty.is_some());
    let ledger = service.list_execution_ledger_events();
    let rows = review_domain::realized_pnl_by_group(&service.list_orders(), &ledger, 0, 1_000);
    let row = rows.get("hedge-3").expect("review pnl row");
    assert_close(row.price_pnl_usd, 3.0);
    assert_close(row.fee_usd, 0.20);
    assert_close(row.funding_usd, -0.12);
    assert_close(row.net_pnl_usd, 2.68);
    assert_eq!(
        row.evidence.funding_event_ids,
        ["funding_payment:hedge-3-long:private_ws:hl-funding:BTC:12"]
    );

    let pnl = crate::services::portfolio_pnl::today(&state, 1_000).await;
    assert_close(pnl.realized_pnl_usd, 2.68);
    assert_close(pnl.funding_usd, -0.12);
    assert_close(pnl.fee_rebate_usd, -0.20);
    Ok(())
}

#[tokio::test]
async fn funding_and_liquidation_mark_scoped_account_cache_dirty_without_matched_order_ledger() {
    let service = TradingService::new_mock();

    let funding = service
        .apply_private_ws_event(PrivateWsEvent::Funding(PrivateFundingDelta {
            venue: "hyperliquid".into(),
            venue_event_id: "hl-funding:BTC:10".into(),
            coin: "BTC".into(),
            amount: -0.12,
            currency: "USDC".into(),
            occurred_at_ms: 10,
        }))
        .await;
    let liquidation = service
        .apply_private_ws_event(PrivateWsEvent::Liquidation(PrivateLiquidationDelta {
            venue: "hyperliquid".into(),
            venue_event_id: "hl-liquidation:7".into(),
            liquidator: "0xabc".into(),
            liquidated_user: "0xdef".into(),
            notional_position: 100.0,
            account_value: 10.0,
            occurred_at_ms: 11,
        }))
        .await;

    assert!(funding.account_cache_dirty.is_some());
    assert!(liquidation.account_cache_dirty.is_some());
    assert!(!funding.ledger_updated);
    assert_eq!(
        funding.funding_skip_reason,
        Some(shared_types::FundingPaymentIngestSkipReason::NoMatchingOrder)
    );
    assert!(!liquidation.ledger_updated);
    assert!(service.list_execution_ledger_events().is_empty());
}

#[tokio::test]
async fn non_user_cancel_updates_order_by_exchange_order_id() {
    let service = TradingService::new_mock();
    let mut intent = intent("i1", "c1");
    intent.mode = ExecutionMode::Live;
    service.journal.insert_created(intent.clone(), 1);
    service
        .journal
        .mark_risk_checked("i1", shared_types::RiskDecision::allow(50_000.0), 2);
    service.journal.mark_submitted("i1", 3);
    service.journal.apply_ack(&shared_types::OrderAck {
        internal_order_id: intent.id,
        exchange_order_id: Some("e1".into()),
        client_order_id: intent.client_order_id,
        identity_update: Default::default(),
        state: shared_types::LiveOrderState::Accepted,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    });
    let accepted = service.get_order("i1").expect("accepted order");
    service
        .live_order_proof_health
        .record_submit_ack_from_record(&accepted);

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::NonUserCancel(PrivateNonUserCancelDelta {
            venue: "hyperliquid".into(),
            venue_event_id: "hl-non-user-cancel:BTC:e1".into(),
            exchange_order_id: "e1".into(),
            coin: "BTC".into(),
            occurred_at_ms: 12,
        }))
        .await;

    assert!(outcome.account_cache_dirty.is_none());
    assert_eq!(
        outcome.order.as_ref().map(|record| record.state),
        Some(shared_types::LiveOrderState::Cancelled)
    );
    assert!(service.list_execution_ledger_events().iter().any(|event| {
        event.event_type == shared_types::ExecutionLedgerEventType::Cancel
            && event.source == OrderUpdateSource::PrivateWs
            && event.order.identity.exchange_order_id.as_deref() == Some("e1")
    }));
    let proof = service.live_order_proof_health.snapshot(13);
    assert_eq!(proof.len(), 1);
    assert_eq!(proof[0].status, shared_types::VenueOperationStatus::Ok);
    assert_eq!(
        proof[0]
            .cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("private_ws_non_user_cancel")
    );
}

#[tokio::test]
async fn unmatched_non_user_cancel_does_not_dirty_account_without_a_fill() {
    let service = TradingService::new_mock();

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::NonUserCancel(PrivateNonUserCancelDelta {
            venue: "hyperliquid".into(),
            venue_event_id: "hl-non-user-cancel:BTC:missing".into(),
            exchange_order_id: "missing".into(),
            coin: "BTC".into(),
            occurred_at_ms: 12,
        }))
        .await;

    assert!(outcome.account_cache_dirty.is_none());
    assert!(outcome.order.is_none());
    assert!(service.list_execution_ledger_events().is_empty());
}
