use super::*;
use std::fmt::Debug;

#[allow(clippy::panic)]
fn must_ok<T, E: Debug>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error:?}"),
    }
}

fn credentials() -> AdapterCredentials {
    AdapterCredentials {
        binance_live: Some(("lk".into(), "ls".into())),
        bitget_live: Some(("gk".into(), "gs".into(), "gp".into())),
        bybit_live: Some(("bk".into(), "bs".into())),
        gate_live: Some(("gtk".into(), "gts".into())),
        gate_crossex_live: None,
        hyperliquid_live: Some(HyperliquidAdapterCredentials {
            account_address: "0x0000000000000000000000000000000000000001".into(),
            private_key: "0101010101010101010101010101010101010101010101010101010101010101".into(),
            vault_address: None,
        }),
        kucoin_live: Some(("kk".into(), "ks".into(), "kp".into())),
        kraken_live: None,
        okx_live: Some(("lk".into(), "ls".into(), "lp".into())),
    }
}

#[test]
fn dispatcher_includes_gate_crossex_and_configured_kraken() {
    let credentials = AdapterCredentials {
        gate_crossex_live: Some(("cgk".into(), "cgs".into())),
        kraken_live: Some(KrakenAdapterCredentials {
            spot: Some(("ksk".into(), "kss".into())),
            futures: None,
        }),
        ..AdapterCredentials::default()
    };

    assert_eq!(
        dispatcher_venues_for(&credentials),
        vec!["gate_crossex".to_owned(), "kraken".to_owned()]
    );
}

#[test]
fn select_binance_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_binance_live_adapter("k".into(), "s".into()),
        "binance live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), BINANCE_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("binance"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn select_bybit_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_bybit_live_adapter("k".into(), "s".into()),
        "bybit live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), BYBIT_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("bybit"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn select_bitget_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_bitget_live_adapter("k".into(), "s".into(), "p".into()),
        "bitget live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), BITGET_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("bitget"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn select_gate_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_gate_live_adapter("k".into(), "s".into()),
        "gate live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), GATE_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("gate"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn select_kucoin_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_kucoin_live_adapter("k".into(), "s".into(), "p".into()),
        "kucoin live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), KUCOIN_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("kucoin"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn select_okx_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_okx_live_adapter("k".into(), "s".into(), "p".into()),
        "okx live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), OKX_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("okx"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn select_hyperliquid_live_adapter_sets_live_risk() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.select_hyperliquid_live_adapter(
            "0x0000000000000000000000000000000000000001".into(),
            "0101010101010101010101010101010101010101010101010101010101010101".into(),
        ),
        "hyperliquid live adapter constructs offline",
    );
    assert_eq!(service.adapter_name(), HYPERLIQUID_LIVE_ADAPTER_ID);
    assert!(risk.allowed_exchanges.contains("hyperliquid"));
    assert!(risk.live_trading_enabled);
}

#[test]
fn try_select_live_router_does_not_require_readiness() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, credentials()),
        "live router should only require configured credentials",
    );
    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    assert!(risk.live_trading_enabled);
    assert_eq!(
        service.account_reader_venues(),
        risk.allowed_exchanges.into_iter().collect::<Vec<_>>()
    );
}

#[test]
fn try_select_live_router_allows_partial_live_credentials() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.try_select_adapter(
            LIVE_ROUTER_ADAPTER_ID,
            AdapterCredentials {
                kucoin_live: None,
                ..credentials()
            },
        ),
        "live router should enable with the configured venue routes",
    );
    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    assert!(risk.live_trading_enabled);
    assert!(risk.allowed_exchanges.contains("binance"));
    assert!(!risk.allowed_exchanges.contains("kucoin"));
}

#[test]
fn try_select_live_router_with_all_credentials_enables_all_routes() {
    let service = TradingService::new_mock();
    let risk = must_ok(
        service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, credentials()),
        "ready live router switch",
    );
    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    assert!(risk.live_trading_enabled);
    assert!(risk.allowed_exchanges.contains("hyperliquid"));
    assert!(risk.allowed_exchanges.contains("hyperliquid:xyz"));
    assert!(!risk.allowed_exchanges.contains("hyperliquid:km"));
    assert!(risk.allowed_exchanges.contains("binance"));
    assert!(risk.allowed_exchanges.contains("kucoin"));
}
