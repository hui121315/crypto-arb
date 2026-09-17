use exchange::adapters::hyperliquid_ws_user::{
    parse_user_event, subscribe_user_fundings_payload, HyperliquidUserWsEvent,
};
use exchange::venue_spec::{
    endpoint_evidence, EndpointDataKind, EndpointUseCase, VenueId, ENDPOINT_SPECS,
    UNRECORDED_EVIDENCE_MARKER,
};
use serde_json::Value;

const HYPERLIQUID_USER: &str = "0x0000000000000000000000000000000000000000";
const HYPERLIQUID_FUNDING_FIXTURE: &str =
    include_str!("../fixtures/hyperliquid/ws_user_fundings_btc.json");

#[test]
fn funding_payment_evidence_matrix_covers_all_seven_venues_without_skips() -> Result<(), String> {
    let rest_venues = [
        VenueId::Binance,
        VenueId::Okx,
        VenueId::Bybit,
        VenueId::Bitget,
        VenueId::Gate,
        VenueId::Kucoin,
    ];

    for venue in rest_venues {
        let rows = ENDPOINT_SPECS
            .iter()
            .filter(|spec| {
                spec.venue == venue
                    && spec.use_case == EndpointUseCase::PrivateRead
                    && spec.data_kind == EndpointDataKind::FundingPayment
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows.len(),
            1,
            "{venue:?} must expose one funding payment route"
        );
        let evidence = endpoint_evidence(venue.as_str(), rows[0].method, rows[0].path)
            .ok_or_else(|| format!("{venue:?} funding payment evidence missing"))?;
        assert_ne!(evidence.fixture_id, UNRECORDED_EVIDENCE_MARKER);
        assert_ne!(evidence.parser_test, UNRECORDED_EVIDENCE_MARKER);
        assert_ne!(evidence.request_builder_test, UNRECORDED_EVIDENCE_MARKER);
        assert!(!evidence.doc_urls.is_empty());
    }

    let subscription_payload = subscribe_user_fundings_payload(HYPERLIQUID_USER)
        .map_err(|error| format!("hyperliquid userFundings subscription: {error}"))?;
    let subscription: Value = serde_json::from_str(&subscription_payload)
        .map_err(|error| format!("hyperliquid userFundings subscription JSON: {error}"))?;
    assert_eq!(subscription["method"], "subscribe");
    assert_eq!(subscription["subscription"]["type"], "userFundings");
    assert_eq!(subscription["subscription"]["user"], HYPERLIQUID_USER);

    let event = parse_user_event(HYPERLIQUID_FUNDING_FIXTURE)
        .map_err(|error| format!("hyperliquid userFundings fixture: {error}"))?
        .ok_or_else(|| "hyperliquid userFundings fixture was ignored".to_owned())?;
    let HyperliquidUserWsEvent::Funding(rows) = event else {
        return Err("hyperliquid userFundings fixture mapped to wrong event".to_owned());
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].coin, "BTC");
    assert_eq!(rows[0].usdc, -0.1);
    assert_eq!(rows[0].size, 0.5);
    assert_eq!(rows[0].funding_rate, 0.0001);
    Ok(())
}
