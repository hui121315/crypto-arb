use super::*;

const LIVE_BBO_HEX: &str = "3b000100010001009c92b295e9570600026691b295e9570600c3fab2241c000000ff006f8d090000000000d6150100000000006e8d090000000000355700000000000013667574757265732e626f6f6b5f7469636b6572084254435f55534454";
const LIVE_TICKER_HEX: &str = "0900090001000100de50bb95e9570600027a000100c854ba95e9570600fc2c801b0100000000687ef6ffffffffffbcd8190100000000dc6e270100000000fc34871b0100000000fcf8a91b0100000000fc4481fffffffffffffa1300000000000000fdf81b7b0921000000ffa2d9627400000000fda2d9627400000000007e573dd800000000007e573dd800000000084554485f5553445400046c617374033234680f667574757265732e7469636b657273";
const LIVE_ORDER_BOOK_HEX: &str = "1c00040001000100b954f0799058060003e0c6ef799058060053fae10500000000fcfe0110000100d4ed06000000000020030000000000001000010048e3060000000000bc0200000000000012667574757265732e6f726465725f626f6f6b09494f4e515f55534454";

#[test]
fn parses_live_gate_sbe_bbo_frame() {
    let payload = hex::decode(LIVE_BBO_HEX).unwrap();
    let Some(GateSbeUpdate::Book(symbol, row)) = parse_sbe_update(&payload).unwrap() else {
        panic!("expected Gate SBE BBO update");
    };
    assert_eq!(symbol, "BTC_USDT");
    assert!((row.bid - 62_603.0).abs() < f64::EPSILON);
    assert!((row.ask - 62_603.1).abs() < 1e-9);
    assert_eq!(row.data_timestamp_ms, 1_785_510_610_768);
}

#[test]
fn parses_live_gate_sbe_futures_ticker_frame() {
    let payload = hex::decode(LIVE_TICKER_HEX).unwrap();
    let Some(GateSbeUpdate::Markets(rows)) = parse_sbe_update(&payload).unwrap() else {
        panic!("expected Gate SBE futures ticker update");
    };
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.item.contract, "ETH_USDT");
    assert_eq!(row.item.last, "1857.9500");
    assert_eq!(row.item.mark_price, "1858.1300");
    assert_eq!(row.item.index_price, "1859.0200");
    assert_eq!(row.item.funding_rate, "0.000019");
    assert_eq!(row.item.volume_24h_quote, "3627898750");
    assert_eq!(row.item.total_size, "141892983.800");
    assert_eq!(row.data_timestamp_ms, 1_785_510_611_277);
}

#[test]
fn parses_live_gate_sbe_one_level_order_book_snapshot() {
    let payload = hex::decode(LIVE_ORDER_BOOK_HEX).unwrap();
    let Some(GateSbeUpdate::BookSnapshot(symbol, row)) = parse_sbe_update(&payload).unwrap() else {
        panic!("expected Gate SBE order-book snapshot");
    };
    assert_eq!(symbol, "IONQ_USDT");
    assert!((row.bid - 45.14).abs() < 1e-9);
    assert!((row.ask - 45.41).abs() < 1e-9);
}

#[test]
fn rejects_unknown_schema_and_truncated_frames() {
    let mut unknown_schema = hex::decode(LIVE_BBO_HEX).unwrap();
    unknown_schema[4..6].copy_from_slice(&2_u16.to_le_bytes());
    assert_eq!(
        parse_sbe_update(&unknown_schema).unwrap_err(),
        "unsupported Gate SBE schema"
    );
    assert_eq!(
        parse_sbe_update(&unknown_schema[..12]).unwrap_err(),
        "unsupported Gate SBE schema"
    );
    assert_eq!(
        parse_sbe_update(&hex::decode(LIVE_BBO_HEX).unwrap()[..24]).unwrap_err(),
        "Gate SBE frame is truncated"
    );
}
