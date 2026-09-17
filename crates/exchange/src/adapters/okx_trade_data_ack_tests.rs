use super::*;
use pretty_assertions::assert_eq;

fn ack_item(ord_id: &str, s_code: &str, s_msg: &str) -> OrderAckItem {
    OrderAckItem {
        ord_id: ord_id.to_owned(),
        cl_ord_id: "c1".to_owned(),
        s_code: s_code.to_owned(),
        s_msg: s_msg.to_owned(),
    }
}

#[test]
fn ack_requires_confirmed_scode_and_ord_id() {
    // (ord_id, s_code, expected_accepted)
    let cases = [
        ("ex-1", "0", true),
        ("", "0", false),
        ("ex-1", "", false),
        ("", "", false),
        ("ex-1", "51008", false),
    ];
    for (ord_id, s_code, accepted) in cases {
        let ack = ack_from_item("i1".into(), "c1".into(), ack_item(ord_id, s_code, ""));
        let expected = if accepted {
            LiveOrderState::Accepted
        } else {
            LiveOrderState::Rejected
        };
        assert_eq!(
            ack.state, expected,
            "ord_id={ord_id:?} s_code={s_code:?} mapped to the wrong state"
        );
        if !accepted {
            assert!(
                ack.exchange_order_id.is_none(),
                "unconfirmed ack must not expose an exchange order id (ord_id={ord_id:?} s_code={s_code:?})"
            );
            assert!(
                ack.message.is_some(),
                "unconfirmed ack must carry a diagnostic message (ord_id={ord_id:?} s_code={s_code:?})"
            );
        }
    }
}

#[test]
fn ack_empty_scode_is_unconfirmed_not_accepted() {
    let ack = ack_from_item("i1".into(), "c1".into(), ack_item("ex-1", "", ""));
    assert_eq!(ack.state, LiveOrderState::Rejected);
    assert_eq!(
        ack.message.as_deref(),
        Some("okx order ack missing sCode; treating as unconfirmed")
    );
}

#[test]
fn ack_scode_zero_without_ord_id_is_unconfirmed() {
    let ack = ack_from_item("i1".into(), "c1".into(), ack_item("", "0", ""));
    assert_eq!(ack.state, LiveOrderState::Rejected);
    assert_eq!(
        ack.message.as_deref(),
        Some("okx order ack reported sCode=0 without ordId; treating as unconfirmed")
    );
}

#[test]
fn ack_confirmed_success_maps_to_accepted() {
    let ack = ack_from_item("i1".into(), "c1".into(), ack_item("ex-9", "0", ""));
    assert_eq!(ack.state, LiveOrderState::Accepted);
    assert_eq!(ack.exchange_order_id.as_deref(), Some("ex-9"));
    assert!(ack.message.is_none());
}

// PR-CU: official OKX V5 cancel-order REST response-envelope fixtures.
// Source: <https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-cancel-order>
// OKX wraps the result in {code,msg,data:[{ordId,clOrdId,sCode,sMsg}]}. A
// top-level `code != "0"` is a request-level failure that must fail closed; a
// per-item `sCode != "0"` is an order-level rejection that must never surface as
// an acceptance.
#[test]
fn cancel_order_official_success_envelope_maps_to_accepted() {
    let body = r#"{"code":"0","msg":"","data":[{"clOrdId":"c1","ordId":"312269865356374016","sCode":"0","sMsg":""}]}"#;
    let mut rows =
        crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "cancel order")
            .expect("official success envelope parses");
    assert_eq!(rows.len(), 1, "single cancel result expected");
    let ack = ack_from_item("i1".into(), "c1".into(), rows.remove(0));
    assert_eq!(ack.state, LiveOrderState::Accepted);
    assert_eq!(ack.exchange_order_id.as_deref(), Some("312269865356374016"));
}

#[test]
fn cancel_order_official_per_item_failure_is_rejected() {
    // Top-level code is "0" but the order itself failed (sCode 51400 = order does not exist).
    let body = r#"{"code":"0","msg":"","data":[{"clOrdId":"c1","ordId":"","sCode":"51400","sMsg":"Cancellation failed as the order does not exist."}]}"#;
    let mut rows =
        crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "cancel order")
            .expect("official per-item failure envelope parses");
    let ack = ack_from_item("i1".into(), "c1".into(), rows.remove(0));
    assert_eq!(ack.state, LiveOrderState::Rejected);
    assert!(
        ack.exchange_order_id.is_none(),
        "rejected cancel must not expose an order id"
    );
    assert_eq!(
        ack.message.as_deref(),
        Some("51400 Cancellation failed as the order does not exist.")
    );
}

#[test]
fn cancel_order_top_level_error_envelope_fails_closed() {
    // Request-level rejection: top-level code != "0" must become an Api error.
    let body = r#"{"code":"51000","msg":"Parameter ordId or clOrdId is required","data":[]}"#;
    let error = crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "cancel order")
        .expect_err("top-level error must fail closed");
    let text = error.to_string();
    assert!(
        text.contains("51000"),
        "error should carry the okx code: {text}"
    );
    assert!(
        text.contains("cancel order"),
        "error should carry the op context: {text}"
    );
}

// PR-CU: official OKX V5 place-order REST response-envelope fixtures.
// Source: <https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order>
// place_order decodes {code,msg,data:[{ordId,clOrdId,sCode,sMsg}]} via
// data_from_text + ack_from_item, exactly like cancel. The existing ack_* tests
// build OrderAckItem directly and the cancel_order_official_* tests cover the
// envelope only for cancel; the place path's end-to-end envelope decode (success,
// per-item failure, top-level request error) was never exercised.
#[test]
fn place_order_official_success_envelope_maps_to_accepted() {
    let body = r#"{"code":"0","msg":"","data":[{"clOrdId":"c1","ordId":"312269865356374016","sCode":"0","sMsg":"Order placed"}]}"#;
    let mut rows =
        crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "place order")
            .expect("official success envelope parses");
    assert_eq!(rows.len(), 1, "single place result expected");
    let ack = ack_from_item("i1".into(), "c1".into(), rows.remove(0));
    assert_eq!(ack.state, LiveOrderState::Accepted);
    assert_eq!(ack.exchange_order_id.as_deref(), Some("312269865356374016"));
    assert!(ack.message.is_none());
}

#[test]
fn place_order_official_per_item_failure_is_rejected() {
    // Top-level code is "0" but the order itself was rejected (sCode 51008 =
    // order placement failed due to insufficient balance). It must never become
    // an acceptance and must not fabricate an exchange order id.
    let body = r#"{"code":"0","msg":"","data":[{"clOrdId":"c1","ordId":"","sCode":"51008","sMsg":"Order placement failed due to insufficient balance."}]}"#;
    let mut rows =
        crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "place order")
            .expect("official per-item failure envelope parses");
    let ack = ack_from_item("i1".into(), "c1".into(), rows.remove(0));
    assert_eq!(ack.state, LiveOrderState::Rejected);
    assert!(
        ack.exchange_order_id.is_none(),
        "rejected place must not expose an order id"
    );
    assert_eq!(
        ack.message.as_deref(),
        Some("51008 Order placement failed due to insufficient balance.")
    );
}

#[test]
fn place_order_top_level_error_envelope_fails_closed() {
    // Request-level rejection: top-level code != "0" must become an Api error
    // carrying the okx code and the op context, not a silently empty ack.
    let body = r#"{"code":"51000","msg":"Parameter sz error","data":[]}"#;
    let error = crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "place order")
        .expect_err("top-level error must fail closed");
    let text = error.to_string();
    assert!(
        text.contains("51000"),
        "error should carry the okx code: {text}"
    );
    assert!(
        text.contains("place order"),
        "error should carry the op context: {text}"
    );
}

#[test]
fn okx_place_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/okx/trade_place_order_ack.json");
    let mut rows =
        crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "place order")
            .expect("official okx place-order envelope parses");
    assert_eq!(rows.len(), 1, "single place result expected");
    let ack = ack_from_item(
        "internal-okx-1".into(),
        "crossline-okx-1".into(),
        rows.remove(0),
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("312269865356374016"));
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn okx_cancel_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/okx/trade_cancel_order_ack.json");
    let mut rows =
        crate::adapters::okx_response::data_from_text::<OrderAckItem>(body, "cancel order")
            .expect("official okx cancel-order envelope parses");
    assert_eq!(rows.len(), 1, "single cancel result expected");
    let ack = ack_from_item(
        "internal-okx-cancel-1".into(),
        "oktswap6".into(),
        rows.remove(0),
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("12345689"));
    assert_eq!(ack.client_order_id, "oktswap6");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}
