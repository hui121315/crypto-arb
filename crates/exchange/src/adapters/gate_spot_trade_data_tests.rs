use super::*;

#[test]
fn point_query_projects_native_pair_to_canonical_symbol() {
    let row: GateSpotOrderRow = serde_json::from_str(
        r#"{"id":"7","text":"t-cid","currency_pair":"SOL_USDT","type":"limit","side":"buy","amount":"0.1","price":"150","left":"0","status":"closed","finish_as":"filled","avg_deal_price":"149.9","time_in_force":"gtc","create_time_ms":1780000000000}"#,
    )
    .expect("gate spot order row");

    let order = order_info(row).expect("gate spot order projection");
    assert_eq!(order.symbol, "SOL");
    assert_eq!(order.status, OrderStatus::Filled);
    assert_eq!(order.filled_quantity, 0.1);
}
