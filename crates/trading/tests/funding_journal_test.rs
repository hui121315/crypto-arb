#![allow(clippy::expect_used, clippy::panic)]

use shared_types::{
    ExecutionMode, LiveOrderState, OrderAck, OrderIntent, OrderSide, OrderSource, OrderType,
    OrderUpdateSource, RiskDecision,
};
use trading::{FillLedgerInput, FillOrderIdentity, FundingLedgerInput, OrderJournal};

#[test]
fn record_funding_by_venue_symbol_accepts_cross_venue_fill_event_anchors() {
    let journal = OrderJournal::new_with_storage_paths(None, None);

    for (index, case) in funding_cases().into_iter().enumerate() {
        let order_id = format!("hedge-{index}-long");
        let exchange_order_id = format!("x-{index}");
        let intent = arbitrage_intent(
            &order_id,
            &format!("c{index}"),
            case.order_venue,
            case.order_symbol,
        );
        submit_accepted_order(&journal, intent, &exchange_order_id);
        record_fill_anchor(&journal, &case, &exchange_order_id, index);

        let event = journal
            .record_funding_by_venue_symbol_reported(
                case.event_venue,
                case.funding_symbol,
                &FundingLedgerInput {
                    venue_event_id: format!("{}-funding-{index}", case.event_venue),
                    amount: -0.12,
                    currency: case.currency.to_owned(),
                    funding_time_ms: 10,
                },
                OrderUpdateSource::PrivateWs,
                11,
            )
            .expect("funding ledger event");

        assert_eq!(event.order.identity.internal_order_id, order_id);
        assert_eq!(event.order.exchange, case.order_venue);
    }
}

fn submit_accepted_order(journal: &OrderJournal, intent: OrderIntent, exchange_order_id: &str) {
    journal.insert_created(intent.clone(), 1);
    journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
        .expect("risk checked");
    journal.mark_submitted(&intent.id, 3).expect("submitted");
    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some(exchange_order_id.to_owned()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("accepted");
}

fn record_fill_anchor(
    journal: &OrderJournal,
    case: &FundingCase,
    exchange_order_id: &str,
    index: usize,
) {
    let events = journal.record_fill_by_order_identity(
        FillOrderIdentity {
            venue: Some(case.event_venue),
            exchange_order_id: Some(exchange_order_id),
            client_order_id: None,
            symbol: Some(case.funding_symbol),
            side: Some(OrderSide::Buy),
        },
        &FillLedgerInput {
            venue_event_id: format!("{}-fill-{index}", case.event_venue),
            quantity: 0.25,
            price: 11.0,
            fee_amount: Some(0.02),
            fee_currency: Some(case.currency.to_owned()),
            occurred_at_ms: 8,
        },
        OrderUpdateSource::PrivateWs,
        9,
    );
    assert!(!events.is_empty(), "fill ledger event");
}

#[derive(Clone, Copy)]
struct FundingCase {
    order_venue: &'static str,
    event_venue: &'static str,
    order_symbol: &'static str,
    funding_symbol: &'static str,
    currency: &'static str,
}

fn funding_cases() -> [FundingCase; 7] {
    [
        funding_case("binance", "binance", "BTCUSDT", "BTCUSDT", "USDT"),
        funding_case("okx", "okx", "BTC-USDT-SWAP", "BTC-USDT", "USDT"),
        funding_case("bybit", "bybit", "BTCUSDT", "BTCUSDT", "USDT"),
        funding_case("bitget", "bitget", "BTCUSDT_UMCBL", "BTCUSDT", "USDT"),
        funding_case("gate", "gate", "BTC_USDT", "BTC_USDT", "USDT"),
        funding_case("kucoin", "kucoin", "XBTUSDTM", "XBTUSDTM", "USDT"),
        funding_case("hyperliquid:xyz", "hyperliquid", "BTC-USDC", "BTC", "USDC"),
    ]
}

fn funding_case(
    order_venue: &'static str,
    event_venue: &'static str,
    order_symbol: &'static str,
    funding_symbol: &'static str,
    currency: &'static str,
) -> FundingCase {
    FundingCase {
        order_venue,
        event_venue,
        order_symbol,
        funding_symbol,
        currency,
    }
}

fn arbitrage_intent(id: &str, client_id: &str, exchange: &str, symbol: &str) -> OrderIntent {
    OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(shared_types::StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: exchange.into(),
        symbol: symbol.into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(10.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: client_id.into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
