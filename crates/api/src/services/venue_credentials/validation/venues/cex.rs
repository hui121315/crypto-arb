use super::*;

pub(in super::super) async fn validate_binance(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let adapter = Binance::new(BinanceConfig {
        credentials: Some(BinanceCredentials {
            api_key: values.get("api_key")?,
            api_secret: values.get("api_secret")?,
        }),
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    let account_mode = optional_account_mode_probe(
        "usds_m_futures",
        "binance.GET /fapi/v1/positionSide/dual",
        adapter.get_exchange_account_mode("binance"),
    )
    .await;
    let order_permission = optional_safe_order_place_cancel_test_probe(
        "binance_usdm_futures_order_test_cancel_no_match",
        "binance.POST /fapi/v1/order/test + DELETE /fapi/v1/order",
        adapter.validate_safe_order_place_cancel_test_permission(),
    )
    .await;
    validated_private_read_evidence(
        values.spec.venue,
        &adapter,
        "USDT",
        vec![account_mode, order_permission],
        &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
    )
    .await
}

pub(in super::super) async fn validate_okx(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let profile = required_okx_profile(values)?;
    let adapter = Okx::new(OkxConfig {
        credentials: Some(OkxCredentials {
            api_key: profile.api_key.clone(),
            api_secret: profile.api_secret.clone(),
            passphrase: profile.passphrase.clone(),
        }),
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    let account_mode = optional_account_mode_probe(
        "exchange_account",
        "okx.GET /api/v5/account/config",
        adapter.get_exchange_account_mode("okx"),
    )
    .await;
    let order_permission = optional_safe_order_pre_check_probe(
        "okx_order_precheck",
        "okx.POST /api/v5/trade/order-precheck",
        adapter.validate_safe_order_pre_check_permission(),
    )
    .await;
    validated_private_read_evidence(
        values.spec.venue,
        &adapter,
        "USDT",
        vec![account_mode, order_permission],
        &[VenueCredentialPermission::PlaceOrder],
    )
    .await
}

pub(in super::super) async fn validate_bybit(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let adapter = Bybit::new(BybitConfig {
        credentials: Some(BybitCredentials {
            api_key: values.get("api_key")?,
            api_secret: values.get("api_secret")?,
        }),
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    let account_mode = optional_account_mode_probe(
        "bybit_account",
        "bybit.GET /v5/account/info",
        adapter.credential_account_mode_info(),
    );
    let order_permission = optional_order_permission_probe(
        "bybit_linear_trade_read_write",
        "bybit.GET /v5/user/query-api",
        adapter.validate_api_order_permission_status(),
    );
    let (account_mode, order_permission) = tokio::join!(account_mode, order_permission);
    validated_private_read_evidence(
        values.spec.venue,
        &adapter,
        "USDT",
        vec![account_mode, order_permission],
        &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
    )
    .await
}

pub(in super::super) async fn validate_bitget(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let adapter = Bitget::new(BitgetConfig {
        credentials: Some(BitgetCredentials {
            api_key: values.get("api_key")?,
            api_secret: values.get("api_secret")?,
            passphrase: values.get("passphrase")?,
        }),
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    let account_mode = optional_account_mode_probe(
        "uta",
        "bitget.GET /api/v3/account/settings",
        adapter.get_exchange_account_mode("bitget"),
    )
    .await;
    let order_permission = optional_order_permission_probe(
        "bitget_uta_trade_read_write",
        "bitget.GET /api/v3/account/info",
        adapter.validate_api_order_permission_status(),
    )
    .await;
    validated_private_read_evidence(
        values.spec.venue,
        &adapter,
        "USDT",
        vec![account_mode, order_permission],
        &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
    )
    .await
}

pub(in super::super) async fn validate_gate(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let adapter = Gate::new(GateConfig {
        credentials: Some(GateCredentials {
            api_key: values.get("api_key")?,
            api_secret: values.get("api_secret")?,
        }),
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    let account_mode = optional_account_mode_probe(
        "usdt_futures",
        "gate.GET /api/v4/futures/usdt/accounts",
        adapter.get_exchange_account_mode("gate"),
    )
    .await;
    let order_permission = optional_safe_order_cancel_no_match_probe(
        "gate_futures_cancel_no_match",
        "gate.DELETE /api/v4/futures/usdt/orders/{order_id}",
        adapter.validate_safe_order_cancel_no_match_permission(),
    )
    .await;
    validated_private_read_evidence(
        values.spec.venue,
        &adapter,
        "USDT",
        vec![account_mode, order_permission],
        &[VenueCredentialPermission::CancelOrder],
    )
    .await
}

pub(in super::super) async fn validate_gate_crossex(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let adapter = GateCrossEx::new(GateCrossExConfig {
        credentials: Some(GateCrossExCredentials {
            api_key: values.get("api_key")?,
            api_secret: values.get("api_secret")?,
        }),
        allow_live_writes: false,
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    validate_exchange_request(adapter.get_account_read(None), VALIDATION_TIMEOUT_SECS).await?;
    let account_mode = optional_account_mode_probe(
        "crossex_unified",
        "gate_crossex.GET /api/v4/crossex/accounts",
        adapter.get_exchange_account_mode("gate_crossex"),
    );
    let positions = optional_exchange_probe(
        "positions_read",
        "private_read.positions",
        "gate_crossex private WS + bounded REST bootstrap",
        "positions",
        adapter.get_positions(None),
    );
    let open_orders = optional_exchange_probe(
        "open_orders_read",
        "private_read.open_orders",
        "gate_crossex private WS + bounded REST bootstrap",
        "open orders",
        adapter.get_open_orders(None),
    );
    let (account_mode, positions, open_orders) = tokio::join!(account_mode, positions, open_orders);
    Ok(read_only_evidence_with_order_permission_scopes(
        vec![
            balance_probe("CrossEx account"),
            account_mode,
            positions,
            open_orders,
            order_permission_unproven_probe("gate_crossex"),
        ],
        &[],
    ))
}

pub(in super::super) async fn validate_kraken(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let spot_key = values.optional("spot_api_key");
    let spot_secret = values.optional("spot_api_secret");
    let futures_key = values.optional("futures_api_key");
    let futures_secret = values.optional("futures_api_secret");
    let complete_spot = match (spot_key, spot_secret) {
        (Some(api_key), Some(api_secret)) => Some(KrakenSpotCredentials {
            api_key,
            api_secret,
        }),
        (None, None) => None,
        _ => {
            return Err(CredentialUpdateError::Validation(
                "Kraken Spot API Key and Secret must be configured together".to_owned(),
            ))
        }
    };
    let complete_futures = match (futures_key, futures_secret) {
        (Some(api_key), Some(api_secret)) => Some(KrakenFuturesCredentials {
            api_key,
            api_secret,
        }),
        (None, None) => None,
        _ => {
            return Err(CredentialUpdateError::Validation(
                "Kraken Futures API Key and Secret must be configured together".to_owned(),
            ))
        }
    };
    if complete_spot.is_none() && complete_futures.is_none() {
        return Err(CredentialUpdateError::Validation(
            "configure at least one complete Kraken Spot or Futures credential pair".to_owned(),
        ));
    }
    let has_futures = complete_futures.is_some();
    let adapter = Kraken::new(KrakenConfig {
        credentials: Some(KrakenCredentials {
            spot: complete_spot,
            futures: complete_futures,
        }),
        allow_live_writes: false,
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    validate_exchange_request(adapter.get_account_read(None), VALIDATION_TIMEOUT_SECS).await?;
    let open_orders = optional_exchange_probe(
        "open_orders_read",
        "private_read.open_orders",
        "kraken Spot executions / Futures open_orders WS with bounded REST bootstrap",
        "open orders",
        adapter.get_open_orders(None),
    )
    .await;
    let positions = if has_futures {
        optional_exchange_probe(
            "positions_read",
            "private_read.positions",
            "kraken Futures open_positions WS with bounded REST bootstrap",
            "positions",
            adapter.get_positions(None),
        )
        .await
    } else {
        probe(
            "positions_read",
            VenueCredentialProbeStatus::Unknown,
            "private_read.positions",
            "kraken Futures credentials not configured",
            "positions were not probed because only Kraken Spot credentials are configured",
        )
    };
    Ok(read_only_evidence_with_order_permission_scopes(
        vec![
            balance_probe("Kraken configured accounts"),
            positions,
            open_orders,
            order_permission_unproven_probe("kraken"),
        ],
        &[],
    ))
}

pub(in super::super) async fn validate_kucoin(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let adapter = Kucoin::new(KucoinConfig {
        credentials: Some(KucoinCredentials {
            api_key: values.get("api_key")?,
            api_secret: values.get("api_secret")?,
            passphrase: values.get("passphrase")?,
        }),
        timeout_secs: HTTP_TIMEOUT_SECS,
        ..Default::default()
    })
    .map_err(|error| validation_error(&error))?;
    let account_mode = optional_account_mode_probe(
        "classic_futures",
        "kucoin.GET /api/v2/position/getPositionMode",
        adapter.get_exchange_account_mode("kucoin"),
    )
    .await;
    let order_permission = optional_safe_order_place_cancel_test_probe(
        "kucoin_classic_futures_order_test_cancel_no_match",
        "kucoin.POST /api/v1/orders/test + DELETE /api/v1/orders/client-order/{clientOid}",
        adapter.validate_safe_order_place_cancel_test_permission(),
    )
    .await;
    let fee_rate = optional_exchange_probe(
        "fee_rate_read",
        "private_read.fee_rate:XBTUSDTM",
        "kucoin.GET /api/v1/trade-fees",
        "actual futures fee rate",
        adapter.actual_fee_rates("XBTUSDTM"),
    )
    .await;
    validated_private_read_evidence(
        values.spec.venue,
        &adapter,
        "USDT",
        vec![account_mode, order_permission, fee_rate],
        &[
            VenueCredentialPermission::PlaceOrder,
            VenueCredentialPermission::CancelOrder,
        ],
    )
    .await
}
