use super::*;
use crate::adapters::hyperliquid_market_data::{parse_levels, L2Book};
use crate::adapters::hyperliquid_private_data::{order_status_to_info, OrderStatusPayload};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn info_post_request_matches_official_envelope() {
    let request = WsInfoPostRequest {
        method: METHOD_POST,
        id: 123,
        request: WsInfoRequestEnvelope {
            request_type: REQUEST_TYPE_INFO,
            payload: json!({"type": "l2Book", "coin": "ETH"}),
        },
    };
    let value = serde_json::to_value(request).expect("info request serializes");

    assert_eq!(value["method"], "post");
    assert_eq!(value["id"], 123);
    assert_eq!(value["request"]["type"], "info");
    assert_eq!(value["request"]["payload"]["type"], "l2Book");
    assert_eq!(value["request"]["payload"]["coin"], "ETH");
}

#[test]
fn info_post_response_parses_official_l2_book_fixture() {
    let response = parse_info_response(include_str!(
        "../../fixtures/hyperliquid/ws_post_l2_book.json"
    ))
    .expect("official info response parses")
    .expect("post channel response");
    assert!(response.matches_id(123));

    let book: L2Book = response
        .into_result(123, "l2Book")
        .expect("l2Book response data parses");
    assert_eq!(book.time, 1_754_450_974_231);
    assert_eq!(parse_levels(&book.levels[0]), vec![[3007.1, 2.7954]]);
}

#[test]
fn info_post_response_parses_official_order_status_fixture() {
    let response = parse_info_response(include_str!(
        "../../fixtures/hyperliquid/ws_post_order_status.json"
    ))
    .expect("official info response parses")
    .expect("post channel response");
    assert!(response.matches_id(124));

    let payload: OrderStatusPayload = response
        .into_result(124, "orderStatus")
        .expect("orderStatus response data parses");
    let error = order_status_to_info(payload, "ETH", "hyperliquid")
        .expect_err("filled status cannot invent fill economics");
    assert!(error.to_string().contains("userFills evidence"));
}

#[test]
fn info_post_response_rejects_type_and_id_mismatches() {
    let body = include_str!("../../fixtures/hyperliquid/ws_post_l2_book.json");
    let wrong_id = parse_info_response(body)
        .expect("response parses")
        .expect("post response")
        .into_result::<L2Book>(124, "l2Book")
        .expect_err("request id mismatch rejected");
    assert!(wrong_id.to_string().contains("id mismatch"));

    let wrong_type = parse_info_response(body)
        .expect("response parses")
        .expect("post response")
        .into_result::<L2Book>(123, "allMids")
        .expect_err("request type mismatch rejected");
    assert!(wrong_type.to_string().contains("type mismatch"));
}

#[test]
fn info_post_response_surfaces_official_error_shape() {
    let body = r#"{
        "channel":"post",
        "data":{"id":7,"response":{"type":"error","payload":"429 Too Many Requests"}}
    }"#;
    let error = parse_info_response(body)
        .expect("error response parses")
        .expect("post response")
        .into_result::<serde_json::Value>(7, "allMids")
        .expect_err("error response remains an error");

    assert!(matches!(error, ExchangeError::Api { code, .. } if code == "info"));
}
