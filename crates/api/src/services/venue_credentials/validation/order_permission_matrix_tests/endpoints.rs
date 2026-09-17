struct EndpointCase {
    venue: exchange::VenueId,
    method: exchange::HttpMethod,
    path: &'static str,
    use_case: exchange::EndpointUseCase,
    data_kind: exchange::EndpointDataKind,
}

const ORDER_PERMISSION_ENDPOINT_CASES: &[EndpointCase] = &[
    EndpointCase {
        venue: exchange::VenueId::Binance,
        method: exchange::HttpMethod::Post,
        path: "/fapi/v1/order/test",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Binance,
        method: exchange::HttpMethod::Delete,
        path: "/fapi/v1/order",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Okx,
        method: exchange::HttpMethod::Post,
        path: "/api/v5/trade/order-precheck",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Bybit,
        method: exchange::HttpMethod::Post,
        path: "/v5/order/pre-check",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Bitget,
        method: exchange::HttpMethod::Post,
        path: "/api/v3/trade/cancel-order",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Gate,
        method: exchange::HttpMethod::Delete,
        path: "/api/v4/futures/usdt/orders/{order_id}",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Kucoin,
        method: exchange::HttpMethod::Post,
        path: "/api/v1/orders/test",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Kucoin,
        method: exchange::HttpMethod::Delete,
        path: "/api/v1/orders/client-order/{clientOid}",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
    EndpointCase {
        venue: exchange::VenueId::Hyperliquid,
        method: exchange::HttpMethod::Post,
        path: "/exchange",
        use_case: exchange::EndpointUseCase::TradeWrite,
        data_kind: exchange::EndpointDataKind::OrderAck,
    },
];

#[test]
fn save_time_order_permission_probe_sources_have_endpoint_specs() {
    for case in ORDER_PERMISSION_ENDPOINT_CASES {
        assert!(
            exchange::venue_spec::ENDPOINT_SPECS.iter().any(|spec| {
                spec.venue == case.venue
                    && spec.method == case.method
                    && spec.path == case.path
                    && spec.use_case == case.use_case
                    && spec.data_kind == case.data_kind
            }),
            "missing EndpointSpec for {:?} {} {}",
            case.venue,
            case.method.as_str(),
            case.path
        );
    }
}
