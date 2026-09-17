use exchange::ws::{trading_ws_operation_registry, trading_ws_venues};
use shared_types::ExchangeWsEvidenceScope;

#[test]
fn bitget_private_operations_are_fixture_backed_and_not_ack_only() -> Result<(), String> {
    let registry = trading_ws_operation_registry();
    let bitget = registry
        .venues
        .iter()
        .find(|venue| venue.venue == "bitget")
        .ok_or_else(|| "missing Bitget operation registry".to_owned())?;

    for (label, scope, fixture) in [
        (
            "account_stream",
            ExchangeWsEvidenceScope::PrivateAccountStream,
            "crates/exchange/fixtures/bitget/uta_ws_account_snapshot.json",
        ),
        (
            "position_stream",
            ExchangeWsEvidenceScope::PrivatePositionStream,
            "crates/exchange/fixtures/bitget/uta_ws_position_snapshot.json",
        ),
        (
            "order_stream",
            ExchangeWsEvidenceScope::PrivateOrderStream,
            "crates/exchange/fixtures/bitget/uta_ws_order_filled.json",
        ),
        (
            "fill_stream",
            ExchangeWsEvidenceScope::PrivateFillStream,
            "crates/exchange/fixtures/bitget/uta_ws_fill.json",
        ),
    ] {
        let operation = bitget
            .operations
            .iter()
            .find(|operation| operation.label == label)
            .ok_or_else(|| format!("missing Bitget {label}"))?;
        assert_eq!(operation.evidence_scope, scope, "{label}");
        assert_eq!(operation.fixture_id.as_deref(), Some(fixture), "{label}");
        assert!(
            operation
                .fixture_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:")),
            "{label}"
        );
        assert_eq!(operation.auth_kind, "server_ack_confirmed_login", "{label}");
    }
    Ok(())
}

#[test]
fn bitget_write_scope_is_linear_and_finality_stays_on_private_order_stream() -> Result<(), String> {
    let response = trading_ws_venues();
    let bitget = response
        .venues
        .iter()
        .find(|venue| venue.venue == "bitget")
        .ok_or_else(|| "missing Bitget venue".to_owned())?;
    assert_eq!(bitget.place_order.product, "USDT/USDC-FUTURES");
    assert_eq!(bitget.cancel_order.product, "USDT/USDC-FUTURES");
    assert!(bitget.note.contains("COIN/Reality fail-closed"));
    assert_eq!(bitget.order_stream.operation.as_deref(), Some("order"));
    assert_eq!(bitget.order_status.operation.as_deref(), Some("order"));
    Ok(())
}

#[test]
fn bitget_pr_en_source_keeps_compiler_cache_and_confirmed_runtime_anchors() {
    let compiler = include_str!("../src/adapters/bitget_order_compiler.rs");
    let instruments = include_str!("../src/adapters/bitget_instruments.rs");
    let user_ws = include_str!("../src/adapters/bitget_uta_ws_user.rs");
    assert!(compiler.contains("BitgetPositionMode::Hedge"));
    assert!(compiler.contains("observation-only: inverse/Reality"));
    assert!(instruments.contains("BitgetInstrumentCache"));
    assert!(instruments.contains("execution_supported"));
    assert!(user_ws.contains("parse_user_control"));
    assert!(user_ws.contains("inst_type: INST_TYPE_UTA"));
}
