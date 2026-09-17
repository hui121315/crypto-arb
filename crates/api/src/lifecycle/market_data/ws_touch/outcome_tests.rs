use super::*;

#[test]
fn kucoin_warmup_windows_cover_control_and_first_event_delay() {
    assert_eq!(
        ws_warmup_grace_ms("kucoin", MARKET_OP_WS_SPOT_SNAPSHOT),
        8_000
    );
    assert_eq!(
        ws_warmup_grace_ms("kucoin", MARKET_OP_WS_TICKER_SNAPSHOT),
        7_000
    );
    assert_eq!(
        ws_warmup_grace_ms("kucoin", MARKET_OP_WS_FUNDING_SNAPSHOT),
        65_000
    );
}

#[test]
fn gate_market_snapshot_allows_for_sparse_first_events() {
    assert_eq!(
        ws_warmup_grace_ms("gate", MARKET_OP_WS_TICKER_SNAPSHOT),
        30_000
    );
    assert_eq!(
        ws_warmup_grace_ms("gate", MARKET_OP_WS_FUNDING_SNAPSHOT),
        30_000
    );
    assert_eq!(
        ws_warmup_grace_ms("gate", MARKET_OP_WS_TICKER_SUBSCRIBE),
        10_000
    );
}
