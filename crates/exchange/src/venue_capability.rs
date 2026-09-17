use crate::{client_order_id_policy, trading_ws_operation_registry, ExchangeCapabilities};
use shared_types::{
    venue_family, ExchangeWsEvidenceScope, ExchangeWsReleaseStatus, FeeProduct, MarginMode,
    OrderPayloadPricePolicy, OrderType, TimeInForce, VenueAccountCapability, VenueCapabilityMatrix,
    VenueClientOrderIdCapability, VenueFinalityCapability, VenueInstrumentCapability,
    VenueMarketOrderStyle, VenueOrderCapability, VenueOrderKind,
};

pub const LIVE_VENUE_FAMILIES: [&str; 9] = [
    "binance",
    "okx",
    "bybit",
    "bitget",
    "gate",
    "gate_crossex",
    "kraken",
    "kucoin",
    "hyperliquid",
];

#[must_use]
pub fn static_exchange_capabilities(venue: &str) -> Option<ExchangeCapabilities> {
    let family = venue_family(venue);
    LIVE_VENUE_FAMILIES
        .contains(&family)
        .then(|| ExchangeCapabilities {
            supports_testnet: matches!(family, "binance" | "bybit" | "gate"),
            supports_live: true,
            supports_spot: true,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        })
}

#[must_use]
pub fn static_venue_capability_matrix(venue: &str) -> Option<VenueCapabilityMatrix> {
    static_venue_capability_matrix_for_product(venue, FeeProduct::Perp)
}

#[must_use]
pub fn static_venue_capability_matrix_for_product(
    venue: &str,
    product: FeeProduct,
) -> Option<VenueCapabilityMatrix> {
    static_exchange_capabilities(venue).map(|capabilities| {
        venue_capability_matrix_for_product(
            venue,
            capabilities,
            static_order_margin_modes(venue),
            product,
        )
    })
}

#[must_use]
pub fn venue_capability_matrix(
    venue: &str,
    capabilities: ExchangeCapabilities,
    order_margin_modes: Vec<MarginMode>,
) -> VenueCapabilityMatrix {
    venue_capability_matrix_for_product(venue, capabilities, order_margin_modes, FeeProduct::Perp)
}

#[must_use]
pub fn venue_capability_matrix_for_product(
    venue: &str,
    capabilities: ExchangeCapabilities,
    order_margin_modes: Vec<MarginMode>,
    product: FeeProduct,
) -> VenueCapabilityMatrix {
    let family = venue_family(venue);
    let policy = client_order_id_policy(venue, "capability-probe");
    let mut official_doc_urls = policy.official_doc_urls.clone();
    official_doc_urls.push(order_doc_url(family, product).to_owned());
    official_doc_urls.sort();
    official_doc_urls.dedup();

    VenueCapabilityMatrix {
        venue: family.to_owned(),
        product,
        orders: order_capabilities(family, capabilities, product),
        account: VenueAccountCapability {
            order_margin_modes: if product == FeeProduct::Spot {
                Vec::new()
            } else {
                order_margin_modes
            },
            account_mode_scope: account_mode_scope(family, product).to_owned(),
            runtime_account_mode_read: product != FeeProduct::Spot
                && !matches!(family, "hyperliquid" | "kraken"),
        },
        client_order_id: VenueClientOrderIdCapability {
            venue_field: policy.venue_field,
            policy_version: policy.policy_version,
            official_format: policy.official_format,
            max_length: policy.max_length,
            supports_query_by_client_id: policy.supports_query_by_client_id,
            supports_cancel_by_client_id: policy.supports_cancel_by_client_id,
        },
        instrument: VenueInstrumentCapability {
            native_symbol_required: true,
            instrument_spec_required: true,
            native_sizing_required: true,
        },
        finality: finality_capability(family, product),
        source:
            "exchange.venue_capability_matrix+client_order_id_policy+trading_ws_operation_registry"
                .to_owned(),
        official_doc_urls,
    }
}

fn order_capabilities(
    family: &str,
    capabilities: ExchangeCapabilities,
    product: FeeProduct,
) -> Vec<VenueOrderCapability> {
    let mut rows = Vec::with_capacity(3);
    if capabilities.supports_limit_orders {
        rows.push(order_capability(
            OrderType::Limit,
            OrderType::Limit,
            limit_time_in_force(family),
            Vec::new(),
            VenueOrderKind::Limit,
            OrderPayloadPricePolicy::LimitPrice,
        ));
    }
    if capabilities.supports_market_orders {
        rows.push(market_capability(family, product));
    }
    if capabilities.supports_limit_orders && capabilities.supports_post_only {
        rows.push(order_capability(
            OrderType::PostOnly,
            OrderType::PostOnly,
            vec![TimeInForce::Gtc],
            Vec::new(),
            VenueOrderKind::PostOnly,
            OrderPayloadPricePolicy::LimitPrice,
        ));
    }
    rows
}

fn market_capability(family: &str, product: FeeProduct) -> VenueOrderCapability {
    match (family, product) {
        ("gate", FeeProduct::Perp) => order_capability(
            OrderType::Market,
            OrderType::Market,
            vec![TimeInForce::Ioc],
            Vec::new(),
            VenueOrderKind::PriceZeroIoc,
            OrderPayloadPricePolicy::ZeroPrice,
        ),
        ("hyperliquid", _) => order_capability(
            OrderType::Market,
            OrderType::Limit,
            vec![TimeInForce::Ioc],
            Vec::new(),
            VenueOrderKind::ProtectedIoc,
            OrderPayloadPricePolicy::ProtectionPrice,
        ),
        _ => order_capability(
            OrderType::Market,
            OrderType::Market,
            vec![TimeInForce::Ioc],
            Vec::new(),
            VenueOrderKind::NativeMarket,
            OrderPayloadPricePolicy::Omit,
        ),
    }
}

fn order_capability(
    requested_order_type: OrderType,
    effective_order_type: OrderType,
    time_in_force: Vec<TimeInForce>,
    market_order_styles: Vec<VenueMarketOrderStyle>,
    venue_order_kind: VenueOrderKind,
    payload_price_policy: OrderPayloadPricePolicy,
) -> VenueOrderCapability {
    VenueOrderCapability {
        requested_order_type,
        effective_order_type,
        time_in_force,
        market_order_styles,
        venue_order_kind,
        payload_price_policy,
    }
}

fn limit_time_in_force(family: &str) -> Vec<TimeInForce> {
    match family {
        "gate" => vec![TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc],
        "gate_crossex" => vec![
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx,
        ],
        "kraken" => vec![TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc],
        "hyperliquid" => vec![TimeInForce::Ioc, TimeInForce::Gtc],
        _ => vec![
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx,
        ],
    }
}

fn static_order_margin_modes(venue: &str) -> Vec<MarginMode> {
    match venue_family(venue) {
        "okx" | "bitget" | "kucoin" => {
            vec![MarginMode::Cross, MarginMode::Isolated]
        }
        _ => Vec::new(),
    }
}

fn account_mode_scope(family: &str, product: FeeProduct) -> &'static str {
    if product == FeeProduct::Spot {
        return match family {
            "bitget" => "uta_spot",
            "hyperliquid" => "spot_portfolio",
            _ => "spot",
        };
    }
    match family {
        "binance" => "usds_m_futures",
        "okx" => "account_config",
        "bybit" => "linear_usdt_symbol",
        "bitget" => "uta",
        "gate" => "usdt_futures",
        "gate_crossex" => "cross_exchange_unified",
        "kraken" => "spot_and_derivatives_accounts",
        "kucoin" => "classic_futures",
        "hyperliquid" => "portfolio",
        _ => "unknown",
    }
}

fn finality_capability(family: &str, product: FeeProduct) -> VenueFinalityCapability {
    if product == FeeProduct::Spot {
        return spot_finality_capability(family);
    }
    let registry = trading_ws_operation_registry();
    let Some(venue) = registry
        .venues
        .iter()
        .find(|venue| venue_family(&venue.venue) == family)
    else {
        return VenueFinalityCapability::default();
    };
    let evidenced = |scope| {
        venue.operations.iter().any(|row| {
            row.evidence_scope == scope
                && row.supported
                && row.release_status == ExchangeWsReleaseStatus::ProductionReady
                && row.parser_test.is_some()
        })
    };
    let mut evidence_sources = venue
        .operations
        .iter()
        .filter(|row| {
            matches!(
                row.evidence_scope,
                ExchangeWsEvidenceScope::PrivateOrderStream
                    | ExchangeWsEvidenceScope::PrivateFillStream
                    | ExchangeWsEvidenceScope::OrderStatusRead
            ) && row.release_status == ExchangeWsReleaseStatus::ProductionReady
        })
        .map(|row| row.doc_url.clone())
        .collect::<Vec<_>>();
    evidence_sources.sort();
    evidence_sources.dedup();
    VenueFinalityCapability {
        ack_is_final: false,
        private_order_stream: evidenced(ExchangeWsEvidenceScope::PrivateOrderStream),
        private_fill_stream: evidenced(ExchangeWsEvidenceScope::PrivateFillStream),
        order_status_read: evidenced(ExchangeWsEvidenceScope::OrderStatusRead),
        evidence_sources,
    }
}

fn spot_finality_capability(family: &str) -> VenueFinalityCapability {
    let (order_updates, fill_updates, status_read, source) = match family {
        "binance" => (true, true, true, "executionReport+order.status"),
        "okx" => (true, true, true, "orders:ANY+GET /api/v5/trade/order"),
        "bybit" => (
            true,
            true,
            true,
            "order/execution spot+GET /v5/order/realtime",
        ),
        "bitget" => (true, true, true, "UTA order/fill+order-info"),
        "gate" => (true, true, true, "spot.orders+spot.order_status"),
        "gate_crossex" => (true, true, true, "order+usertrades+crossex order query"),
        "kraken" => (
            true,
            true,
            true,
            "executions+WS v2 cancel/order identifiers",
        ),
        "kucoin" => (true, true, true, "spotMarket/tradeOrdersV2+hf order query"),
        "hyperliquid" => (true, true, true, "orderUpdates/userFills+orderStatus"),
        _ => (false, false, false, "unknown"),
    };
    VenueFinalityCapability {
        ack_is_final: false,
        private_order_stream: order_updates,
        private_fill_stream: fill_updates,
        order_status_read: status_read,
        evidence_sources: vec![source.to_owned()],
    }
}

fn order_doc_url(family: &str, product: FeeProduct) -> &'static str {
    if product == FeeProduct::Spot {
        return match family {
            "binance" => "https://developers.binance.com/docs/binance-spot-api-docs/websocket-api/trading-requests",
            "okx" => "https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-place-order",
            "bybit" => "https://bybit-exchange.github.io/docs/v5/websocket/trade/guideline",
            "bitget" => "https://www.bitget.com/api-doc/uta/websocket/private/Place-Order-Channel",
            "gate" => "https://www.gate.com/docs/developers/apiv4/ws/en/",
            "gate_crossex" => "https://www.gate.com/docs/developers/crossex/ws/en/",
            "kraken" => "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/add_order",
            "kucoin" => "https://www.kucoin.com/docs-new/3470252w0",
            "hyperliquid" => "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint",
            _ => "https://example.invalid/unsupported-venue",
        };
    }
    match family {
        "binance" => "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/New-Order",
        "okx" => "https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order",
        "bybit" => "https://bybit-exchange.github.io/docs/v5/order/create-order",
        "bitget" => "https://www.bitget.com/api-doc/uta/trade/Place-Order",
        "gate" => "https://www.gate.com/docs/developers/apiv4/en/#futures-order",
        "gate_crossex" => "https://www.gate.com/docs/developers/crossex/ws/en/",
        "kraken" => "https://docs.kraken.com/exchange/api-reference/futures-rest-api",
        "kucoin" => "https://www.kucoin.com/docs-new/rest/futures-trading/orders/add-order",
        "hyperliquid" => "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_venue_contract(venue: &str, limit_tif: &[TimeInForce]) {
        let matrix = static_venue_capability_matrix(venue).expect("known live venue matrix");
        assert_eq!(matrix.venue, venue);
        assert_eq!(
            matrix
                .order(OrderType::Limit)
                .map(|row| row.time_in_force.as_slice()),
            Some(limit_tif)
        );
        assert!(matrix.order(OrderType::Market).is_some());
        assert!(matrix.order(OrderType::PostOnly).is_some());
        assert!(!matrix.client_order_id.venue_field.is_empty());
        assert!(matrix.instrument.native_symbol_required);
        assert!(matrix.instrument.instrument_spec_required);
        assert!(matrix.instrument.native_sizing_required);
        assert!(!matrix.finality.ack_is_final);
        assert!(matrix.finality.has_confirmed_path());
        assert!(!matrix.official_doc_urls.is_empty());
    }

    macro_rules! venue_contract_test {
        ($name:ident, $venue:literal, [$($tif:expr),+ $(,)?]) => {
            #[test]
            fn $name() {
                assert_venue_contract($venue, &[$($tif),+]);
            }
        };
    }

    venue_contract_test!(
        binance_matrix_contract,
        "binance",
        [
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx
        ]
    );
    venue_contract_test!(
        okx_matrix_contract,
        "okx",
        [
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx
        ]
    );
    venue_contract_test!(
        bybit_matrix_contract,
        "bybit",
        [
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx
        ]
    );
    venue_contract_test!(
        bitget_matrix_contract,
        "bitget",
        [
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx
        ]
    );
    venue_contract_test!(
        gate_matrix_contract,
        "gate",
        [TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc]
    );
    venue_contract_test!(
        kucoin_matrix_contract,
        "kucoin",
        [
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtc,
            TimeInForce::Gtx
        ]
    );
    venue_contract_test!(
        hyperliquid_matrix_contract,
        "hyperliquid",
        [TimeInForce::Ioc, TimeInForce::Gtc]
    );

    #[test]
    fn non_native_market_compilers_remain_explicit() {
        let gate = static_venue_capability_matrix("gate").expect("gate matrix");
        let hyperliquid = static_venue_capability_matrix("hyperliquid").expect("hl matrix");

        assert_eq!(
            gate.order(OrderType::Market)
                .map(|row| row.venue_order_kind),
            Some(VenueOrderKind::PriceZeroIoc)
        );
        assert_eq!(
            hyperliquid
                .order(OrderType::Market)
                .map(|row| row.effective_order_type),
            Some(OrderType::Limit)
        );
    }

    #[test]
    fn new_venue_account_mode_capabilities_match_official_interfaces() {
        let crossex = static_venue_capability_matrix("gate_crossex").expect("CrossEx matrix");
        let kraken = static_venue_capability_matrix("kraken").expect("Kraken matrix");

        assert!(crossex.account.runtime_account_mode_read);
        assert!(!kraken.account.runtime_account_mode_read);
    }
}
