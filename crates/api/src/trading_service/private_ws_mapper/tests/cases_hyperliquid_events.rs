use super::super::*;
use super::fixtures_a::*;
use shared_types::FundingPaymentData;

#[test]
fn hyperliquid_user_fill_maps_to_private_fill_delta() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Fill(vec![
        hyperliquid_fill("12345", "0xabc"),
    ]));

    let fill = events.iter().find_map(|event| match event {
        PrivateWsEvent::FillWithEvidence(fill) => Some(&fill.fill),
        _ => None,
    });
    assert_eq!(
        fill.map(|fill| fill.exchange_order_id.as_str()),
        Some("12345")
    );
    assert_eq!(fill.map(|fill| fill.venue.as_str()), Some("hyperliquid"));
    assert_eq!(fill.and_then(|fill| fill.symbol.as_deref()), Some("BTC"));
    assert_eq!(fill.and_then(|fill| fill.side), Some(OrderSide::Buy));
    assert!(fill
        .and_then(|fill| fill.client_order_id.as_deref())
        .is_none());
    assert_eq!(
        fill.map(|fill| fill.venue_event_id.as_str()),
        Some("hyperliquid_fill:hyperliquid:12345:BTC:42:456")
    );
    assert_eq!(fill.map(|fill| fill.quantity), Some(0.25));
    assert_eq!(fill.map(|fill| fill.price), Some(100.0));
    assert_eq!(fill.and_then(|fill| fill.fee_amount), Some(0.01));
    assert_eq!(
        fill.and_then(|fill| fill.fee_currency.as_deref()),
        Some("USDC")
    );
    assert_eq!(fill.map(|fill| fill.occurred_at_ms), Some(42));
    let evidence = events.iter().find_map(|event| match event {
        PrivateWsEvent::FillWithEvidence(fill) => {
            fill.transport_metadata.venue_fill_evidence.as_ref()
        }
        _ => None,
    });
    assert_eq!(
        evidence.map(|evidence| evidence.venue_closed_pnl.as_str()),
        Some("0")
    );
    assert!(evidence
        .and_then(|evidence| evidence.liquidation.as_ref())
        .is_none());
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::AccountDirty(_))));
}

#[test]
fn hyperliquid_user_funding_maps_to_typed_account_event() {
    let events = map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Funding(vec![
        hyperliquid_funding("BTC", -0.12, -0.0001, 42),
    ]));

    let funding = events.iter().find_map(|event| match event {
        PrivateWsEvent::Funding(funding) => Some(funding),
        _ => None,
    });
    assert_eq!(
        funding.map(|funding| funding.venue_event_id.as_str()),
        Some("hyperliquid_funding:hyperliquid:BTC:42")
    );
    assert_eq!(
        funding.map(|funding| funding.venue.as_str()),
        Some("hyperliquid")
    );
    assert_eq!(funding.map(|funding| funding.coin.as_str()), Some("BTC"));
    assert_eq!(funding.map(|funding| funding.amount), Some(-0.12));
    assert_eq!(
        funding.map(|funding| funding.currency.as_str()),
        Some("USDC")
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event, PrivateWsEvent::AccountDirty(_))));
}

#[test]
fn hyperliquid_dex_scoped_fill_and_funding_keep_their_venue() {
    let mut fill = hyperliquid_fill("12345", "0xabc");
    fill.venue = "hyperliquid:xyz".to_owned();
    let fill_events =
        map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Fill(vec![
            fill,
        ]));
    let mapped_fill = fill_events.iter().find_map(|event| match event {
        PrivateWsEvent::FillWithEvidence(fill) => Some(&fill.fill),
        _ => None,
    });
    assert_eq!(
        mapped_fill.map(|fill| fill.venue.as_str()),
        Some("hyperliquid:xyz")
    );
    assert_eq!(
        mapped_fill.map(|fill| fill.venue_event_id.as_str()),
        Some("hyperliquid_fill:hyperliquid:xyz:12345:BTC:42:456")
    );

    let mut funding = hyperliquid_funding("BTC", -0.12, -0.0001, 42);
    funding.venue = "hyperliquid:xyz".to_owned();
    let funding_events =
        map_hyperliquid_event(hyperliquid_ws_user::HyperliquidUserWsEvent::Funding(vec![
            funding,
        ]));
    let mapped_funding = funding_events.iter().find_map(|event| match event {
        PrivateWsEvent::Funding(funding) => Some(funding),
        _ => None,
    });
    assert_eq!(
        mapped_funding.map(|funding| funding.venue.as_str()),
        Some("hyperliquid:xyz")
    );
    assert_eq!(
        mapped_funding.map(|funding| funding.venue_event_id.as_str()),
        Some("hyperliquid_funding:hyperliquid:xyz:BTC:42")
    );
}

#[test]
fn private_rest_funding_payment_maps_to_private_delta() {
    let delta = funding_payment_delta(FundingPaymentData {
        venue: "binance".to_owned(),
        symbol: "BTC".to_owned(),
        amount: -0.375,
        currency: "USDT".to_owned(),
        funding_time_ms: 1_570_608_000_000,
        venue_event_id: "binance_funding:9689322392".to_owned(),
    });

    assert_eq!(
        delta.as_ref().map(|row| row.venue.as_str()),
        Some("binance")
    );
    assert_eq!(
        delta.as_ref().map(|row| row.venue_event_id.as_str()),
        Some("binance_funding:9689322392")
    );
    assert_eq!(delta.as_ref().map(|row| row.coin.as_str()), Some("BTC"));
    assert_eq!(delta.as_ref().map(|row| row.amount), Some(-0.375));
    assert_eq!(
        delta.as_ref().map(|row| row.currency.as_str()),
        Some("USDT")
    );
    assert_eq!(
        delta.as_ref().map(|row| row.occurred_at_ms),
        Some(1_570_608_000_000)
    );
}
