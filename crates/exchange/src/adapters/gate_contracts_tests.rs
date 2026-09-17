use super::*;
use pretty_assertions::assert_eq;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_row() -> GateContractMetadataRow {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let mut rows: Vec<GateContractMetadataRow> =
        serde_json::from_str(fixture).expect("official Gate contracts fixture must parse");
    assert_eq!(rows.len(), 1, "fixture row must not be skipped");
    rows.pop().expect("one Gate contract fixture row")
}

#[test]
fn official_fixture_closes_native_identity_and_contract_spec() {
    let spec = GateContractSpec::from_row(&fixture_row(), "usdt")
        .expect("official Gate contract metadata must validate");

    assert_contract_identity_and_limits(&spec);
    assert_contract_risk_and_funding(&spec);
}

#[test]
fn market_order_limit_uses_the_stricter_official_cap() {
    for (reported_market_max, expected_effective_max) in [
        (0, 12_000_000),
        (8_000_000, 8_000_000),
        (15_000_000, 12_000_000),
    ] {
        let mut value = serde_json::to_value(fixture_row()).expect("serialize fixture row");
        value["market_order_size_max"] = Value::from(reported_market_max);
        let row = serde_json::from_value(value).expect("market limit fixture row");

        let spec = GateContractSpec::from_row(&row, "usdt")
            .expect("valid Gate contract must not be discarded for divergent caps");

        assert_eq!(
            spec.market_max_qty, expected_effective_max as f64,
            "reported market max {reported_market_max}"
        );
        assert!(spec.has_complete_execution_metadata());
    }
}

fn assert_contract_identity_and_limits(spec: &GateContractSpec) {
    assert_eq!(
        spec.identity,
        GateContractIdentity {
            settle: "usdt".to_owned(),
            native_symbol: "BTC_USDT".to_owned(),
        }
    );
    assert_eq!(spec.asset_class, InstrumentAssetClass::Crypto);
    assert_eq!(spec.contract_size, 0.0001);
    assert_eq!(spec.price_tick, 0.1);
    assert_eq!(spec.qty_step, Some(1.0));
    assert_eq!(spec.min_qty, 1.0);
    assert_eq!(spec.max_qty, 12_000_000.0);
    assert_eq!(spec.market_max_qty, 8_000_000.0);
    assert_eq!(spec.listing_status, InstrumentListingStatus::Trading);
}

#[test]
fn official_zhipu_stock_contract_maps_to_equity() {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contract_zhipu_usdt.json");
    let row: GateContractMetadataRow =
        serde_json::from_str(fixture).expect("official Gate ZHIPU contract fixture must parse");

    let instrument = GateContractSpec::from_row(&row, "usdt")
        .expect("official Gate stock contract metadata must validate")
        .into_venue_instrument(1_785_556_491_379);

    assert_eq!(instrument.native_symbol, "ZHIPU_USDT");
    assert_eq!(instrument.canonical_symbol, "ZHIPU");
    assert_eq!(instrument.asset_class, InstrumentAssetClass::Equity);
    assert!(instrument.is_hedge_constructible());
}

#[test]
fn unknown_gate_contract_classification_remains_observation_only_identity() {
    let mut value = serde_json::to_value(fixture_row()).expect("serialize fixture row");
    value["contract_type"] = Value::from("commodities");
    let row = serde_json::from_value(value).expect("commodity fixture row");

    let instrument = GateContractSpec::from_row(&row, "usdt")
        .expect("unknown classification remains observable")
        .into_venue_instrument(1);

    assert_eq!(instrument.asset_class, InstrumentAssetClass::Unknown);
}

fn assert_contract_risk_and_funding(spec: &GateContractSpec) {
    assert_eq!(spec.leverage_min, 1.0);
    assert_eq!(spec.leverage_max, 200.0);
    assert_eq!(spec.maintenance_rate, 0.003);
    assert_eq!(spec.maker_fee_rate, -0.0001);
    assert_eq!(spec.taker_fee_rate, 0.00075);
    assert_eq!(spec.funding_interval_hours, 8);
    assert_eq!(spec.funding_next_apply_ms, Some(1_780_444_800_000));
}

#[test]
fn cached_funding_schedule_rolls_the_official_anchor_forward() {
    let spec = GateContractSpec::from_row(&fixture_row(), "usdt").expect("contract spec");
    let anchor_ms = spec
        .funding_next_apply_ms
        .expect("official fixture funding anchor");
    let cache = GateContractCache::default();
    cache.insert_spec(spec);
    let at_ms = anchor_ms + 8 * 3_600_000 + 1;

    let (interval_hours, next_apply_ms) = cache
        .funding_schedule("BTC", at_ms)
        .expect("cached schedule");

    assert_eq!(interval_hours, 8);
    assert_eq!(next_apply_ms, anchor_ms + 16 * 3_600_000);
}

#[test]
fn endpoint_settle_must_match_native_contract_quote() {
    let mut value = serde_json::to_value(fixture_row()).expect("serialize fixture row");
    value["name"] = Value::from("BTC_USDC");
    let row = serde_json::from_value(value).expect("mismatched fixture schema");

    let error = GateContractSpec::from_row(&row, "usdt").expect_err("settle mismatch");

    assert!(error.to_string().contains("does not match endpoint settle"));
}

#[test]
fn venue_instrument_uses_the_same_verified_native_identity() {
    let instrument = GateContractSpec::from_row(&fixture_row(), "usdt")
        .expect("contract spec")
        .into_venue_instrument(1_780_442_009_894);

    assert_eq!(instrument.native_symbol, "BTC_USDT");
    assert_eq!(instrument.canonical_symbol, "BTC");
    assert_eq!(instrument.quote_asset.as_deref(), Some("USDT"));
    assert_eq!(instrument.settle_asset.as_deref(), Some("USDT"));
    assert_eq!(instrument.contract_size, Some(0.0001));
    assert_eq!(instrument.price_tick, Some(0.1));
    assert_eq!(instrument.qty_step, Some(1.0));
    assert_eq!(instrument.min_qty, Some(1.0));
    assert_eq!(instrument.listing_status, InstrumentListingStatus::Trading);
    assert_eq!(
        instrument.source_url.as_deref(),
        Some("/api/v4/futures/usdt/contracts")
    );
    assert_eq!(
        instrument.schema_version.as_deref(),
        Some(CONTRACT_SCHEMA_VERSION)
    );
    assert!(instrument.is_hedge_constructible());
}

#[test]
fn decimal_contract_without_official_step_is_observation_only() {
    let mut value = serde_json::to_value(fixture_row()).expect("serialize fixture row");
    value["enable_decimal"] = Value::Bool(true);
    let row = serde_json::from_value(value).expect("decimal fixture row");
    let instrument = GateContractSpec::from_row(&row, "usdt")
        .expect("decimal contract metadata")
        .into_venue_instrument(1);

    assert_eq!(instrument.qty_step, None);
    assert!(instrument.is_observation_only());
}

#[test]
fn official_decimal_contract_fixture_preserves_fractional_minimum() {
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_rave_decimal.json");
    let rows: Vec<GateContractMetadataRow> =
        serde_json::from_str(fixture).expect("official Gate decimal contract fixture");
    let instrument = GateContractSpec::from_row(&rows[0], "usdt")
        .expect("decimal contract metadata")
        .into_venue_instrument(1);

    assert_eq!(instrument.native_symbol, "RAVE_USDT");
    assert_eq!(instrument.min_qty, Some(0.1));
    assert_eq!(instrument.qty_step, None);
    assert!(instrument.is_observation_only());
}

#[test]
fn decimal_contract_with_zero_minimum_is_retained_as_observation_only() {
    let mut value = serde_json::to_value(fixture_row()).expect("serialize fixture row");
    value["enable_decimal"] = Value::Bool(true);
    value["order_size_min"] = Value::from(0);
    let row = serde_json::from_value(value).expect("decimal fixture row");
    let instrument = GateContractSpec::from_row(&row, "usdt")
        .expect("official zero minimum is accepted without inventing a decimal step")
        .into_venue_instrument(1);

    assert_eq!(instrument.min_qty, None);
    assert_eq!(instrument.qty_step, None);
    assert!(instrument.is_observation_only());
}

#[test]
fn unicode_native_contract_is_retained_but_not_marked_executable() {
    let mut value = serde_json::to_value(fixture_row()).expect("serialize fixture row");
    value["name"] = Value::from("币安人生_USDT");
    let row = serde_json::from_value(value).expect("unicode fixture row");
    let instrument = GateContractSpec::from_row(&row, "usdt")
        .expect("official unicode identity is observable")
        .into_venue_instrument(1);

    assert_eq!(instrument.native_symbol, "币安人生_USDT");
    assert!(instrument.is_observation_only());
}

#[test]
fn invalid_limits_and_leverage_fail_closed() {
    for (field, value) in [
        ("order_size_max", Value::from(0)),
        ("market_order_size_max", Value::from(-1)),
        ("leverage_max", Value::from("0")),
        ("maintenance_rate", Value::from("bad")),
    ] {
        let mut row = serde_json::to_value(fixture_row()).expect("serialize fixture row");
        row[field] = value;
        let row = serde_json::from_value(row).expect("mutated fixture schema");
        let error = GateContractSpec::from_row(&row, "usdt").expect_err("invalid spec");
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

#[test]
fn status_mapping_is_strict_and_non_trading_is_not_constructible() {
    for (status, expected) in [
        ("prelaunch", InstrumentListingStatus::PreLaunch),
        ("delisting", InstrumentListingStatus::Delisted),
        ("delisted", InstrumentListingStatus::Delisted),
        ("circuit_breaker", InstrumentListingStatus::Suspended),
        ("future_status", InstrumentListingStatus::Unknown),
    ] {
        let mut row = serde_json::to_value(fixture_row()).expect("serialize fixture row");
        row["status"] = Value::from(status);
        let row = serde_json::from_value(row).expect("status fixture row");
        let instrument = GateContractSpec::from_row(&row, "usdt")
            .expect("known row schema")
            .into_venue_instrument(1);
        assert_eq!(instrument.listing_status, expected, "{status}");
        assert!(instrument.is_observation_only(), "{status}");
    }
}

#[test]
fn native_symbol_hint_preserves_explicit_gate_contracts() {
    assert_eq!(native_symbol_hint("btc_usdt").as_deref(), Some("BTC_USDT"));
    assert_eq!(native_symbol_hint("btc/usdc").as_deref(), Some("BTC_USDC"));
    assert_eq!(native_symbol_hint("btc-usd").as_deref(), Some("BTC_USD"));
    assert_eq!(native_symbol_hint("btc"), None);
    assert_eq!(native_symbol_hint("btc-usdt-swap"), None);
}

#[tokio::test]
async fn contract_list_request_binds_the_verified_settle_path() {
    let server = MockServer::start().await;
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let body: Value = serde_json::from_str(fixture).expect("official Gate fixture");
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts"))
        .and(header(
            crate::adapters::gate_public_rest::SIZE_DECIMAL_HEADER,
            crate::adapters::gate_public_rest::SIZE_DECIMAL_HEADER_VALUE,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    let http = crate::http::HttpClient::builder("gate")
        .timeout_secs(5)
        .build()
        .expect("Gate test HTTP client");

    let rows = fetch_contracts(&http, &server.uri(), "usdt")
        .await
        .expect("Gate contract list request");

    assert_eq!(rows.len(), 1, "official response row must not be skipped");
    let row = rows.into_iter().next().expect("one contract");
    let spec =
        GateContractSpec::from_row(&row, "usdt").expect("request row must produce a verified spec");
    assert_eq!(spec.identity.native_symbol, "BTC_USDT");
    assert_eq!(spec.identity.settle, "usdt");
}

#[tokio::test]
async fn concurrent_contract_refreshes_share_one_full_snapshot_request() {
    let server = MockServer::start().await;
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let body: Value = serde_json::from_str(fixture).expect("official Gate fixture");
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(50))
                .set_body_json(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    let http = crate::http::HttpClient::builder("gate")
        .timeout_secs(5)
        .build()
        .expect("Gate test HTTP client");
    let cache = std::sync::Arc::new(GateContractCache::default());
    let mut tasks = Vec::new();

    for _ in 0..16 {
        let cache = std::sync::Arc::clone(&cache);
        let http = http.clone();
        let base_url = server.uri();
        tasks.push(tokio::spawn(async move {
            cache.refresh_all(&http, &base_url).await
        }));
    }
    for task in tasks {
        task.await
            .expect("refresh task must join")
            .expect("shared refresh must succeed");
    }

    assert_eq!(
        cache.cached_native_symbol("BTC").as_deref(),
        Some("BTC_USDT")
    );
}

#[tokio::test]
async fn explicit_non_usdt_contract_fails_before_futures_request() {
    let server = MockServer::start().await;
    let http = crate::http::HttpClient::builder("gate")
        .timeout_secs(5)
        .build()
        .expect("Gate test HTTP client");
    let cache = GateContractCache::default();

    for symbol in ["BTC_USD", "BTC_USDC", "BTC/USD"] {
        let error = cache
            .order_unit(&http, &server.uri(), symbol)
            .await
            .expect_err("settle mismatch must fail closed");
        assert!(error.to_string().contains("cannot use futures/usdt"));
    }
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .is_empty(),
        "settle mismatch must not reach any Gate futures endpoint"
    );
}

#[tokio::test]
async fn canonical_fallback_is_not_executable_until_officially_verified() {
    let server = MockServer::start().await;
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let mut rows: Vec<Value> = serde_json::from_str(fixture).expect("official Gate fixture");
    let body = rows.pop().expect("one official contract row");
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts/BTC_USDT"))
        .and(header(
            crate::adapters::gate_public_rest::SIZE_DECIMAL_HEADER,
            crate::adapters::gate_public_rest::SIZE_DECIMAL_HEADER_VALUE,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    let http = crate::http::HttpClient::builder("gate")
        .timeout_secs(5)
        .build()
        .expect("Gate test HTTP client");
    let cache = GateContractCache::default();

    let unit = cache
        .order_unit(&http, &server.uri(), "BTC")
        .await
        .expect("official contract lookup verifies canonical fallback and size");

    assert_eq!(unit, 0.0001);
    assert_eq!(
        cache.cached_native_symbol("BTC").as_deref(),
        Some("BTC_USDT")
    );
}

#[tokio::test]
async fn single_contract_response_identity_mismatch_fails_closed() {
    let server = MockServer::start().await;
    let fixture = include_str!("../../fixtures/gate/futures_usdt_contracts_btc_usdt.json");
    let mut rows: Vec<Value> = serde_json::from_str(fixture).expect("official Gate fixture");
    let mut body = rows.pop().expect("one official contract row");
    body["name"] = Value::from("ETH_USDT");
    Mock::given(method("GET"))
        .and(path("/api/v4/futures/usdt/contracts/BTC_USDT"))
        .and(header(
            crate::adapters::gate_public_rest::SIZE_DECIMAL_HEADER,
            crate::adapters::gate_public_rest::SIZE_DECIMAL_HEADER_VALUE,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    let http = crate::http::HttpClient::builder("gate")
        .timeout_secs(5)
        .build()
        .expect("Gate test HTTP client");
    let cache = GateContractCache::default();

    let error = cache
        .verified_native_symbol(&http, &server.uri(), "BTC")
        .await
        .expect_err("mismatched native identity must fail closed");

    assert!(error.to_string().contains("identity mismatch"));
    assert_eq!(cache.get_unit("ETH_USDT"), None);
}
