use super::*;

#[test]
fn rejects_invalid_numeric_patch() {
    let patch = RiskConfigPatch {
        max_order_notional: Some(0.0),
        max_open_orders: None,
        max_hedge_imbalance_pct: None,
        allowed_exchanges: None,
        allowed_symbols: None,
        protected_positions: None,
        auto_profit_close: None,
    };

    let result = NormalizedRiskConfigPatch::new(patch);

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[test]
fn trims_and_deduplicates_lists() {
    let patch = RiskConfigPatch {
        max_order_notional: None,
        max_open_orders: None,
        max_hedge_imbalance_pct: None,
        allowed_exchanges: Some(vec![" binance ".into(), "binance".into()]),
        allowed_symbols: None,
        protected_positions: None,
        auto_profit_close: None,
    };

    let normalized = NormalizedRiskConfigPatch::new(patch);
    assert!(normalized.is_ok(), "{normalized:?}");
    let Ok(normalized) = normalized else { return };

    assert_eq!(
        normalized
            .allowed_exchanges
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["binance"]
    );
}

#[test]
fn lowercases_lists() -> Result<(), AppError> {
    let patch = RiskConfigPatch {
        max_order_notional: None,
        max_open_orders: None,
        max_hedge_imbalance_pct: None,
        allowed_exchanges: Some(vec!["OKX".into(), "Hyperliquid:XYZ".into()]),
        allowed_symbols: Some(vec!["MU".into()]),
        protected_positions: None,
        auto_profit_close: None,
    };

    let normalized = NormalizedRiskConfigPatch::new(patch)?;

    assert_eq!(
        normalized.allowed_exchanges.unwrap_or_default(),
        BTreeSet::from(["hyperliquid:xyz".to_owned(), "okx".to_owned()])
    );
    assert_eq!(
        normalized.allowed_symbols.unwrap_or_default(),
        BTreeSet::from(["mu".to_owned()])
    );
    Ok(())
}

#[test]
fn normalizes_and_preserves_protected_position_fingerprint() -> Result<(), AppError> {
    let patch = RiskConfigPatch {
        max_order_notional: None,
        max_open_orders: None,
        max_hedge_imbalance_pct: None,
        allowed_exchanges: None,
        allowed_symbols: None,
        protected_positions: Some(vec![protected_position()]),
        auto_profit_close: None,
    };

    let normalized = NormalizedRiskConfigPatch::new(patch)?;
    let positions = normalized.protected_positions.unwrap_or_default();

    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].venue, "binance");
    assert_eq!(positions[0].canonical_symbol, "btc");
    assert_eq!(positions[0].native_symbol, "btcusdt");
    assert_eq!(positions[0].side, "long");
    assert_eq!(positions[0].position_mode.as_deref(), Some("both"));
    Ok(())
}

#[test]
fn rejects_incomplete_protected_position_fingerprint() {
    let mut position = protected_position();
    position.opening_identity.clear();
    let patch = RiskConfigPatch {
        max_order_notional: None,
        max_open_orders: None,
        max_hedge_imbalance_pct: None,
        allowed_exchanges: None,
        allowed_symbols: None,
        protected_positions: Some(vec![position]),
        auto_profit_close: None,
    };

    let result = NormalizedRiskConfigPatch::new(patch);

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[test]
fn restores_every_persisted_risk_field() -> Result<(), AppError> {
    let original = RiskConfig {
        kill_switch_active: true,
        max_order_notional: 20.0,
        max_open_orders: 4,
        max_hedge_imbalance_pct: 0.01,
        liquidation_warn_pct: 12.0,
        liquidation_danger_pct: 6.0,
        allowed_exchanges: BTreeSet::from(["bitget".to_owned(), "hyperliquid".to_owned()]),
        allowed_symbols: BTreeSet::from(["hyper".to_owned()]),
        protected_positions: vec![normalized_protected_position()],
        auto_profit_close: shared_types::AutoProfitCloseConfig {
            enabled: true,
            min_net_profit_usd: 0.02,
            min_roi_bps: 10.0,
            exit_buffer_bps: 5.0,
            stop_loss_enabled: true,
            max_net_loss_usd: 0.25,
            max_loss_roi_bps: 200.0,
            liquidation_guard_enabled: true,
            liquidation_exit_distance_pct: 12.0,
            confirmation_samples: 3,
            cooldown_secs: 30,
        },
        ..RiskConfig::default()
    };
    let expected = snapshot(&original);

    let restored = snapshot(&restored_config(&expected)?);

    assert_eq!(restored, expected);
    Ok(())
}

#[test]
fn rejects_restored_liquidation_threshold_inversion() {
    let mut status = snapshot(&RiskConfig::default());
    status.liquidation_warn_pct = 5.0;
    status.liquidation_danger_pct = 10.0;

    assert!(matches!(
        restored_config(&status),
        Err(AppError::BadRequest(_))
    ));
}

fn protected_position() -> ProtectedPositionFingerprint {
    ProtectedPositionFingerprint {
        venue: " BINANCE ".to_owned(),
        canonical_symbol: " BTC ".to_owned(),
        native_symbol: " BTCUSDT ".to_owned(),
        side: " LONG ".to_owned(),
        quantity: 0.232,
        entry_price: 64_456.2,
        position_mode: Some(" BOTH ".to_owned()),
        opening_identity: "preexisting-binance-btc-long".to_owned(),
        source: "account_position_runtime".to_owned(),
        captured_at_ms: 42,
    }
}

fn normalized_protected_position() -> ProtectedPositionFingerprint {
    let mut position = protected_position();
    position.venue = "binance".to_owned();
    position.canonical_symbol = "btc".to_owned();
    position.native_symbol = "btcusdt".to_owned();
    position.side = "long".to_owned();
    position.position_mode = Some("both".to_owned());
    position
}
