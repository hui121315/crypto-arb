use super::*;

#[test]
fn spot_query_paths_are_identity_bound_and_encoded() {
    assert_eq!(
        query_by_client_path("cid-1", "SOL-USDT").expect("client query"),
        "/api/v1/hf/orders/client-order/cid-1?symbol=SOL-USDT"
    );
    assert_eq!(
        query_by_order_path("order/1", "SOL-USDT").expect("order query"),
        "/api/v1/hf/orders/order%2F1?symbol=SOL-USDT"
    );
}

#[test]
fn point_query_projects_native_pair_to_canonical_symbol() {
    let row: SpotOrderRow = serde_json::from_str(
        r#"{"id":"7","symbol":"SOL-USDT","type":"limit","side":"buy","price":"150","size":"0.1","dealSize":"0.1","dealFunds":"15","fee":"0.01","timeInForce":"GTC","clientOid":"cid-1","remainSize":"0","active":false,"createdAt":1780000000000}"#,
    )
    .expect("kucoin spot order row");
    let order = order_info(row).expect("kucoin spot order projection");
    assert_eq!(order.symbol, "SOL");
    assert_eq!(order.status, OrderStatus::Filled);
}
