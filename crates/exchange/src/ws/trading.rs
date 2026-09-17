//! 9 家交易所交易 WebSocket 能力规格。

use shared_types::{
    ExchangeWsDoc, ExchangeWsEvidenceScope, ExchangeWsOperation, ExchangeWsOperationEvidence,
    ExchangeWsOperationRegistryRow, ExchangeWsOperationVenue, ExchangeWsOperationsResponse,
    ExchangeWsReleaseStatus, ExchangeWsSupportStatus, ExchangeWsVenue, ExchangeWsVenuesResponse,
};

pub const TRADING_WS_VENUE_COUNT: usize = 9;

#[derive(Clone, Copy)]
struct WsDocSpec {
    label: &'static str,
    url: &'static str,
}

#[derive(Clone, Copy)]
struct WsOperationSpec {
    supported: bool,
    status: ExchangeWsSupportStatus,
    operation: Option<&'static str>,
    product: &'static str,
    note: &'static str,
    evidence: Option<WsOperationEvidenceSpec>,
}

#[derive(Clone, Copy)]
struct WsOperationEvidenceSpec {
    release_status: ExchangeWsReleaseStatus,
    requires_authenticated_runtime_evidence: bool,
    authenticated_runtime_evidence: bool,
    checked_at: &'static str,
    doc_version: &'static str,
    doc_url: &'static str,
    parser_test: Option<&'static str>,
    subscription_test: Option<&'static str>,
    fixture_id: Option<&'static str>,
    fixture_hash: Option<&'static str>,
    auth_kind: &'static str,
}

#[derive(Clone, Copy)]
struct WsVenueSpec {
    venue: &'static str,
    label: &'static str,
    public_endpoint: &'static str,
    private_endpoint: Option<&'static str>,
    trade_endpoint: Option<&'static str>,
    account_stream: WsOperationSpec,
    position_stream: WsOperationSpec,
    fill_stream: WsOperationSpec,
    order_stream: WsOperationSpec,
    place_order: WsOperationSpec,
    cancel_order: WsOperationSpec,
    close_position: WsOperationSpec,
    order_status: WsOperationSpec,
    auth_fields: &'static [&'static str],
    docs: &'static [WsDocSpec],
    note: &'static str,
}

const READY: ExchangeWsSupportStatus = ExchangeWsSupportStatus::Ready;
const PERMISSION: ExchangeWsSupportStatus = ExchangeWsSupportStatus::RequiresPermission;
const SCHEMA_PENDING: ExchangeWsSupportStatus = ExchangeWsSupportStatus::SchemaPending;

const fn ws_op(
    supported: bool,
    status: ExchangeWsSupportStatus,
    operation: Option<&'static str>,
    product: &'static str,
    note: &'static str,
) -> WsOperationSpec {
    WsOperationSpec {
        supported,
        status,
        operation,
        product,
        note,
        evidence: None,
    }
}

const fn ws_op_with_evidence(
    supported: bool,
    status: ExchangeWsSupportStatus,
    operation: Option<&'static str>,
    product: &'static str,
    note: &'static str,
    evidence: WsOperationEvidenceSpec,
) -> WsOperationSpec {
    WsOperationSpec {
        supported,
        status,
        operation,
        product,
        note,
        evidence: Some(evidence),
    }
}

const fn ws_evidence(
    checked_at: &'static str,
    doc_version: &'static str,
    doc_url: &'static str,
    parser_test: Option<&'static str>,
    subscription_test: Option<&'static str>,
    auth_kind: &'static str,
) -> WsOperationEvidenceSpec {
    WsOperationEvidenceSpec {
        release_status: ExchangeWsReleaseStatus::ProductionReady,
        requires_authenticated_runtime_evidence: false,
        authenticated_runtime_evidence: false,
        checked_at,
        doc_version,
        doc_url,
        parser_test,
        subscription_test,
        fixture_id: None,
        fixture_hash: None,
        auth_kind,
    }
}

const fn ws_evidence_beta(evidence: WsOperationEvidenceSpec) -> WsOperationEvidenceSpec {
    WsOperationEvidenceSpec {
        release_status: ExchangeWsReleaseStatus::BetaUnavailable,
        requires_authenticated_runtime_evidence: evidence.requires_authenticated_runtime_evidence,
        authenticated_runtime_evidence: false,
        checked_at: evidence.checked_at,
        doc_version: evidence.doc_version,
        doc_url: evidence.doc_url,
        parser_test: evidence.parser_test,
        subscription_test: evidence.subscription_test,
        fixture_id: evidence.fixture_id,
        fixture_hash: evidence.fixture_hash,
        auth_kind: evidence.auth_kind,
    }
}

const fn ws_evidence_with_fixture(
    evidence: WsOperationEvidenceSpec,
    fixture_id: &'static str,
    fixture_hash: &'static str,
) -> WsOperationEvidenceSpec {
    WsOperationEvidenceSpec {
        release_status: evidence.release_status,
        requires_authenticated_runtime_evidence: evidence.requires_authenticated_runtime_evidence,
        authenticated_runtime_evidence: evidence.authenticated_runtime_evidence,
        checked_at: evidence.checked_at,
        doc_version: evidence.doc_version,
        doc_url: evidence.doc_url,
        parser_test: evidence.parser_test,
        subscription_test: evidence.subscription_test,
        fixture_id: Some(fixture_id),
        fixture_hash: Some(fixture_hash),
        auth_kind: evidence.auth_kind,
    }
}

const BINANCE_DOCS: &[WsDocSpec] = &[
    doc(
        "current ws account api",
        "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/account",
    ),
    doc(
        "current ws trade api",
        "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/trade",
    ),
    doc(
        "current ws user data stream control",
        "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/user-data-streams",
    ),
    doc(
        "order update event",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Event-Order-Update",
    ),
    doc(
        "ws api general info",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-api-general-info",
    ),
    doc(
        "ws new order",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/websocket-api/New-Order",
    ),
    doc(
        "ws cancel order",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/websocket-api/Cancel-Order",
    ),
];

const OKX_DOCS: &[WsDocSpec] = &[doc("api v5 websocket", "https://www.okx.com/docs-v5/en/")];

const BYBIT_DOCS: &[WsDocSpec] = &[
    doc(
        "private order stream",
        "https://bybit-exchange.github.io/docs/v5/websocket/private/order",
    ),
    doc(
        "trade websocket",
        "https://bybit-exchange.github.io/docs/v5/websocket/trade/guideline",
    ),
];

const BITGET_DOCS: &[WsDocSpec] = &[
    doc(
        "private order channel",
        "https://www.bitget.com/api-doc/uta/websocket/private/Order-Channel",
    ),
    doc(
        "place order channel",
        "https://www.bitget.com/api-doc/uta/websocket/private/Place-Order-Channel",
    ),
    doc(
        "cancel order channel",
        "https://www.bitget.com/api-doc/uta/websocket/private/Cancel-Order-Channel",
    ),
];

const GATE_DOCS: &[WsDocSpec] = &[doc(
    "futures websocket",
    "https://www.gate.com/docs/developers/futures/ws/en/",
)];

const GATE_CROSSEX_DOCS: &[WsDocSpec] = &[
    doc(
        "crossex websocket v1.0.0",
        "https://www.gate.com/docs/developers/crossex/ws/en/",
    ),
    doc(
        "crossex rest v1.0.2",
        "https://www.gate.com/docs/developers/crossex/en/",
    ),
];

const KRAKEN_DOCS: &[WsDocSpec] = &[
    doc(
        "spot websocket v2",
        "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2",
    ),
    doc(
        "derivatives websocket",
        "https://docs.kraken.com/exchange/api-reference/futures-websocket",
    ),
    doc(
        "derivatives rest",
        "https://docs.kraken.com/exchange/api-reference/futures-rest-api",
    ),
];

const KUCOIN_DOCS: &[WsDocSpec] = &[
    doc(
        "private order changes",
        "https://www.kucoin.com/docs-new/3470090w0",
    ),
    doc(
        "pro ws add order",
        "https://www.kucoin.com/docs-new/3470252w0",
    ),
    doc(
        "pro ws cancel order",
        "https://www.kucoin.com/docs-new/3470253w0",
    ),
];

const HYPERLIQUID_DOCS: &[WsDocSpec] = &[
    doc(
        "websocket",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket",
    ),
    doc(
        "post requests",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/post-requests",
    ),
    doc(
        "exchange endpoint",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint",
    ),
    doc(
        "signing",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/signing",
    ),
    doc(
        "subscriptions",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
    ),
];

const BINANCE_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "binance-usdm-user-data-stream-account-update-2026-07-02",
    "https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Event-Balance-and-Position-Update",
    Some("parses_account_update_without_inventing_shared_balance_fields"),
    Some("user_stream_url_uses_private_ws_base"),
    "listen_key",
);
const BINANCE_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-11",
        "binance-usdm-user-data-stream-order-update-2026-07-11",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Event-Order-Update",
        Some("parses_order_trade_update_to_order_delta"),
        Some("user_stream_url_uses_private_ws_base"),
        "listen_key",
    ),
    "crates/exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json",
    "sha256:590ce8eeaa980adeed148aeaac453da49bd130571bfb6fe0aeed47178ea26127",
);
const BINANCE_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "binance-usdm-ws-api-order-place-2026-07-02",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/websocket-api/New-Order",
        Some("binance_ws_place_order_ack_parses_official_fixture"),
        Some("order_place_request_matches_binance_ws_schema"),
        "api_key_signature",
    ),
    "crates/exchange/fixtures/binance/ws_order_place_success.json",
    "sha256:ebf0c6b4d768c737094e49dd337346b8084da9f5e394931307157e8c20b58fa6",
);
const BINANCE_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "binance-usdm-ws-api-order-cancel-2026-07-02",
        "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/websocket-api/Cancel-Order",
        Some("binance_ws_cancel_order_ack_parses_official_fixture"),
        Some("order_cancel_request_matches_binance_ws_schema"),
        "api_key_signature",
    ),
    "crates/exchange/fixtures/binance/ws_order_cancel_success.json",
    "sha256:0cdac638750c9358c3fe1158b887794ca930bfcd7e771a0d4dd6a82b2f72dbb7",
);
const BINANCE_ORDER_STATUS_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-30",
        "binance-usdm-ws-api-order-status-2026-07-30",
        "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/trade",
        Some("binance_ws_order_status_parses_supported_official_schema"),
        Some("private_read_requests_match_current_binance_ws_methods"),
        "api_key_signature",
    ),
    "crates/exchange/fixtures/binance/ws_order_status_filled.json",
    "sha256:adf8ea0c2c78ff70746441276d83061e61558a7a5abdc1a88d44e72552b0ee5f",
);
const OKX_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "okx-v5-private-ws-account-channel-2026-07-13",
        "https://www.okx.com/docs-v5/en/#trading-account-websocket-account-channel",
        Some("parses_account_snapshot_details"),
        Some("account_subscription_matches_okx_schema"),
        "login",
    ),
    "crates/exchange/fixtures/okx/ws_user_account_snapshot.json",
    "sha256:837fffd52faf84446073be08d72cb23c6b3c458ed17855dafccf489d0257d9a8",
);
const OKX_POSITION_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "okx-v5-private-ws-positions-channel-2026-07-13",
        "https://www.okx.com/docs-v5/en/#trading-account-websocket-positions-channel",
        Some("parses_position_snapshot_rows"),
        Some("positions_and_orders_use_any_inst_type"),
        "login",
    ),
    "crates/exchange/fixtures/okx/ws_user_positions_snapshot.json",
    "sha256:0d85b05993c200d4371245547a751d4580bdc94defbc7064e3b13ea4771609eb",
);
const OKX_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "okx-v5-private-ws-orders-channel-2026-07-13",
        "https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-order-channel",
        Some("parses_order_update_rows"),
        Some("positions_and_orders_use_any_inst_type"),
        "login",
    ),
    "crates/exchange/fixtures/okx/ws_user_orders_partial_fill.json",
    "sha256:f1d2431b6abbb8639c83784ca70859b6cf5839b5cb8093b1b030df9d0837306d",
);
const OKX_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "okx-v5-private-ws-order-inst-id-code-2026-07-02",
        "https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-place-order",
        Some("okx_ws_place_order_ack_parses_official_fixture"),
        Some("order_request_matches_okx_ws_schema"),
        "login",
    ),
    "crates/exchange/fixtures/okx/ws_trade_place_order_ack.json",
    "sha256:de0e9f857f687f60bbfab4a50af4042420371c6f1a46b357e63402f0cf7e9882",
);
const OKX_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "okx-v5-private-ws-cancel-order-inst-id-code-2026-07-02",
        "https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-cancel-order",
        Some("okx_ws_cancel_order_ack_parses_official_fixture"),
        Some("cancel_request_matches_okx_ws_schema"),
        "login",
    ),
    "crates/exchange/fixtures/okx/ws_trade_cancel_order_ack.json",
    "sha256:a439243e8c548d995079fe726503d98073dc7366fc5d395727e96f4210d62314",
);
const BYBIT_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bybit-v5-private-ws-order-2026-07-13",
        "https://bybit-exchange.github.io/docs/v5/websocket/private/order",
        Some("parses_official_order_fixture_to_terminal_delta"),
        Some("subscribe_payload_uses_bybit_private_topics"),
        "signed_subscription",
    ),
    "crates/exchange/fixtures/bybit/ws_user_order_filled.json",
    "sha256:92185c0e848f978cfe0e301a86439ea47d95984b583304c25c5897e5b9104536",
);
const BYBIT_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bybit-v5-private-ws-wallet-2026-07-13",
        "https://bybit-exchange.github.io/docs/v5/websocket/private/wallet",
        Some("parses_position_and_wallet_events_without_available_balance_guessing"),
        Some("subscribe_payload_uses_bybit_private_topics"),
        "signed_subscription",
    ),
    "crates/exchange/fixtures/bybit/ws_user_wallet_snapshot.json",
    "sha256:aa7b33dddabafac0c57e34644bf15d9a6363f8bed48fd8a2d9fec7d9af5f8799",
);
const BYBIT_POSITION_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bybit-v5-private-ws-position-2026-07-13",
        "https://bybit-exchange.github.io/docs/v5/websocket/private/position",
        Some("parses_position_and_wallet_events_without_available_balance_guessing"),
        Some("subscribe_payload_uses_bybit_private_topics"),
        "signed_subscription",
    ),
    "crates/exchange/fixtures/bybit/ws_user_position_snapshot.json",
    "sha256:85aa610f0572bc1de40bb72500927ea456b63d815aa0edc787e7a146a606fd56",
);
const BYBIT_EXECUTION_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bybit-v5-private-ws-execution-2026-07-13",
        "https://bybit-exchange.github.io/docs/v5/websocket/private/execution",
        Some("parses_official_execution_fixture_with_usdc_fee_identity"),
        Some("subscribe_payload_uses_bybit_private_topics"),
        "signed_subscription",
    ),
    "crates/exchange/fixtures/bybit/ws_user_execution_fill.json",
    "sha256:0a74cdbfda5e62ad0fdd105592fa5493fda63bdf396a515adcd489213b1bdb39",
);
const BYBIT_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "bybit-v5-trade-ws-order-create-2026-07-02",
        "https://bybit-exchange.github.io/docs/v5/websocket/trade/guideline",
        Some("bybit_ws_place_order_ack_parses_official_fixture"),
        Some("order_create_request_matches_bybit_ws_schema"),
        "signed_trade_ws",
    ),
    "crates/exchange/fixtures/bybit/ws_order_create_ack.json",
    "sha256:b7a1a38959a03f88011ebc6b28e8861bec7eafdfe70468522f4fbd171c42a794",
);
const BYBIT_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "bybit-v5-trade-ws-order-cancel-2026-07-02",
        "https://bybit-exchange.github.io/docs/v5/websocket/trade/guideline",
        Some("bybit_ws_cancel_order_ack_parses_official_fixture"),
        Some("order_cancel_request_matches_bybit_ws_schema"),
        "signed_trade_ws",
    ),
    "crates/exchange/fixtures/bybit/ws_order_cancel_ack.json",
    "sha256:5450bef4c50c8eb370be02788198bd4eb43887f2e23652aa3f918a7e2d624940",
);
const BITGET_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bitget-uta-private-ws-account-2026-07-13",
        "https://www.bitget.com/api-doc/uta/websocket/private/Account-Channel",
        Some("account_fixture_preserves_aggregate_equity_and_nested_assets"),
        Some("private_subscriptions_are_all_uta_global_without_v2_wildcards"),
        "server_ack_confirmed_login",
    ),
    "crates/exchange/fixtures/bitget/uta_ws_account_snapshot.json",
    "sha256:27210ec8419cefb14a8d0b69defe3a24440ee3800ab428bc223f4ad4029d6438",
);
const BITGET_POSITION_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bitget-uta-private-ws-position-2026-07-13",
        "https://www.bitget.com/api-doc/uta/websocket/private/Positions-Channel",
        Some("position_fixture_uses_official_v3_field_names_and_native_identity"),
        Some("private_subscriptions_are_all_uta_global_without_v2_wildcards"),
        "server_ack_confirmed_login",
    ),
    "crates/exchange/fixtures/bitget/uta_ws_position_snapshot.json",
    "sha256:6da3d56b2220def023e07cfba21364810160de998d05a16c4d04cc0a5b068c6f",
);
const BITGET_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bitget-uta-private-ws-order-finality-2026-07-13",
        "https://www.bitget.com/api-doc/uta/websocket/private/Order-Channel",
        Some("order_and_fill_fixtures_preserve_terminal_finality_and_fees"),
        Some("private_subscriptions_are_all_uta_global_without_v2_wildcards"),
        "server_ack_confirmed_login",
    ),
    "crates/exchange/fixtures/bitget/uta_ws_order_filled.json",
    "sha256:6f5da324c84e6c7fd9a4d7b4a69a089631ebc0f749fd607887c71a9601066e1b",
);
const BITGET_FILL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-13",
        "bitget-uta-private-ws-fill-finality-2026-07-13",
        "https://www.bitget.com/api-doc/uta/websocket/private/Fill-Channel",
        Some("order_and_fill_fixtures_preserve_terminal_finality_and_fees"),
        Some("private_subscriptions_are_all_uta_global_without_v2_wildcards"),
        "server_ack_confirmed_login",
    ),
    "crates/exchange/fixtures/bitget/uta_ws_fill.json",
    "sha256:1ab2ef3fb717c23a36540cf6b3d21c3c9e64fccf5c2b568d91892e64d895e519",
);
const BITGET_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "bitget-uta-ws-place-order-2026-07-02",
        "https://www.bitget.com/api-doc/uta/websocket/private/Place-Order-Channel",
        Some("bitget_uta_ws_place_order_ack_parses_official_fixture"),
        Some("place_request_matches_v3_envelope_shape"),
        "login",
    ),
    "crates/exchange/fixtures/bitget/uta_ws_place_order_ack.json",
    "sha256:edb7ed0bc16594f7538548d67d9f8a53e831a093432c76bac0bf41d85016c9e6",
);
const BITGET_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "bitget-uta-ws-cancel-order-2026-07-02",
        "https://www.bitget.com/api-doc/uta/websocket/private/Cancel-Order-Channel",
        Some("bitget_uta_ws_cancel_order_ack_parses_official_fixture"),
        Some("cancel_request_envelope_carries_client_oid_only"),
        "login",
    ),
    "crates/exchange/fixtures/bitget/uta_ws_cancel_order_ack.json",
    "sha256:4599399cd0b6fe7ba69dbca6128b226792ddb7ae35517ef3f132cfad6b7d879d",
);
const GATE_PRIVATE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "gate-futures-private-ws-orders-positions-balances-2026-07-02",
    "https://www.gate.com/docs/developers/futures/ws/en/",
    Some("parses_order_position_and_balance_events"),
    Some("subscribe_payload_matches_gate_authenticated_schema"),
    "signed_subscription",
);
const GATE_USERTRADES_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "gate-futures-private-ws-usertrades-2026-07-02",
    "https://www.gate.com/docs/developers/futures/ws/en/#user-trades-api",
    Some("parses_usertrade_event_with_fee_evidence"),
    Some("usertrades_subscription_supports_all_contracts_payload"),
    "signed_subscription",
);
const GATE_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "gate-futures-ws-order-place-2026-07-02",
        "https://www.gate.com/docs/developers/futures/ws/en/#futures-account-trade",
        Some("gate_ws_order_place_ack_response_parses_official_fixture"),
        Some("place_request_matches_gate_ws_schema"),
        "login_then_api",
    ),
    "crates/exchange/fixtures/gate/ws_futures_order_place_success.json",
    "sha256:11af61e8c9755048b888ee8525b889ab7ec9e48e0ae616a72ce0692957ec82b3",
);
const GATE_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "gate-futures-ws-order-cancel-2026-07-02",
        "https://www.gate.com/docs/developers/futures/ws/en/#order-cancel",
        Some("gate_ws_order_cancel_ack_response_parses_official_fixture"),
        Some("cancel_request_matches_gate_ws_schema"),
        "login_then_api",
    ),
    "crates/exchange/fixtures/gate/ws_futures_order_cancel_success.json",
    "sha256:5596ef816f58b773d63e089e033571b8c3e9e823b2340110f5ed4125659113d4",
);
const GATE_ORDER_STATUS_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-30",
        "gate-futures-ws-order-status-list-v4.0.0-2026-07-30",
        "https://www.gate.com/docs/developers/futures/ws/en/#order-status",
        Some("gate_ws_order_status_response_parses_official_fixture"),
        Some("order_status_request_matches_gate_ws_schema"),
        "login_then_api",
    ),
    "crates/exchange/fixtures/gate/ws_futures_order_status.json",
    "sha256:9e425393dfff950bf9a194ca06b56dcf89b4c9e6c01955f5c7b4eb4e970966dc",
);
const KUCOIN_PRIVATE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "kucoin-futures-private-ws-orders-balance-position-2026-07-02",
    "https://www.kucoin.com/docs-new/websocket-api/base-info/futures-private-channels",
    Some("parses_order_balance_position_and_pro_ack"),
    Some("subscribe_payloads_match_futures_private_topics"),
    "bullet_private",
);
const KUCOIN_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-11",
        "kucoin-classic-futures-private-order-finality-2026-07-11",
        "https://www.kucoin.com/docs-new/3470090w0",
        Some("parses_order_balance_position_and_pro_ack"),
        Some("subscribe_payloads_match_futures_private_topics"),
        "bullet_private",
    ),
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_match.json",
    "sha256:25e4efb4849b0eb4ffbaab5b29391f6ec9a1982e5ba01abd270210210fc26980",
);
const KUCOIN_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence_beta(ws_evidence(
        "2026-07-30",
        "kucoin-pro-ws-futures-order-beta-2026-07-30",
        "https://www.kucoin.com/docs-new/3470252w0",
        Some("parses_order_balance_position_and_pro_ack"),
        Some("pro_order_and_cancel_payloads_use_official_ops"),
        "signed_wsapi_query_challenge",
    )),
    "crates/exchange/fixtures/kucoin/wsapi_pro_order_ack.json",
    "sha256:55fce65cecb72815dfb731f151d3adbf50ab576414ddbc231f6f125c0558ea9d",
);
const KUCOIN_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence_beta(ws_evidence(
        "2026-07-30",
        "kucoin-pro-ws-futures-cancel-beta-2026-07-30",
        "https://www.kucoin.com/docs-new/3470253w0",
        Some("parses_order_balance_position_and_pro_ack"),
        Some("pro_order_and_cancel_payloads_use_official_ops"),
        "signed_wsapi_query_challenge",
    )),
    "crates/exchange/fixtures/kucoin/wsapi_pro_cancel_ack.json",
    "sha256:7088a84487eb2b905a93360bad0d7ddcbff869c443f2c73c0409224bc955d53b",
);
const HYPERLIQUID_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "hyperliquid-user-ws-order-updates-2026-07-02",
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
    Some("parses_order_fills_funding_and_user_events"),
    Some("subscribe_payloads_match_official_user_subscriptions"),
    "user_address",
);
const HYPERLIQUID_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "hyperliquid-user-ws-clearinghouse-spot-state-2026-07-02",
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
    Some("parses_clearinghouse_spot_and_all_dexs_state"),
    Some("subscribe_payloads_match_official_user_subscriptions"),
    "user_address",
);
const HYPERLIQUID_FILL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence(
    "2026-07-02",
    "hyperliquid-user-ws-fills-2026-07-02",
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
    Some("parses_order_fills_funding_and_user_events"),
    Some("subscribe_payloads_match_official_user_subscriptions"),
    "user_address",
);
const HYPERLIQUID_ORDER_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "hyperliquid-ws-post-order-action-2026-07-02",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/post-requests",
        Some("hyperliquid_ws_place_order_ack_parses_official_fixture"),
        Some("signed_post_order_request_matches_hyperliquid_ws_schema"),
        "signed_action",
    ),
    "crates/exchange/fixtures/hyperliquid/ws_post_order_resting.json",
    "sha256:5123a11c462aef3b55e8c5cdfa3c67d78a0c0771853ce08fcba6007157798372",
);
const HYPERLIQUID_ORDER_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-02",
        "hyperliquid-ws-post-cancel-action-2026-07-02",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/post-requests",
        Some("hyperliquid_ws_cancel_order_ack_parses_official_fixture"),
        Some("signed_post_request_matches_hyperliquid_ws_schema"),
        "signed_action",
    ),
    "crates/exchange/fixtures/hyperliquid/ws_post_cancel_success.json",
    "sha256:b046ba05b5dd8322138824a88d6d882bd74c6ee91a76c2a886dddfd6c5b23504",
);
const HYPERLIQUID_ORDER_STATUS_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-07-30",
        "hyperliquid-ws-post-info-order-status-2026-07-30",
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/post-requests",
        Some("info_post_response_parses_official_order_status_fixture"),
        Some("info_post_request_matches_official_envelope"),
        "user_address",
    ),
    "crates/exchange/fixtures/hyperliquid/ws_post_order_status.json",
    "sha256:79ffa1106bd65ba23a01e305fc64915ec10a1ee5f6941ad7d1dc98d1c50b5b06",
);

const GATE_CROSSEX_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "gate-crossex-ws-v1.0.0-asset",
        "https://www.gate.com/docs/developers/crossex/ws/en/#asset-subscription-unsubscription",
        Some("parses_official_order_asset_position_and_fill_frames"),
        Some("login_and_subscriptions_match_official_envelope"),
        "api_key_login_hmac_sha512",
    ),
    "crates/exchange/fixtures/gate_crossex/private_asset_update.json",
    "sha256:a7d5e62475c2f2f56ed54956721398254c16f6b7ec396a01729623f4e1556600",
);
const GATE_CROSSEX_POSITION_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "gate-crossex-ws-v1.0.0-position",
        "https://www.gate.com/docs/developers/crossex/ws/en/#position-subscription-unsubscription",
        Some("parses_official_order_asset_position_and_fill_frames"),
        Some("login_and_subscriptions_match_official_envelope"),
        "api_key_login_hmac_sha512",
    ),
    "crates/exchange/fixtures/gate_crossex/private_position_update.json",
    "sha256:47c1f10ea5a63550049ccd621178251051e03aa0d059fa4442f0c21ce9617b0f",
);
const GATE_CROSSEX_FILL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "gate-crossex-ws-v1.0.0-usertrades",
        "https://www.gate.com/docs/developers/crossex/ws/en/#trade-subscription-unsubscription",
        Some("parses_official_order_asset_position_and_fill_frames"),
        Some("login_and_subscriptions_match_official_envelope"),
        "api_key_login_hmac_sha512",
    ),
    "crates/exchange/fixtures/gate_crossex/private_fill_update.json",
    "sha256:e18a948c3a735c9a4bce580df8b5ff9ae07bde351faa0b41a3a1d36e8bf2528d",
);
const GATE_CROSSEX_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "gate-crossex-ws-v1.0.0-order",
        "https://www.gate.com/docs/developers/crossex/ws/en/#order-subscription-unsubscription",
        Some("parses_official_order_asset_position_and_fill_frames"),
        Some("login_and_subscriptions_match_official_envelope"),
        "api_key_login_hmac_sha512",
    ),
    "crates/exchange/fixtures/gate_crossex/private_order_update.json",
    "sha256:5e90ac4508f094f556f69bde5d37507d39b54fcd95391496646378fd9daa76ca",
);
const GATE_CROSSEX_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "gate-crossex-ws-v1.0.0-place-order",
        "https://www.gate.com/docs/developers/crossex/ws/en/#place-order",
        Some("parses_ws_api_ack_and_rejects_wrong_request"),
        Some("trade_requests_match_official_envelope"),
        "api_key_login_hmac_sha512",
    ),
    "crates/exchange/fixtures/gate_crossex/place_order_ack.json",
    "sha256:e77dc3daed93b4ad319d4d507181f53c383f5639db964add25c9f1bce785e190",
);
const GATE_CROSSEX_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "gate-crossex-ws-v1.0.0-cancel-order",
        "https://www.gate.com/docs/developers/crossex/ws/en/#cancel-order",
        Some("parses_ws_api_ack_and_rejects_wrong_request"),
        Some("trade_requests_match_official_envelope"),
        "api_key_login_hmac_sha512",
    ),
    "crates/exchange/fixtures/gate_crossex/cancel_order_ack.json",
    "sha256:a9eefb1c1368789eb521eb2753d7cb36dc652868c96d4ef02b6d3642ef47d353",
);

const KRAKEN_ACCOUNT_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-spot-ws-v2-balances",
        "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/balances",
        Some("parses_official_execution_and_balance_frames"),
        Some("requests_follow_spot_v2_contract"),
        "rest_issued_ws_token",
    ),
    "crates/exchange/fixtures/kraken/spot_v2_balances_snapshot.json",
    "sha256:b068220cd04a3d77785bea7f9c99bdf6711b9db0414b37002313ac5e9c7d8d68",
);
const KRAKEN_POSITION_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-derivatives-ws-open-positions",
        "https://docs.kraken.com/exchange/api-reference/futures-websocket/open_positions",
        Some("parses_official_orders_fills_positions_and_balances"),
        Some("challenge_and_subscription_follow_official_contract"),
        "signed_challenge",
    ),
    "crates/exchange/fixtures/kraken/futures_open_positions.json",
    "sha256:c7f885e4c2117efdd0244f7e4e97deebfd91d370437a998883d03a16c001d4e2",
);
const KRAKEN_FILL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-derivatives-ws-fills",
        "https://docs.kraken.com/exchange/api-reference/futures-websocket/fills",
        Some("parses_official_orders_fills_positions_and_balances"),
        Some("challenge_and_subscription_follow_official_contract"),
        "signed_challenge",
    ),
    "crates/exchange/fixtures/kraken/futures_fills_snapshot.json",
    "sha256:953196a0dda54021fc25a6a2b16d87632af6555f35b52460025689b7aabd497f",
);
const KRAKEN_ORDER_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-spot-ws-v2-executions",
        "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/executions",
        Some("parses_official_execution_and_balance_frames"),
        Some("requests_follow_spot_v2_contract"),
        "rest_issued_ws_token",
    ),
    "crates/exchange/fixtures/kraken/spot_v2_execution_update.json",
    "sha256:a9bfcd02ace223d8fb3c98583ca4ff47d70155215b0d90419f80f4ea8057740d",
);
const KRAKEN_PLACE_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-spot-ws-v2-add-order",
        "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/add_order",
        Some("spot_v2_write_responses_parse_official_fixtures"),
        Some("requests_follow_spot_v2_contract"),
        "rest_issued_ws_token",
    ),
    "crates/exchange/fixtures/kraken/spot_v2_add_order_ack.json",
    "sha256:b9160622af804c351d710cad2f386af0b62c35d3decdbe133526033966f76d58",
);
const KRAKEN_CANCEL_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-spot-ws-v2-cancel-order",
        "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/cancel_order",
        Some("spot_v2_write_responses_parse_official_fixtures"),
        Some("requests_follow_spot_v2_contract"),
        "rest_issued_ws_token",
    ),
    "crates/exchange/fixtures/kraken/spot_v2_cancel_order_ack.json",
    "sha256:e4cbfc690e3922ef291b4c3f77091beb6b70b4dde22b0e404aaf6bd5022d3eea",
);
const KRAKEN_ORDER_STATUS_WS_EVIDENCE: WsOperationEvidenceSpec = ws_evidence_with_fixture(
    ws_evidence(
        "2026-08-06",
        "kraken-derivatives-ws-open-orders",
        "https://docs.kraken.com/exchange/api-reference/futures-websocket/open_orders",
        Some("parses_official_orders_fills_positions_and_balances"),
        Some("challenge_and_subscription_follow_official_contract"),
        "signed_challenge",
    ),
    "crates/exchange/fixtures/kraken/futures_open_orders_snapshot.json",
    "sha256:319c4f1923f165229b8abb2b85d478d16fdde62a877ce34759253f886d7659ed",
);

const SPECS: &[WsVenueSpec] = &[
    WsVenueSpec {
        venue: "binance",
        label: "Binance",
        public_endpoint: "wss://fstream.binance.com/public/ws",
        private_endpoint: Some("wss://fstream.binance.com/private/ws?listenKey=<listenKey>"),
        trade_endpoint: Some("wss://ws-fapi.binance.com/ws-fapi/v1"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("ACCOUNT_UPDATE"),
            "USD-M",
            "余额更新事件",
            BINANCE_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("ACCOUNT_UPDATE"),
            "USD-M",
            "仓位更新在账户事件中推送",
            BINANCE_ACCOUNT_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("ORDER_TRADE_UPDATE TRADE"),
            "USD-M",
            "订单事件携带逐笔成交字段",
            BINANCE_ORDER_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("ORDER_TRADE_UPDATE"),
            "USD-M",
            "listenKey 私有流",
            BINANCE_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            READY,
            Some("order.place"),
            "USD-M",
            "WS API 写单",
            BINANCE_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            READY,
            Some("order.cancel"),
            "USD-M",
            "WS API 撤单",
            BINANCE_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            READY,
            Some("order.place reduceOnly"),
            "USD-M",
            "按 reduceOnly/closePosition 映射",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("order.status"),
            "USD-M",
            "WS 点查；ORDER_TRADE_UPDATE 继续承担热更新与最终态推送",
            BINANCE_ORDER_STATUS_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret"],
        docs: BINANCE_DOCS,
        note: "用户流 start/ping/stop 走当前 WS API 并保留 REST 恢复；盘口走 /public，统计走 /market，私有流走 /private；写单与订单点查走 USD-M WebSocket API。",
    },
    WsVenueSpec {
        venue: "okx",
        label: "OKX",
        public_endpoint: "wss://ws.okx.com:8443/ws/v5/public",
        private_endpoint: Some("wss://ws.okx.com:8443/ws/v5/private"),
        trade_endpoint: Some("wss://ws.okx.com:8443/ws/v5/private"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("account"),
            "Spot/SWAP",
            "私有 account channel",
            OKX_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("positions"),
            "SWAP",
            "私有 positions channel",
            OKX_POSITION_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("orders fill*"),
            "Spot/SWAP",
            "orders channel 携带 fillPx/fillSz/fillFee",
            OKX_ORDER_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("orders"),
            "Spot/SWAP",
            "私有 orders channel",
            OKX_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "Spot/SWAP",
            "login 后 op=order；payload 使用 instIdCode",
            OKX_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            READY,
            Some("cancel-order"),
            "Spot/SWAP",
            "login 后 op=cancel-order；payload 使用 instIdCode",
            OKX_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            READY,
            Some("order reduceOnly"),
            "SWAP",
            "SWAP 平仓按 reduceOnly/posSide 映射",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("orders"),
            "Spot/SWAP",
            "实时订单状态",
            OKX_ORDER_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret", "passphrase"],
        docs: OKX_DOCS,
        note: "Demo/实盘由 x-simulated-trading 区分；HedgeTicket 执行前校验双腿凭证与能力。",
    },
    WsVenueSpec {
        venue: "bybit",
        label: "Bybit",
        public_endpoint: "wss://stream.bybit.com/v5/public/linear",
        private_endpoint: Some("wss://stream.bybit.com/v5/private"),
        trade_endpoint: Some("wss://stream.bybit.com/v5/trade"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("wallet"),
            "Unified",
            "私有 wallet topic",
            BYBIT_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("position"),
            "Linear",
            "私有 position topic",
            BYBIT_POSITION_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("execution"),
            "Spot/Linear",
            "私有 execution topic",
            BYBIT_EXECUTION_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "Spot/Linear",
            "私有 order topic",
            BYBIT_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            READY,
            Some("order.create"),
            "Linear",
            "trade WS request",
            BYBIT_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            READY,
            Some("order.cancel"),
            "Linear",
            "trade WS request",
            BYBIT_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            READY,
            Some("order.create reduceOnly"),
            "Linear",
            "平仓按 reduceOnly/qty 映射",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "Spot/Linear",
            "成交/撤单异步确认",
            BYBIT_ORDER_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret"],
        docs: BYBIT_DOCS,
        note: "trade WS ack 不等于最终状态；最终以 private order topic 对账。",
    },
    WsVenueSpec {
        venue: "bitget",
        label: "Bitget",
        public_endpoint: "wss://ws.bitget.com/v3/ws/public",
        private_endpoint: Some("wss://ws.bitget.com/v3/ws/private"),
        trade_endpoint: Some("wss://ws.bitget.com/v3/ws/private"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("account"),
            "UTA",
            "私有账户 topic",
            BITGET_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("position"),
            "UTA",
            "UTA 全局私有仓位 topic",
            BITGET_POSITION_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("fill"),
            "UTA",
            "UTA 全局成交 topic",
            BITGET_FILL_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "UTA",
            "UTA 全局私有订单 topic",
            BITGET_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            PERMISSION,
            Some("place-order"),
            "USDT/USDC-FUTURES",
            "需要交易权限和私有登录",
            BITGET_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            PERMISSION,
            Some("cancel-order"),
            "USDT/USDC-FUTURES",
            "需要交易权限和私有登录",
            BITGET_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            PERMISSION,
            Some("place-order close"),
            "USDT-FUTURES",
            "按 hedge/one-way 映射 close/reduceOnly",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "UTA",
            "订单状态推送",
            BITGET_ORDER_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret", "passphrase"],
        docs: BITGET_DOCS,
        note: "USDT/USDC 写单按账户 holdMode 生成 posSide/reduceOnly；COIN/Reality fail-closed。",
    },
    WsVenueSpec {
        venue: "gate",
        label: "Gate",
        public_endpoint: "wss://fx-ws.gateio.ws/v4/ws/usdt",
        private_endpoint: Some("wss://fx-ws.gateio.ws/v4/ws/usdt"),
        trade_endpoint: Some("wss://fx-ws.gateio.ws/v4/ws/usdt"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("futures.balances"),
            "USDT futures",
            "私有余额 channel",
            GATE_PRIVATE_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("futures.positions"),
            "USDT futures",
            "私有仓位 channel",
            GATE_PRIVATE_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("futures.usertrades"),
            "USDT futures",
            "逐笔成交/费用 channel",
            GATE_USERTRADES_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("futures.orders"),
            "USDT futures",
            "私有订单 channel",
            GATE_PRIVATE_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            READY,
            Some("futures.order_place"),
            "USDT futures",
            "WS API 下单",
            GATE_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            READY,
            Some("futures.order_cancel"),
            "USDT futures",
            "WS API 撤单",
            GATE_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            READY,
            Some("futures.order_place close"),
            "USDT futures",
            "按 size/auto_size 处理平仓",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("futures.order_status"),
            "USDT futures",
            "WS 点查；futures.orders 继续承担热更新，futures.order_list 承担未结订单快照",
            GATE_ORDER_STATUS_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret"],
        docs: GATE_DOCS,
        note: "futures WS 的 settlement 在路径中固定为 usdt；symbol 使用 BTC_USDT 风格；握手携带 X-Gate-Size-Decimal=1。",
    },
    WsVenueSpec {
        venue: "kucoin",
        label: "KuCoin",
        public_endpoint: "bullet-public negotiated websocket",
        private_endpoint: Some("bullet-private negotiated websocket"),
        trade_endpoint: Some("wss://wsapi.kucoin.com/v1/private"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("/contractAccount/wallet"),
            "Futures",
            "私有余额变更",
            KUCOIN_PRIVATE_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("/contract/positionAll"),
            "Futures",
            "私有仓位变更",
            KUCOIN_PRIVATE_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("/contractMarket/tradeOrders#match"),
            "Futures",
            "Classic match 推送逐笔 identity/price/size；实际 fee 由 signed REST fills 补齐",
            KUCOIN_ORDER_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("/contractMarket/tradeOrders"),
            "Futures",
            "私有订单变更",
            KUCOIN_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            SCHEMA_PENDING,
            Some("futures.order"),
            "Futures",
            "Pro WS schema 已发布但官方仍标注 beta，生产 writer 继续使用 Classic REST 单次提交",
            KUCOIN_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            SCHEMA_PENDING,
            Some("futures.cancel"),
            "Futures",
            "Pro WS schema 已发布但官方仍标注 beta，生产 writer 继续使用 Classic REST 单次提交",
            KUCOIN_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            SCHEMA_PENDING,
            Some("futures.order reduceOnly"),
            "Futures",
            "Pro WS futures.order 支持 reduceOnly，但官方仍标注 beta，保持 display-only",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("/contractMarket/tradeOrders"),
            "Futures",
            "订单状态推送",
            KUCOIN_ORDER_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret", "passphrase"],
        docs: KUCOIN_DOCS,
        note: "Classic 私有流继续使用 bullet token；Pro WS 官方仍标注 beta，不进入生产 writer。保留 Classic REST 单次提交，禁止 WS 失败后 REST 重放。",
    },
    WsVenueSpec {
        venue: "hyperliquid",
        label: "Hyperliquid",
        public_endpoint: "wss://api.hyperliquid.xyz/ws",
        private_endpoint: Some("wss://api.hyperliquid.xyz/ws"),
        trade_endpoint: Some("wss://api.hyperliquid.xyz/ws"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("clearinghouseState/spotState"),
            "Perp/Spot",
            "按 user 订阅账户状态",
            HYPERLIQUID_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("clearinghouseState"),
            "Perp",
            "按 user 订阅永续仓位状态",
            HYPERLIQUID_ACCOUNT_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("userFills"),
            "Perp/Spot",
            "按 user 订阅逐笔成交",
            HYPERLIQUID_FILL_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("orderUpdates"),
            "Perp/Spot",
            "按 user 订阅",
            HYPERLIQUID_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            READY,
            Some("post/order"),
            "Perp",
            "WS post request 转 exchange action",
            HYPERLIQUID_ORDER_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            READY,
            Some("post/cancel"),
            "Perp",
            "WS post request 转 exchange action",
            HYPERLIQUID_ORDER_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            READY,
            Some("post/order reduceOnly"),
            "Perp",
            "reduceOnly=true 平仓",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("post/info orderStatus"),
            "Perp/Spot",
            "WS info 点查；orderUpdates 继续承担热更新",
            HYPERLIQUID_ORDER_STATUS_WS_EVIDENCE,
        ),
        auth_fields: &["user_address", "private_key"],
        docs: HYPERLIQUID_DOCS,
        note: "写单签名仍是 L1 action signing；WS post 只改变传输层。",
    },
    WsVenueSpec {
        venue: "gate_crossex",
        label: "Gate CrossEx",
        public_endpoint: "wss://api.gateio.ws/ws/crossex/public",
        private_endpoint: Some("wss://api.gateio.ws/ws/crossex"),
        trade_endpoint: Some("wss://api.gateio.ws/ws/crossex"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("asset"),
            "Spot/Margin/Future",
            "登录后订阅 asset；每个 channel 取得真实数据样本后才算可用",
            GATE_CROSSEX_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("position"),
            "Future",
            "position 支持 !all；底层交易所身份保留在 symbol 中",
            GATE_CROSSEX_POSITION_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("usertrades"),
            "Spot/Margin/Future",
            "transaction_id 去重后合并逐笔数量、均价与费用",
            GATE_CROSSEX_FILL_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "Spot/Margin/Future",
            "订单变更热路径；冷启动和缺口恢复使用有界 CrossEx REST",
            GATE_CROSSEX_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            PERMISSION,
            Some("place_order"),
            "Spot/Margin/Future",
            "专用 CrossEx 私有 WS 写单；ACK 不代表成交终态",
            GATE_CROSSEX_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            PERMISSION,
            Some("cancel_order"),
            "Spot/Margin/Future",
            "专用 CrossEx 私有 WS 撤单；最终状态由 order/usertrades 对账",
            GATE_CROSSEX_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            false,
            SCHEMA_PENDING,
            Some("close_position"),
            "Future",
            "官方有专用 WS close_position，但当前 adapter 尚未接入该独立命令",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("order"),
            "Spot/Margin/Future",
            "order 承担实时状态；身份点查、分页和恢复使用有界 CrossEx REST",
            GATE_CROSSEX_ORDER_WS_EVIDENCE,
        ),
        auth_fields: &["api_key", "api_secret"],
        docs: GATE_CROSSEX_DOCS,
        note: "只使用 CrossEx 专用 public/private socket；公开订阅必须传明确 symbol，私有四个 channel 共用一个登录会话，协议 Ping 不超过 30 秒，凭证释放时共享会话随 Weak registry 退出。",
    },
    WsVenueSpec {
        venue: "kraken",
        label: "Kraken",
        public_endpoint: "wss://ws.kraken.com/v2 | wss://futures.kraken.com/ws/v1",
        private_endpoint: Some(
            "wss://ws-auth.kraken.com/v2 | wss://futures.kraken.com/ws/v1",
        ),
        trade_endpoint: Some("wss://ws-auth.kraken.com/v2 (Spot only)"),
        account_stream: ws_op_with_evidence(
            true,
            READY,
            Some("balances"),
            "Spot/Derivatives",
            "Spot balances 与 Derivatives balances 分会话接入；运行态逐流取证",
            KRAKEN_ACCOUNT_WS_EVIDENCE,
        ),
        position_stream: ws_op_with_evidence(
            true,
            READY,
            Some("open_positions"),
            "Derivatives",
            "签名 challenge 后订阅 open_positions",
            KRAKEN_POSITION_WS_EVIDENCE,
        ),
        fill_stream: ws_op_with_evidence(
            true,
            READY,
            Some("executions / fills"),
            "Spot/Derivatives",
            "Spot executions 与 Derivatives fills 共同提供成交终态",
            KRAKEN_FILL_WS_EVIDENCE,
        ),
        order_stream: ws_op_with_evidence(
            true,
            READY,
            Some("executions / open_orders"),
            "Spot/Derivatives",
            "Spot v2 executions 与 Derivatives open_orders 分会话接入",
            KRAKEN_ORDER_WS_EVIDENCE,
        ),
        place_order: ws_op_with_evidence(
            true,
            PERMISSION,
            Some("add_order"),
            "Spot",
            "Spot 使用 WS v2 add_order；Derivatives 官方 WS 仅推流，写单保持签名 REST v3",
            KRAKEN_PLACE_WS_EVIDENCE,
        ),
        cancel_order: ws_op_with_evidence(
            true,
            PERMISSION,
            Some("cancel_order"),
            "Spot",
            "Spot 使用 WS v2 cancel_order；Derivatives 撤单保持签名 REST v3",
            KRAKEN_CANCEL_WS_EVIDENCE,
        ),
        close_position: ws_op(
            true,
            ExchangeWsSupportStatus::RestOnly,
            Some("sendorder reduceOnly"),
            "Derivatives",
            "官方 Derivatives WS 不提供写请求，减仓平仓使用签名 REST v3",
        ),
        order_status: ws_op_with_evidence(
            true,
            READY,
            Some("executions / open_orders"),
            "Spot/Derivatives",
            "WS 承担热状态；无缓存身份时才使用有界签名 REST 点查",
            KRAKEN_ORDER_STATUS_WS_EVIDENCE,
        ),
        auth_fields: &[
            "spot_api_key",
            "spot_api_secret",
            "futures_api_key",
            "futures_api_secret",
        ],
        docs: KRAKEN_DOCS,
        note: "Spot 使用 WS v2 与 REST 发放的 WebSockets token；Derivatives 使用签名 challenge 的私有推流，官方未提供 Futures WS 写命令，因此 place/cancel/query 不伪装成 WS。两个私有会话按已配置产品独立启动并在凭证释放时退出。",
    },
];

pub fn trading_ws_venues() -> ExchangeWsVenuesResponse {
    ExchangeWsVenuesResponse {
        venues: SPECS.iter().map(to_venue).collect(),
    }
}

pub fn trading_ws_operation_registry() -> ExchangeWsOperationsResponse {
    ExchangeWsOperationsResponse {
        venues: SPECS.iter().map(to_operation_venue).collect(),
    }
}

fn to_venue(spec: &WsVenueSpec) -> ExchangeWsVenue {
    ExchangeWsVenue {
        venue: spec.venue.to_owned(),
        label: spec.label.to_owned(),
        public_endpoint: spec.public_endpoint.to_owned(),
        private_endpoint: spec.private_endpoint.map(str::to_owned),
        trade_endpoint: spec.trade_endpoint.map(str::to_owned),
        account_stream: to_operation(spec.account_stream),
        position_stream: to_operation(spec.position_stream),
        fill_stream: to_operation(spec.fill_stream),
        order_stream: to_operation(spec.order_stream),
        place_order: to_operation(spec.place_order),
        cancel_order: to_operation(spec.cancel_order),
        close_position: to_operation(spec.close_position),
        order_status: to_operation(spec.order_status),
        auth_fields: spec
            .auth_fields
            .iter()
            .map(|field| (*field).to_owned())
            .collect(),
        docs: spec.docs.iter().map(to_doc).collect(),
        note: spec.note.to_owned(),
    }
}

fn to_operation_venue(spec: &WsVenueSpec) -> ExchangeWsOperationVenue {
    ExchangeWsOperationVenue {
        venue: spec.venue.to_owned(),
        label: spec.label.to_owned(),
        operations: operation_specs(spec)
            .into_iter()
            .filter_map(|(label, operation)| to_operation_registry_row(label, operation))
            .collect(),
    }
}

fn operation_specs(spec: &WsVenueSpec) -> [(&'static str, WsOperationSpec); 8] {
    [
        ("account_stream", spec.account_stream),
        ("position_stream", spec.position_stream),
        ("fill_stream", spec.fill_stream),
        ("order_stream", spec.order_stream),
        ("place_order", spec.place_order),
        ("cancel_order", spec.cancel_order),
        ("close_position", spec.close_position),
        ("order_status", spec.order_status),
    ]
}

fn to_operation_registry_row(
    label: &'static str,
    spec: WsOperationSpec,
) -> Option<ExchangeWsOperationRegistryRow> {
    let evidence = spec.evidence?;
    Some(ExchangeWsOperationRegistryRow {
        label: label.to_owned(),
        evidence_scope: evidence_scope_for_label(label),
        supported: spec.supported,
        status: spec.status,
        release_status: evidence.release_status,
        requires_authenticated_runtime_evidence: evidence.requires_authenticated_runtime_evidence,
        authenticated_runtime_evidence: evidence.authenticated_runtime_evidence,
        operation: spec.operation.map(str::to_owned),
        product: spec.product.to_owned(),
        note: spec.note.to_owned(),
        checked_at: evidence.checked_at.to_owned(),
        doc_version: evidence.doc_version.to_owned(),
        doc_url: evidence.doc_url.to_owned(),
        parser_test: evidence.parser_test.map(str::to_owned),
        subscription_test: evidence.subscription_test.map(str::to_owned),
        fixture_id: evidence.fixture_id.map(str::to_owned),
        fixture_hash: evidence.fixture_hash.map(str::to_owned),
        auth_kind: evidence.auth_kind.to_owned(),
    })
}

fn evidence_scope_for_label(label: &str) -> ExchangeWsEvidenceScope {
    match label {
        "account_stream" => ExchangeWsEvidenceScope::PrivateAccountStream,
        "position_stream" => ExchangeWsEvidenceScope::PrivatePositionStream,
        "fill_stream" => ExchangeWsEvidenceScope::PrivateFillStream,
        "order_stream" => ExchangeWsEvidenceScope::PrivateOrderStream,
        "place_order" | "cancel_order" => ExchangeWsEvidenceScope::AckOnly,
        "order_status" => ExchangeWsEvidenceScope::OrderStatusRead,
        _ => ExchangeWsEvidenceScope::Unknown,
    }
}

fn to_operation(spec: WsOperationSpec) -> ExchangeWsOperation {
    ExchangeWsOperation {
        supported: spec.supported,
        status: spec.status,
        operation: spec.operation.map(str::to_owned),
        product: spec.product.to_owned(),
        note: spec.note.to_owned(),
        evidence: spec.evidence.map(to_evidence),
    }
}

fn to_evidence(spec: WsOperationEvidenceSpec) -> ExchangeWsOperationEvidence {
    ExchangeWsOperationEvidence {
        release_status: spec.release_status,
        requires_authenticated_runtime_evidence: spec.requires_authenticated_runtime_evidence,
        authenticated_runtime_evidence: spec.authenticated_runtime_evidence,
        checked_at: spec.checked_at.to_owned(),
        doc_version: spec.doc_version.to_owned(),
        doc_url: spec.doc_url.to_owned(),
        parser_test: spec.parser_test.map(str::to_owned),
        subscription_test: spec.subscription_test.map(str::to_owned),
        fixture_id: spec.fixture_id.map(str::to_owned),
        fixture_hash: spec.fixture_hash.map(str::to_owned),
        auth_kind: spec.auth_kind.to_owned(),
    }
}

fn to_doc(spec: &WsDocSpec) -> ExchangeWsDoc {
    ExchangeWsDoc {
        label: spec.label.to_owned(),
        url: spec.url.to_owned(),
    }
}

const fn doc(label: &'static str, url: &'static str) -> WsDocSpec {
    WsDocSpec { label, url }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const EXPECTED_EVIDENCED_OPERATION_ROWS: usize = 63;

    #[test]
    fn retired_htx_is_not_registered_for_ws() {
        assert!(trading_ws_venues()
            .venues
            .iter()
            .all(|venue| venue.venue != "htx"));
        assert!(trading_ws_operation_registry()
            .venues
            .iter()
            .all(|venue| venue.venue != "htx"));
    }

    #[test]
    fn ws_operation_registry_projects_all_recorded_evidence() {
        let registry = trading_ws_operation_registry();
        assert_eq!(TRADING_WS_VENUE_COUNT, registry.venues.len());

        let rows: Vec<&ExchangeWsOperationRegistryRow> = registry
            .venues
            .iter()
            .flat_map(|venue| venue.operations.iter())
            .collect();
        assert_eq!(EXPECTED_EVIDENCED_OPERATION_ROWS, rows.len());
        assert!(rows.iter().all(|row| row.doc_url.starts_with("https://")));
        assert!(rows.iter().all(|row| !row.checked_at.trim().is_empty()));
        assert!(rows.iter().all(|row| !row.doc_version.trim().is_empty()));
        assert!(rows.iter().all(|row| !row.auth_kind.trim().is_empty()));
        assert!(rows
            .iter()
            .all(|row| row.evidence_scope != ExchangeWsEvidenceScope::Unknown));
        assert!(rows.iter().all(|row| has_non_empty(&row.parser_test)));
        assert!(rows.iter().all(|row| has_non_empty(&row.subscription_test)));
    }

    #[test]
    fn ws_operation_registry_keeps_display_only_paths_out() {
        let registry = trading_ws_operation_registry();
        let kucoin = registry
            .venues
            .iter()
            .find(|venue| venue.venue == "kucoin")
            .expect("kucoin venue is registered");

        assert_eq!(7, kucoin.operations.len());
        assert!(kucoin
            .operations
            .iter()
            .any(|operation| operation.label == "fill_stream"));

        assert!(registry.venues.iter().all(|venue| {
            venue
                .operations
                .iter()
                .all(|operation| operation.label != "close_position")
        }));
    }

    #[test]
    fn kraken_and_gate_crossex_keep_official_transport_boundaries() {
        let venues = trading_ws_venues();
        let kraken = venues
            .venues
            .iter()
            .find(|venue| venue.venue == "kraken")
            .expect("kraken venue");
        assert_eq!(
            kraken.close_position.status,
            ExchangeWsSupportStatus::RestOnly
        );
        assert_eq!(kraken.place_order.product, "Spot");
        assert!(kraken.place_order.note.contains("Derivatives"));
        assert!(kraken.place_order.note.contains("REST v3"));

        let crossex = venues
            .venues
            .iter()
            .find(|venue| venue.venue == "gate_crossex")
            .expect("gate crossex venue");
        assert_eq!(
            crossex.public_endpoint,
            "wss://api.gateio.ws/ws/crossex/public"
        );
        assert_eq!(
            crossex.private_endpoint.as_deref(),
            Some("wss://api.gateio.ws/ws/crossex")
        );
        assert!(!crossex.close_position.supported);
    }

    #[test]
    fn ws_operation_registry_keeps_close_position_display_only() {
        let venues = trading_ws_venues();
        let registry = trading_ws_operation_registry();

        let close_position_rows = registry
            .venues
            .iter()
            .flat_map(|venue| venue.operations.iter())
            .filter(|operation| operation.label == "close_position")
            .count();
        assert_eq!(0, close_position_rows);

        for venue in venues.venues {
            assert!(
                venue.close_position.evidence.is_none(),
                "{} close_position must not carry registry evidence",
                venue.venue
            );
            assert!(
                !venue.close_position.is_live_submittable(),
                "{} close_position must stay display-only",
                venue.venue
            );
        }
    }

    #[test]
    fn ws_operation_registry_identity_is_unique_per_venue() {
        let registry = trading_ws_operation_registry();
        let mut seen = BTreeSet::new();

        for venue in &registry.venues {
            for operation in &venue.operations {
                assert!(
                    seen.insert((venue.venue.as_str(), operation.label.as_str())),
                    "duplicate WS operation registry identity: {}/{}",
                    venue.venue,
                    operation.label
                );
            }
        }
    }

    #[test]
    fn ws_operation_registry_keeps_ack_rows_out_of_finality_evidence() {
        let registry = trading_ws_operation_registry();
        let mut ack_rows = Vec::new();
        let mut order_status_rows = 0;
        let mut fill_rows = 0;
        let mut close_position_rows = 0;

        for venue in &registry.venues {
            for operation in &venue.operations {
                match operation.label.as_str() {
                    "place_order" | "cancel_order" => ack_rows.push((venue, operation)),
                    "order_status" => order_status_rows += 1,
                    "fill_stream" => fill_rows += 1,
                    "close_position" => close_position_rows += 1,
                    _ => {}
                }
            }
        }

        assert_eq!(TRADING_WS_VENUE_COUNT * 2, ack_rows.len());
        assert_eq!(TRADING_WS_VENUE_COUNT, order_status_rows);
        assert_eq!(TRADING_WS_VENUE_COUNT, fill_rows);
        assert_eq!(0, close_position_rows);

        for (venue, operation) in ack_rows {
            assert_eq!(
                ExchangeWsEvidenceScope::AckOnly,
                operation.evidence_scope,
                "{}/{} write evidence must stay ACK-scoped: {:?}",
                venue.venue,
                operation.label,
                operation
            );
        }
    }

    #[test]
    fn ws_operation_registry_projects_fixture_metadata_by_scope() {
        let registry = trading_ws_operation_registry();
        let mut ack_rows = 0;

        for venue in &registry.venues {
            for operation in &venue.operations {
                if matches!(operation.label.as_str(), "place_order" | "cancel_order") {
                    ack_rows += 1;
                    assert!(
                        operation
                            .fixture_id
                            .as_deref()
                            .is_some_and(|fixture| fixture
                                .starts_with("crates/exchange/fixtures/")),
                        "{}/{} missing canonical fixture id",
                        venue.venue,
                        operation.label
                    );
                    assert!(
                        operation
                            .fixture_hash
                            .as_deref()
                            .is_some_and(is_canonical_sha256),
                        "{}/{} missing canonical fixture hash",
                        venue.venue,
                        operation.label
                    );
                } else if operation.fixture_id.is_some() || operation.fixture_hash.is_some() {
                    assert_ne!(
                        ExchangeWsEvidenceScope::AckOnly,
                        operation.evidence_scope,
                        "{}/{} non-write fixture must keep its private/read scope",
                        venue.venue,
                        operation.label
                    );
                    assert!(
                        operation
                            .fixture_id
                            .as_deref()
                            .is_some_and(|fixture| fixture.starts_with("crates/exchange/fixtures/")),
                        "{}/{} missing canonical private fixture id",
                        venue.venue,
                        operation.label
                    );
                    assert!(
                        operation
                            .fixture_hash
                            .as_deref()
                            .is_some_and(is_canonical_sha256),
                        "{}/{} missing canonical private fixture hash",
                        venue.venue,
                        operation.label
                    );
                }
            }
        }

        assert_eq!(TRADING_WS_VENUE_COUNT * 2, ack_rows);
    }

    #[test]
    fn ws_operation_registry_uses_typed_evidence_scope_boundaries() {
        let registry = trading_ws_operation_registry();
        let mut counts = EvidenceScopeCounts::default();

        for venue in &registry.venues {
            for operation in &venue.operations {
                counts.record(venue.venue.as_str(), operation);
            }
        }

        counts.assert_expected();
    }

    #[derive(Default)]
    struct EvidenceScopeCounts {
        account_rows: usize,
        position_rows: usize,
        fill_rows: usize,
        order_stream_rows: usize,
        ack_rows: usize,
        order_status_rows: usize,
    }

    impl EvidenceScopeCounts {
        fn record(&mut self, venue: &str, operation: &ExchangeWsOperationRegistryRow) {
            match operation.label.as_str() {
                "account_stream" => Self::record_scope(
                    &mut self.account_rows,
                    operation,
                    ExchangeWsEvidenceScope::PrivateAccountStream,
                ),
                "position_stream" => Self::record_scope(
                    &mut self.position_rows,
                    operation,
                    ExchangeWsEvidenceScope::PrivatePositionStream,
                ),
                "fill_stream" => Self::record_scope(
                    &mut self.fill_rows,
                    operation,
                    ExchangeWsEvidenceScope::PrivateFillStream,
                ),
                "order_stream" => Self::record_scope(
                    &mut self.order_stream_rows,
                    operation,
                    ExchangeWsEvidenceScope::PrivateOrderStream,
                ),
                "place_order" | "cancel_order" => Self::record_scope(
                    &mut self.ack_rows,
                    operation,
                    ExchangeWsEvidenceScope::AckOnly,
                ),
                "order_status" => Self::record_scope(
                    &mut self.order_status_rows,
                    operation,
                    ExchangeWsEvidenceScope::OrderStatusRead,
                ),
                label => panic!("{venue}/{label} has unexpected registry label"),
            }
        }

        fn record_scope(
            count: &mut usize,
            operation: &ExchangeWsOperationRegistryRow,
            expected: ExchangeWsEvidenceScope,
        ) {
            *count += 1;
            assert_eq!(expected, operation.evidence_scope);
        }

        fn assert_expected(&self) {
            assert_eq!(TRADING_WS_VENUE_COUNT, self.account_rows);
            assert_eq!(TRADING_WS_VENUE_COUNT, self.position_rows);
            assert_eq!(TRADING_WS_VENUE_COUNT, self.fill_rows);
            assert_eq!(TRADING_WS_VENUE_COUNT, self.order_stream_rows);
            assert_eq!(TRADING_WS_VENUE_COUNT * 2, self.ack_rows);
            assert_eq!(TRADING_WS_VENUE_COUNT, self.order_status_rows);
        }
    }

    #[test]
    fn ws_operation_registry_keeps_kucoin_schema_pending_writes_non_ready() {
        let registry = trading_ws_operation_registry();

        let kucoin = registry
            .venues
            .iter()
            .find(|venue| venue.venue == "kucoin")
            .expect("kucoin");
        for label in ["place_order", "cancel_order"] {
            let kucoin_operation = kucoin
                .operations
                .iter()
                .find(|operation| operation.label == label)
                .expect("kucoin write evidence row");
            assert_eq!(SCHEMA_PENDING, kucoin_operation.status);
            assert_eq!(
                ExchangeWsReleaseStatus::BetaUnavailable,
                kucoin_operation.release_status
            );
            assert!(!kucoin_operation.requires_authenticated_runtime_evidence);
            assert!(!kucoin_operation.authenticated_runtime_evidence);
        }
    }

    fn has_non_empty(value: &Option<String>) -> bool {
        value
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    }

    fn is_canonical_sha256(value: &str) -> bool {
        let Some(digest) = value.strip_prefix("sha256:") else {
            return false;
        };
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }
}
