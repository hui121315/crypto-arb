use super::super::filters::{default_p0_strategy_kinds, opportunity_symbol_matches};
use super::*;

#[test]
fn parses_strategy_filter_list() {
    let kinds = OpportunityFilters::strategy_kinds(Some(
        "perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross",
    ));
    assert_eq!(
        kinds,
        vec![
            StrategyKind::PerpCross,
            StrategyKind::PerpPriceSpread,
            StrategyKind::SpotPerp,
            StrategyKind::CrossSpotPerp,
            StrategyKind::SpotCross,
        ]
    );
}

#[test]
fn unsupported_strategy_filter_returns_empty_filter() {
    let kinds = OpportunityFilters::strategy_kinds(Some("unknown_kind,triangular"));
    assert!(kinds.is_empty());
}

#[test]
fn unsupported_strategy_filter_is_custom_scope() {
    let filters = OpportunityFilters::parse(&OpportunitiesParams {
        limit: None,
        min_yield: None,
        strategy: Some("unknown_kind".into()),
        symbol: None,
        fresh: false,
        fast: true,
    });

    assert!(filters.strategy_kinds.is_empty());
    assert_eq!(filters.scope(), OpportunityEnvelopeScope::Custom);
}

#[test]
fn missing_strategy_filter_defaults_to_p0_scope() {
    let filters = OpportunityFilters::parse(&OpportunitiesParams {
        limit: Some(10),
        min_yield: None,
        strategy: None,
        symbol: None,
        fresh: false,
        fast: true,
    });

    assert_eq!(filters.strategy_kinds, default_p0_strategy_kinds());
    assert_eq!(filters.scope(), OpportunityEnvelopeScope::MainP0);
    assert!(filters
        .query_key(&OpportunitiesParams {
            limit: Some(10),
            min_yield: None,
            strategy: None,
            symbol: None,
            fresh: false,
            fast: true,
        })
        .contains("scope=main_p0"));
}

#[test]
fn legacy_opportunities_default_and_max_limit_are_backend_contracts() {
    let default_filters = OpportunityFilters::parse(&OpportunitiesParams {
        limit: None,
        min_yield: None,
        strategy: None,
        symbol: None,
        fresh: false,
        fast: true,
    });
    let clamped_filters = OpportunityFilters::parse(&OpportunitiesParams {
        limit: Some(opportunity::MAX_WIDE_LIMIT + 1),
        min_yield: None,
        strategy: None,
        symbol: None,
        fresh: false,
        fast: true,
    });

    assert_eq!(default_filters.limit, Some(opportunity::DEFAULT_WIDE_LIMIT));
    assert_eq!(clamped_filters.limit, Some(opportunity::MAX_WIDE_LIMIT));
}

#[test]
fn legacy_opportunities_always_exposes_compatibility_problem() {
    let problems = legacy_query_problems(opportunity::wide_limit(None));

    assert!(problems
        .iter()
        .any(|problem| problem.code == codes::OPPORTUNITY_LEGACY_WIDE_ENDPOINT));
}

#[test]
fn symbol_filter_accepts_exchange_contract_forms() {
    assert_eq!(OpportunityFilters::symbol(Some("mu")), Some("MU".into()));
    assert_eq!(
        OpportunityFilters::symbol(Some("MUUSDTM")),
        Some("MU".into())
    );
    assert_eq!(
        OpportunityFilters::symbol(Some("xyz:MU")),
        Some("MU".into())
    );
    assert_eq!(
        OpportunityFilters::symbol(Some("MU-USDT-SWAP")),
        Some("MU".into())
    );
}

#[test]
fn symbol_match_is_exact_after_normalization() {
    let mut opp = ArbitrageOpportunityDto {
        symbol: "MU".into(),
        ..test_opp()
    };
    assert!(opportunity_symbol_matches(&opp, "MU"));
    assert!(!opportunity_symbol_matches(&opp, "MUMU"));

    opp.symbol = "MU-USDT-SWAP".into();
    assert!(opportunity_symbol_matches(&opp, "MU"));
}

#[test]
fn strategy_counts_are_computed_before_response_limit() {
    let rows = vec![
        test_opp_with_kind(StrategyKind::PerpCross),
        test_opp_with_kind(StrategyKind::PerpCross),
        test_opp_with_kind(StrategyKind::SpotPerp),
    ];

    let counts = opportunity::counts(&rows);

    assert_eq!(
        counts.strategy_counts.get(&StrategyKind::PerpCross),
        Some(&2)
    );
    assert_eq!(
        counts.strategy_counts.get(&StrategyKind::SpotPerp),
        Some(&1)
    );
    assert_eq!(
        counts.strategy_counts.get(&StrategyKind::CrossSpotPerp),
        None
    );
    assert_eq!(counts.executable_count, 3);
    assert_eq!(
        counts
            .executable_strategy_counts
            .get(&StrategyKind::PerpCross),
        Some(&2)
    );
}

#[test]
fn list_request_meta_exposes_normalized_request_semantics() {
    let params = OpportunityListParams {
        page_size: Some(opportunity::MAX_LIST_PAGE_SIZE + 10),
        limit: None,
        cursor: None,
        sort_key: Some("settlement".into()),
        min_yield: Some(0.25),
        strategy: Some("perp_cross,spot_perp".into()),
        symbol: Some("MUUSDTM".into()),
        fresh: true,
        fast: false,
    };
    let filters = OpportunityFilters::parse_list(&params);
    let window = opportunity::OpportunityListWindow::from_bound_query(
        filters.limit,
        None,
        params.sort_key.as_deref(),
        "test-scope",
    );

    let meta = filters.list_request_meta(&params, window);

    assert!(!meta.fast);
    assert!(meta.fresh);
    assert_eq!(meta.filter.scope, OpportunityEnvelopeScope::MainP0);
    assert_eq!(
        meta.filter.strategy_kinds,
        vec![StrategyKind::PerpCross, StrategyKind::SpotPerp]
    );
    assert_eq!(meta.filter.symbol.as_deref(), Some("MU"));
    assert_eq!(meta.filter.min_yield, Some(0.25));
    assert_eq!(
        meta.sort_key,
        shared_types::OpportunityListSortKey::Settlement
    );
    assert_eq!(
        meta.requested_page_size,
        Some(opportunity::MAX_LIST_PAGE_SIZE + 10)
    );
    assert_eq!(meta.applied_page_size, opportunity::MAX_LIST_PAGE_SIZE);
    assert_eq!(meta.max_page_size, opportunity::MAX_LIST_PAGE_SIZE);
}

#[test]
fn product_list_requires_dual_fresh_ws_market_evidence() {
    let filters = OpportunityFilters::parse_list(&OpportunityListParams {
        page_size: None,
        limit: None,
        cursor: None,
        sort_key: None,
        min_yield: None,
        strategy: Some("perp_cross".into()),
        symbol: None,
        fresh: false,
        fast: true,
    });
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut opportunity = test_opp();
    make_ready(&mut opportunity);
    assert!(filters.matches_product_row(&opportunity, now_ms));

    assert!(opportunity.short_leg_market_evidence.is_some());
    if let Some(evidence) = opportunity.short_leg_market_evidence.as_mut() {
        evidence.health.source = shared_types::MarketDataSourceKind::RestBaseline;
    }
    assert!(!filters.matches_product_row(&opportunity, now_ms));
}

#[test]
fn product_list_hides_a_candidate_that_turns_negative_after_confirmation_costs() {
    let filters = OpportunityFilters::parse_list(&OpportunityListParams {
        page_size: None,
        limit: None,
        cursor: None,
        sort_key: None,
        min_yield: None,
        strategy: Some("spot_cross".into()),
        symbol: None,
        fresh: false,
        fast: true,
    });
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut opportunity = test_opp_with_kind(StrategyKind::SpotCross);
    assert!(filters.matches_product_row(&opportunity, now_ms));

    opportunity.net_single_yield = -0.000_01;
    if let Some(cost) = opportunity.execution_cost.as_mut() {
        cost.one_cycle.net_bps = -0.1;
        cost.one_cycle.covers_round_trip_cost = false;
    }

    assert!(!filters.matches_product_row(&opportunity, now_ms));
}

#[test]
fn product_list_requires_fresh_ws_quote_conversion_evidence() {
    let filters = OpportunityFilters::parse_list(&OpportunityListParams {
        page_size: None,
        limit: None,
        cursor: None,
        sort_key: None,
        min_yield: None,
        strategy: Some("cross_spot_perp".into()),
        symbol: None,
        fresh: false,
        fast: true,
    });
    let mut opportunity = test_opp_with_kind(StrategyKind::CrossSpotPerp);
    opportunity.quote_conversions = vec![shared_types::OpportunityQuoteConversion {
        from_quote: "USDC".into(),
        to_quote: "USDT".into(),
        rate: 0.999,
        venue: "b".into(),
        symbol: "USDCUSDT".into(),
        market_evidence: None,
    }];

    assert!(!filters.matches_product_row(&opportunity, chrono::Utc::now().timestamp_millis()));
    opportunity.quote_conversions[0].market_evidence = Some(market_evidence("b"));
    assert!(filters.matches_product_row(&opportunity, chrono::Utc::now().timestamp_millis()));
}
