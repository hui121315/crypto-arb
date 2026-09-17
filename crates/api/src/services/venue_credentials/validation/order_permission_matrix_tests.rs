use super::*;

mod endpoints;

pub(super) struct ProbeCase {
    pub(super) venue: &'static str,
    pub(super) scope: &'static str,
    pub(super) source: &'static str,
    pub(super) build: fn(&str, &str) -> VenueCredentialProbe,
    pub(super) expected_status: VenueCredentialProbeStatus,
    pub(super) expected_permissions: &'static [VenueCredentialPermission],
    pub(super) wiring_fn: &'static str,
    pub(super) message_fragments: &'static [&'static str],
}

pub(super) const ORDER_PERMISSION_PROBE_CASES: &[ProbeCase] = &[
    ProbeCase {
        venue: "binance",
        scope: "binance_usdm_futures_order_test_cancel_no_match",
        source: "binance.POST /fapi/v1/order/test + DELETE /fapi/v1/order",
        build: safe_order_place_cancel_test_probe,
        expected_status: VenueCredentialProbeStatus::Unknown,
        expected_permissions: &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
        wiring_fn: "optional_safe_order_place_cancel_test_probe",
        message_fragments: &["private order stream", "order finality"],
    },
    ProbeCase {
        venue: "okx",
        scope: "okx_order_precheck",
        source: "okx.POST /api/v5/trade/order-precheck",
        build: safe_order_pre_check_probe,
        expected_status: VenueCredentialProbeStatus::Unknown,
        expected_permissions: &[VenueCredentialPermission::PlaceOrder],
        wiring_fn: "optional_safe_order_pre_check_probe",
        message_fragments: &["cancel permission", "order finality"],
    },
    ProbeCase {
        venue: "bybit",
        scope: "bybit_linear_trade_read_write",
        source: "bybit.GET /v5/user/query-api",
        build: read_only_order_permission_probe,
        expected_status: VenueCredentialProbeStatus::Ok,
        expected_permissions: &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
        wiring_fn: "optional_order_permission_probe",
        message_fragments: &["read-only", "succeeded"],
    },
    ProbeCase {
        venue: "bitget",
        scope: "bitget_uta_trade_read_write",
        source: "bitget.GET /api/v3/account/info",
        build: read_only_order_permission_probe,
        expected_status: VenueCredentialProbeStatus::Ok,
        expected_permissions: &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
        wiring_fn: "optional_order_permission_probe",
        message_fragments: &["read-only", "succeeded"],
    },
    ProbeCase {
        venue: "gate",
        scope: "gate_futures_cancel_no_match",
        source: "gate.DELETE /api/v4/futures/usdt/orders/{order_id}",
        build: safe_order_cancel_no_match_probe,
        expected_status: VenueCredentialProbeStatus::Unknown,
        expected_permissions: &[VenueCredentialPermission::CancelOrder],
        wiring_fn: "optional_safe_order_cancel_no_match_probe",
        message_fragments: &["place permission", "order finality"],
    },
    ProbeCase {
        venue: "kucoin",
        scope: "kucoin_classic_futures_order_test_cancel_no_match",
        source: "kucoin.POST /api/v1/orders/test + DELETE /api/v1/orders/client-order/{clientOid}",
        build: safe_order_place_cancel_test_probe,
        expected_status: VenueCredentialProbeStatus::Unknown,
        expected_permissions: &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
        wiring_fn: "optional_safe_order_place_cancel_test_probe",
        message_fragments: &["private order stream", "order finality"],
    },
    ProbeCase {
        venue: "hyperliquid",
        scope: "hyperliquid_noop",
        source: "hyperliquid.POST /exchange action=noop",
        build: safe_order_noop_probe,
        expected_status: VenueCredentialProbeStatus::Ok,
        expected_permissions: &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
        wiring_fn: "optional_safe_order_noop_probe",
        message_fragments: &["consumed one nonce", "separate runtime gates"],
    },
];

fn read_only_order_permission_probe(scope: &str, source: &str) -> VenueCredentialProbe {
    probe(
        "order_permission",
        VenueCredentialProbeStatus::Ok,
        scope,
        source,
        "read-only order-permission probe succeeded",
    )
}

fn assert_order_permission_probe_case(case: &ProbeCase, venue_wiring: &str) {
    let probe = (case.build)(case.scope, case.source);

    assert_eq!(probe.kind, "order_permission", "{}", case.venue);
    assert_eq!(probe.status, case.expected_status, "{}", case.venue);
    assert_eq!(probe.scope, case.scope, "{}", case.venue);
    assert_eq!(probe.source, case.source, "{}", case.venue);
    case.message_fragments.iter().for_each(|fragment| {
        assert!(
            probe.message.contains(fragment),
            "{} message missing {fragment}: {}",
            case.venue,
            probe.message
        );
    });
    assert!(
        venue_wiring.contains(case.wiring_fn),
        "{} validator missing {}",
        case.venue,
        case.wiring_fn
    );
    assert!(
        venue_wiring.contains(&format!("\"{}\"", case.scope)),
        "{} validator missing scope {}",
        case.venue,
        case.scope
    );
    assert!(
        venue_wiring.contains(&format!("\"{}\"", case.source)),
        "{} validator missing source {}",
        case.venue,
        case.source
    );
}

#[test]
fn save_time_order_permission_probe_matrix_matches_validator_wiring() {
    let venue_wiring = [
        include_str!("venues.rs"),
        include_str!("venues/cex.rs"),
        include_str!("venues/hyperliquid.rs"),
    ]
    .concat();

    for case in ORDER_PERMISSION_PROBE_CASES {
        assert_order_permission_probe_case(case, &venue_wiring);
    }
}

#[test]
fn save_time_permission_matrix_distinguishes_open_place_and_cancel() {
    for case in ORDER_PERMISSION_PROBE_CASES {
        let evidence = evidence_with_order_permission_scopes(
            VenueCredentialValidationStatus::ReadOnlyOk,
            vec![
                probe(
                    "open_orders_read",
                    VenueCredentialProbeStatus::Ok,
                    "private_read.open_orders",
                    "exchange_adapter.get_open_orders",
                    "open orders read succeeded",
                ),
                (case.build)(case.scope, case.source),
            ],
            case.expected_permissions,
        );

        assert_eq!(
            evidence
                .permission(VenueCredentialPermission::OpenOrdersRead)
                .map(|permission| permission.status),
            Some(VenueCredentialPermissionStatus::Validated),
            "{} open orders",
            case.venue
        );
        for permission in [
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ] {
            let status = evidence
                .permission(permission)
                .map(|evidence| evidence.status);
            let expected = if case.expected_permissions.contains(&permission) {
                match case.expected_status {
                    VenueCredentialProbeStatus::Ok => VenueCredentialPermissionStatus::Validated,
                    VenueCredentialProbeStatus::Failed => VenueCredentialPermissionStatus::Denied,
                    VenueCredentialProbeStatus::Unknown => {
                        VenueCredentialPermissionStatus::Unproven
                    }
                }
            } else {
                VenueCredentialPermissionStatus::Missing
            };
            assert_eq!(
                status,
                Some(expected),
                "{} {}",
                case.venue,
                permission.as_str()
            );
        }
    }
}
