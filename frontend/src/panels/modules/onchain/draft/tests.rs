use super::*;

#[test]
fn runtime_snapshot_cannot_overwrite_an_unsaved_cex_market_or_amount() {
    Owner::new().with(|| {
        let config = OnchainComparisonConfig {
            cex_symbol: "ETH/USDC".to_owned(),
            base_mint: "resolved-base".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("SOL/USD".to_owned());
        draft.quote_amount.set("250".to_owned());

        assert!(!draft.matches_applied_config(&config));
        assert_eq!(draft.symbol.get_untracked(), "SOL/USD");
        assert_eq!(draft.quote_amount.get_untracked(), "250");
    });
}

#[test]
fn resolved_identity_requires_explicit_cex_pair_and_updates_raw_units() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("SOL/USD".to_owned());
        draft.apply_base_identity(&OnchainTokenIdentity {
            chain: "base".to_owned(),
            address: "0xbase".to_owned(),
            symbol: "WETH".to_owned(),
            name: Some("Wrapped Ether".to_owned()),
            decimals: 18,
            source: "contract".to_owned(),
            evidence_url: String::new(),
            verified: false,
            native: false,
            observed_at_ms: 1,
        });

        assert_eq!(draft.base_token.get_untracked(), "WETH");
        assert!(!draft.base_identity_resolved.get_untracked());
        assert_eq!(draft.symbol.get_untracked(), "SOL/USD");
        assert_eq!(draft.base_amount.get_untracked(), "1000000000000000000");
    });
}

#[test]
fn runtime_snapshot_does_not_overwrite_an_unsaved_contract_address() {
    Owner::new().with(|| {
        let config = OnchainComparisonConfig::default();
        let draft = OnchainConfigDraft::from_config(&config);
        draft.base_mint.set("new-contract".to_owned());

        assert!(!draft.matches_applied_config(&config));
    });
}

#[test]
fn percentage_inputs_round_trip_to_internal_basis_points() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());

        assert_eq!(draft.cex_fee.get_untracked(), "0.1");
        assert_eq!(draft.alert_threshold.get_untracked(), "0.2");
        assert_eq!(draft.alert_raw_threshold.get_untracked(), "0.2");
        draft.cex_fee.set("0.075".to_owned());
        draft.alert_threshold.set("0.35".to_owned());
        draft.alert_raw_threshold.set("0.5".to_owned());
        draft.alert_mode.set(OnchainSpreadAlertMode::RawObservation);
        let patch = draft.patch();

        assert_eq!(patch.cex_taker_fee_bps, Some(7.5));
        assert_eq!(
            patch
                .spread_alert
                .as_ref()
                .and_then(|alert| alert.min_net_spread_bps),
            Some(35.0)
        );
        let alert = patch.spread_alert.expect("spread alert patch");
        assert_eq!(alert.mode, Some(OnchainSpreadAlertMode::RawObservation));
        assert_eq!(alert.min_raw_spread_bps, Some(50.0));
    });
}

#[test]
fn quote_amount_uses_human_units_and_patches_exact_raw_units() {
    Owner::new().with(|| {
        let config = OnchainComparisonConfig {
            quote_decimals: 6,
            quote_amount_raw: "1250000".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        let draft = OnchainConfigDraft::from_config(&config);

        assert_eq!(draft.quote_amount.get_untracked(), "1.25");
        assert_eq!(draft.patch().quote_amount_raw.as_deref(), Some("1250000"));

        draft.quote_amount.set("0.000001".to_owned());
        assert_eq!(draft.patch().quote_amount_raw.as_deref(), Some("1"));
    });
}

#[test]
fn clearing_chain_identity_preserves_the_explicit_cex_pair() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("SOL/USD".to_owned());

        draft.clear_base_identity();

        assert!(draft.base_mint.get_untracked().is_empty());
        assert!(draft.base_token.get_untracked().is_empty());
        assert_eq!(draft.symbol.get_untracked(), "SOL/USD");
    });
}

#[test]
fn editing_contract_clears_stale_metadata_before_auto_resolution() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("SOL/USD".to_owned());
        draft.invalidate_base_identity();

        assert!(draft.base_token.get_untracked().is_empty());
        assert!(draft.base_decimals.get_untracked().is_empty());
        assert!(draft.base_amount.get_untracked().is_empty());
        assert_eq!(draft.symbol.get_untracked(), "SOL/USD");

        draft.invalidate_quote_identity();
        assert!(draft.quote_token.get_untracked().is_empty());
        assert!(draft.quote_decimals.get_untracked().is_empty());
        assert!(draft.quote_amount.get_untracked().is_empty());
    });
}

#[test]
fn changing_chain_preserves_the_explicit_cex_pair() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("SOL/USD".to_owned());

        draft.apply_chain_preset("base");

        assert_eq!(draft.chain.get_untracked(), "base");
        assert_eq!(draft.symbol.get_untracked(), "SOL/USD");
    });
}

#[test]
fn precision_only_resolution_updates_units_and_marks_identity_unresolved() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.invalidate_base_identity();
        draft.symbol.set("PUPS/USD".to_owned());
        draft.apply_base_precision("2oGLxYuNBJRcepT1mEV6KnETaLD7Bf6qq3CM6skasBfe", 8);
        assert_eq!(draft.base_token.get_untracked(), "2oGLxYuN..kasBfe");
        assert!(!draft.base_identity_resolved.get_untracked());
        assert_eq!(draft.base_decimals.get_untracked(), "8");
        assert_eq!(draft.base_amount.get_untracked(), "100000000");

        draft.invalidate_quote_identity();
        draft.apply_quote_precision("quote-mint", 6);
        assert_eq!(draft.quote_token.get_untracked(), "quote-mint");
        assert!(!draft.quote_identity_resolved.get_untracked());
        assert_eq!(draft.quote_decimals.get_untracked(), "6");
        assert_eq!(draft.quote_amount.get_untracked(), "100");
    });
}

#[test]
fn explicit_cex_pair_updates_only_the_provisional_base_alias() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.base_token.set("old".to_owned());
        draft.quote_token.set("quote".to_owned());
        draft.base_identity_resolved.set(false);
        draft.quote_identity_resolved.set(false);

        draft.apply_cex_symbol("wif/usdt");

        assert_eq!(draft.symbol.get_untracked(), "WIF/USDT");
        assert_eq!(draft.base_token.get_untracked(), "WIF");
        assert_eq!(draft.quote_token.get_untracked(), "quote");
        assert!(!draft.base_identity_resolved.get_untracked());
        assert!(!draft.quote_identity_resolved.get_untracked());
    });
}

#[test]
fn changing_venue_removes_stale_cex_pair() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("ETH/USDC".to_owned());

        draft.apply_venue("bitget");

        assert_eq!(draft.venue.get_untracked(), "bitget");
        assert!(draft.symbol.get_untracked().is_empty());
    });
}

#[test]
fn changing_chain_quote_preserves_the_explicit_cex_pair() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.symbol.set("SOL/USDT".to_owned());

        draft.apply_quote_identity(&OnchainTokenIdentity {
            chain: "solana".to_owned(),
            address: "quote-address".to_owned(),
            symbol: "USDC".to_owned(),
            name: Some("USD Coin".to_owned()),
            decimals: 6,
            source: "contract".to_owned(),
            evidence_url: String::new(),
            verified: true,
            native: false,
            observed_at_ms: 1,
        });

        assert_eq!(draft.symbol.get_untracked(), "SOL/USDT");
    });
}

#[test]
fn wallet_address_is_patched_and_unsaved_input_survives_runtime_sync() {
    Owner::new().with(|| {
        let config = OnchainComparisonConfig {
            wallet_address: "saved-wallet".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        let draft = OnchainConfigDraft::from_config(&config);
        draft.wallet_address.set("unsaved-wallet".to_owned());

        draft.sync_runtime_identity(&config);

        assert_eq!(draft.wallet_address.get_untracked(), "unsaved-wallet");
        assert_eq!(
            draft.patch().wallet_address.as_deref(),
            Some("unsaved-wallet")
        );
    });
}

#[test]
fn applied_config_match_detects_an_external_provider_change() {
    Owner::new().with(|| {
        let config = OnchainComparisonConfig::default();
        let draft = OnchainConfigDraft::from_config(&config);
        let matches = Memo::new(move |_| draft.matches_applied_config(&config));

        assert!(matches.get_untracked());

        draft.provider.set("jupiter_swap_v2_keyed".to_owned());
        draft.pool_or_route.set("jupiter-api-key".to_owned());

        assert!(!matches.get_untracked());
    });
}

#[test]
fn alert_match_detects_an_unapplied_rule_change() {
    Owner::new().with(|| {
        let config = OnchainComparisonConfig::default();
        let draft = OnchainConfigDraft::from_config(&config);
        let alert_config = config.clone();
        let full_config = config.clone();
        let alert_matches = Memo::new(move |_| draft.alert_matches_applied_config(&alert_config));
        let full_matches = Memo::new(move |_| draft.matches_applied_config(&full_config));

        assert!(alert_matches.get_untracked());

        draft.alert_enabled.set(!config.spread_alert.enabled);

        assert!(!alert_matches.get_untracked());
        assert!(!full_matches.get_untracked());
    });
}

#[test]
fn dex_cross_draft_only_enables_an_independent_provider_on_the_same_chain() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());
        draft.apply_chain_preset("base");
        draft.set_dex_compare_enabled(true);

        assert!(draft.dex_compare_enabled.get_untracked());
        assert_eq!(draft.provider.get_untracked(), "zeroex_swap_v2");
        assert_eq!(draft.peer_provider.get_untracked(), "okx_dex_v6");
        let patch = draft.patch().dex_comparison.expect("DEX comparison patch");
        assert_eq!(patch.enabled, Some(true));
        assert_eq!(patch.peer_provider.as_deref(), Some("okx_dex_v6"));

        draft.apply_chain_preset("solana");
        draft.set_dex_compare_enabled(true);
        assert!(!draft.dex_compare_enabled.get_untracked());
        assert!(draft.peer_provider.get_untracked().is_empty());
    });
}

#[test]
fn cross_chain_draft_requires_an_explicit_peer_watch_item() {
    Owner::new().with(|| {
        let draft = OnchainConfigDraft::from_config(&OnchainComparisonConfig::default());

        draft.set_cross_chain_enabled(true);
        assert!(!draft.cross_chain_enabled.get_untracked());

        draft.apply_cross_chain_peer("watch-arbitrum-pups");
        draft.set_cross_chain_enabled(true);
        let patch = draft.patch().cross_chain.expect("cross-chain patch");

        assert!(draft.cross_chain_enabled.get_untracked());
        assert_eq!(patch.enabled, Some(true));
        assert_eq!(patch.peer_item_id.as_deref(), Some("watch-arbitrum-pups"));
        assert_eq!(patch.provider.as_deref(), Some("lifi"));
    });
}
