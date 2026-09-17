use super::*;

#[tokio::test]
async fn private_funding_delta_uses_cross_venue_fill_event_anchors() -> anyhow::Result<()> {
    let service = TradingService::new_mock();

    for (index, case) in funding_venue_cases().into_iter().enumerate() {
        apply_case(&service, index, case).await;
    }

    let ledger = service.list_execution_ledger_events();
    assert_eq!(
        ledger
            .iter()
            .filter(|event| event.event_type == ExecutionLedgerEventType::FundingPayment)
            .count(),
        8
    );
    let funding_total: f64 = ledger
        .iter()
        .filter_map(|event| match &event.payload {
            ExecutionLedgerPayload::FundingPayment(payment) => Some(payment.amount),
            _ => None,
        })
        .sum();
    assert_close(funding_total, -0.36);
    Ok(())
}

async fn apply_case(service: &TradingService, index: usize, case: FundingVenueCase<'_>) {
    let ids = CrossVenueFundingIds::new(index, case.event_venue);
    let funding_amount = -0.01 * (index as f64 + 1.0);
    seed_accepted_order(
        service,
        arbitrage_intent_on(
            &ids.order_id,
            &ids.client_id,
            OrderSide::Buy,
            case.order_venue,
            case.order_symbol,
        ),
        &ids.exchange_order_id,
    );
    apply_fill(service, &ids, case).await;
    apply_funding(service, &ids, case, funding_amount).await;
}

async fn apply_fill(
    service: &TradingService,
    ids: &CrossVenueFundingIds,
    case: FundingVenueCase<'_>,
) {
    let fill = private_fill_on_symbol(
        FillFixture {
            venue: case.event_venue,
            exchange_order_id: &ids.exchange_order_id,
            venue_event_id: &ids.fill_event_id,
            quantity: 1.0,
            price: 100.0,
            fee_amount: 0.01,
            occurred_at_ms: 10,
        },
        case.fill_symbol,
    );
    assert!(
        service
            .apply_private_ws_event(PrivateWsEvent::Fill(fill))
            .await
            .ledger_updated,
        "fill anchor missing for {}",
        case.order_venue
    );
}

async fn apply_funding(
    service: &TradingService,
    ids: &CrossVenueFundingIds,
    case: FundingVenueCase<'_>,
    amount: f64,
) {
    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Funding(PrivateFundingDelta {
            venue: case.event_venue.into(),
            venue_event_id: ids.funding_event_id.clone(),
            coin: case.funding_symbol.into(),
            amount,
            currency: case.currency.into(),
            occurred_at_ms: 20,
        }))
        .await;
    assert!(
        outcome.ledger_updated,
        "funding missing for {}",
        case.order_venue
    );
    assert!(outcome.account_cache_dirty.is_some());
    assert!(service.list_execution_ledger_events().iter().any(|event| {
        event.event_id == ids.expected_ledger_event_id
            && event.source == OrderUpdateSource::PrivateWs
            && event.order.exchange == case.order_venue
    }));
}

struct CrossVenueFundingIds {
    order_id: String,
    client_id: String,
    exchange_order_id: String,
    fill_event_id: String,
    funding_event_id: String,
    expected_ledger_event_id: String,
}

impl CrossVenueFundingIds {
    fn new(index: usize, venue: &str) -> Self {
        let order_id = format!("cross-venue-{index}-long");
        let funding_event_id = format!("{venue}-funding-{index}");
        Self {
            client_id: format!("cross-venue-cid-{index}"),
            exchange_order_id: format!("cross-venue-exchange-{index}"),
            fill_event_id: format!("cross-venue-fill-{index}"),
            expected_ledger_event_id: format!(
                "funding_payment:{order_id}:private_ws:{funding_event_id}"
            ),
            order_id,
            funding_event_id,
        }
    }
}

fn funding_venue_cases() -> [FundingVenueCase<'static>; 7] {
    [
        funding_case(
            "binance", "binance", "BTCUSDT", "BTCUSDT", "BTCUSDT", "USDT",
        ),
        funding_case(
            "okx",
            "okx",
            "BTC-USDT-SWAP",
            "BTC-USDT",
            "BTC-USDT",
            "USDT",
        ),
        funding_case("bybit", "bybit", "BTCUSDT", "BTCUSDT", "BTCUSDT", "USDT"),
        funding_case(
            "bitget",
            "bitget",
            "BTCUSDT_UMCBL",
            "BTCUSDT",
            "BTCUSDT",
            "USDT",
        ),
        funding_case("gate", "gate", "BTC_USDT", "BTC_USDT", "BTC_USDT", "USDT"),
        funding_case(
            "kucoin", "kucoin", "XBTUSDTM", "XBTUSDTM", "XBTUSDTM", "USDT",
        ),
        funding_case(
            "hyperliquid:xyz",
            "hyperliquid",
            "BTC-USDC",
            "BTC",
            "BTC",
            "USDC",
        ),
    ]
}

fn funding_case<'a>(
    order_venue: &'a str,
    event_venue: &'a str,
    order_symbol: &'a str,
    fill_symbol: &'a str,
    funding_symbol: &'a str,
    currency: &'a str,
) -> FundingVenueCase<'a> {
    FundingVenueCase {
        order_venue,
        event_venue,
        order_symbol,
        fill_symbol,
        funding_symbol,
        currency,
    }
}
