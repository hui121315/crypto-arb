use super::*;
use std::collections::BTreeMap;

fn fixture() -> (
    StockMarketSnapshot,
    StockAccountEvidence,
    StockWalletEvidence,
) {
    let mut s = super::super::comparison::tests::fixture();
    s.token_metadata_at_ms = Some(10_000);
    let c = s.comparison.as_mut().unwrap();
    c.mint.checked_at_ms = 10_000;
    c.mint.next_change_at_ms = None;
    c.mint.ui_multiplier = "1.25".into();
    let token = |address: &str, decimals, min: &str, fee: &str| StockChainToken {
        blockchain: "Solana".into(),
        contract_address: Some(address.into()),
        native_decimals: Some(decimals),
        deposit_enabled: Some(true),
        withdraw_enabled: Some(true),
        minimum_deposit: Some(min.into()),
        minimum_withdrawal: Some(min.into()),
        maximum_withdrawal: None,
        withdrawal_fee: Some(fee.into()),
    };
    s.tokens = vec![token(&c.mint.address, c.mint.decimals, "0.0006", "0.0006")];
    s.funding_assets = vec![
        StockFundingAsset {
            asset: "USDC".into(),
            tokens: vec![token(comparison::SOLANA_USDC, 6, "0.5", "0.5")],
        },
        StockFundingAsset {
            asset: "SOL".into(),
            tokens: vec![token("So1", 9, "0.012", "0.006")],
        },
    ];
    let balance = |n: &str| StockAccountBalance {
        available: n.into(),
        locked: "0".into(),
        staked: "0".into(),
        observed_at_ms: 10_000,
        source_at_us: None,
    };
    let a = StockAccountEvidence {
        fingerprint: "local".into(),
        spot_maker_fee_bps: "0".into(),
        spot_taker_fee_bps: "10".into(),
        liquidating: false,
        fees_at_ms: 10_000,
        balances_at_ms: 10_000,
        balances: BTreeMap::from([
            (c.asset.clone(), balance("2")),
            ("USDC".into(), balance("10")),
            ("SOL".into(), balance("1")),
            ("USDT".into(), balance("50")),
        ]),
    };
    let w = StockWalletEvidence {
        owner: "local-wallet".into(),
        mint: c.mint.address.clone(),
        stock_raw: Some("0".into()),
        usdc_raw: Some("12000000".into()),
        sol_lamports: Some("100000000".into()),
        checked_at_ms: 10_000,
        problems: vec![],
    };
    (s, a, w)
}

fn row(direction: StockChainDirection, needs: &[(&str, &str, &str)]) -> StockPreflightDirection {
    StockPreflightDirection {
        direction: direction.label().into(),
        gross_usdc: None,
        cex_fee_usdc: None,
        native_fee_usdc: None,
        after_known_costs_usdc: None,
        fee_basis: String::new(),
        transfer_problem: None,
        blockers: vec![],
        executable: false,
        inventory: needs
            .iter()
            .map(|(location, asset, q)| StockInventoryRequirement {
                location: (*location).into(),
                asset: (*asset).into(),
                required: Some((*q).into()),
                available: None,
                sufficient: None,
            })
            .collect(),
    }
}

#[test]
fn stock_funding_preserves_trade_reserves_and_cannot_fund_a_cycle_from_future_proceeds() {
    let (s, a, w) = fixture();
    let asset = &s.comparison.as_ref().unwrap().asset;
    let rows = [row(
        StockChainDirection::Sell,
        &[
            ("Backpack", "USDC", "20"),
            ("Solana", asset, "0.02"),
            ("Solana", "USDC / SOL 补仓", "5"),
        ],
    )];
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    let cash = result[0].needs.iter().find(|n| n.asset == "USDC").unwrap();
    assert_eq!(cash.shortfall.as_deref(), Some("10"));
    assert_eq!(cash.source_spare.as_deref(), Some("7"));
    assert_eq!(cash.source_sufficient, Some(false));
    let stock = result[0].needs.iter().find(|n| n.asset == *asset).unwrap();
    assert_eq!(stock.conservative_source_budget.as_deref(), Some("0.0212"));
    assert_eq!(stock.source_sufficient, Some(true));
    assert!(stock.blockers.iter().any(|b| b.contains("未生成提币请求")));
}

#[test]
fn stock_funding_rounds_stock_and_usdc_deposits_up_to_native_units_and_checks_only_needed_direction(
) {
    let (mut s, mut a, mut w) = fixture();
    let asset = s.comparison.as_ref().unwrap().asset.clone();
    a.balances.get_mut(&asset).unwrap().available = "0".into();
    a.balances.get_mut("USDC").unwrap().available = "0".into();
    w.stock_raw = Some("1000000".into());
    s.tokens[0].minimum_deposit = Some("0.00000001".into());
    s.tokens[0].withdraw_enabled = Some(false);
    let rows = [row(
        StockChainDirection::Buy,
        &[
            ("Backpack", &asset, "0.0000011"),
            ("Backpack", "USDC", "0.50000001"),
        ],
    )];
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    assert_eq!(
        result[0].needs[0].conservative_source_budget.as_deref(),
        Some("0.00000125")
    );
    assert_eq!(
        result[0].needs[1].conservative_source_budget.as_deref(),
        Some("0.500001")
    );
    assert!(result[0].needs[0].source_sufficient.unwrap());
    assert!(!result[0].needs[0]
        .blockers
        .iter()
        .any(|b| b.contains("未开放")));
    s.tokens[0].deposit_enabled = Some(false);
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    assert!(result[0].needs[0]
        .blockers
        .iter()
        .any(|b| b.contains("充值未开放")));
    s.tokens[0].deposit_enabled = Some(true);
    s.comparison.as_mut().unwrap().mint.next_change_at_ms = Some(10_100);
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    assert_eq!(result[0].needs[0].conservative_source_budget, None);
    assert_eq!(result[0].needs[0].source_spare, None);
    assert_eq!(
        result[0].needs[1].conservative_source_budget.as_deref(),
        Some("0.500001")
    );
}

#[test]
fn stock_funding_unknown_stale_wrong_contract_and_fee_are_not_zero_or_transfer_permission() {
    let (s, a, w) = fixture();
    let rows = [row(StockChainDirection::Buy, &[("Solana", "USDC", "20")])];
    for change in 0..7 {
        let mut s = s.clone();
        let mut a = a.clone();
        let mut w = w.clone();
        match change {
            0 => w.usdc_raw = None,
            1 => s.token_metadata_at_ms = Some(1),
            2 => s.funding_assets[0].tokens[0].contract_address = Some("wrong-usdc".into()),
            3 => s.funding_assets[0].tokens[0].withdrawal_fee = None,
            4 => a.balances.remove("USDC").map(|_| ()).unwrap(),
            5 => a.balances.get_mut("USDC").unwrap().observed_at_ms = 1,
            _ => s.funding_assets[0].tokens[0].maximum_withdrawal = Some("1".into()),
        }
        let now = if change == 1 || change == 5 {
            40_100
        } else {
            10_100
        };
        let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), now);
        let n = &result[0].needs[0];
        if change == 0 {
            assert_eq!(n.shortfall, None);
        }
        if change == 1 || change == 2 {
            assert!(n.token.is_none());
        }
        if change == 3 {
            assert_eq!(n.conservative_source_budget, None);
        }
        if change == 4 {
            assert!(n.blockers.iter().any(|b| b.contains("USDT→USDC")));
        }
        if change == 5 {
            assert_eq!(n.source_available, None);
        }
        if change == 6 {
            assert!(n.blockers.iter().any(|b| b.contains("单笔上限")));
        }
        assert!(!n.blockers.is_empty());
    }
}

#[test]
fn stock_funding_zero_is_known_and_sufficient_inventory_does_not_create_unnecessary_transfers() {
    let (s, a, mut w) = fixture();
    let rows = [row(
        StockChainDirection::Buy,
        &[("Backpack", "USDC", "10"), ("Solana", "USDC", "12")],
    )];
    assert!(evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100)[0]
        .needs
        .is_empty());
    w.usdc_raw = Some("0".into());
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    assert_eq!(result[0].needs[0].shortfall.as_deref(), Some("12"));
    assert_eq!(result[0].needs[0].source_spare.as_deref(), Some("0"));
}

#[test]
fn stock_funding_conversion_targets_cover_source_gap_without_spending_trade_reserves() {
    let (s, a, mut w) = fixture();
    w.usdc_raw = Some("0".into());
    let rows = [row(
        StockChainDirection::Buy,
        &[("Backpack", "USDC", "8"), ("Solana", "USDC", "20")],
    )];
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    let mut need = result[0].needs[0].clone();
    assert_eq!(need.source_spare.as_deref(), Some("2"));
    assert_eq!(need.conservative_source_budget.as_deref(), Some("21"));
    assert_eq!(need.usdc_conversion_target("Solana").as_deref(), Some("20"));
    assert_eq!(
        need.usdc_conversion_target("Backpack").as_deref(),
        Some("19")
    );
    need.source_available = None;
    assert!(need.usdc_conversion_target("Backpack").is_none());
    need.source_available = Some("29".into());
    need.source_spare = Some("21".into());
    assert!(need.usdc_conversion_target("Backpack").is_none());
    need.source = "Solana".into();
    need.target = "Backpack".into();
    need.shortfall = Some("0.0000001".into());
    assert_eq!(
        need.usdc_conversion_target("Backpack").as_deref(),
        Some("0.000001")
    );
    need.shortfall = None;
    assert!(need.usdc_conversion_target("Backpack").is_none());
    need.shortfall = Some("10".into());
    need.asset = "MU.US".into();
    assert!(need.usdc_conversion_target("Backpack").is_none());
}

#[test]
fn stock_funding_combines_trade_and_native_topup_before_comparing_one_balance() {
    let (s, a, mut w) = fixture();
    w.usdc_raw = Some("10000000".into());
    let mut rows = [row(
        StockChainDirection::Buy,
        &[("Solana", "USDC", "8"), ("Solana", "USDC / SOL 补仓", "3")],
    )];
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    assert_eq!(result[0].needs.len(), 1);
    let need = &result[0].needs[0];
    assert_eq!(need.required.as_deref(), Some("11"));
    assert_eq!(need.shortfall.as_deref(), Some("1"));
    assert_eq!(need.conservative_source_budget.as_deref(), Some("2"));
    assert_eq!(need.source_sufficient, Some(true));
    rows[0].inventory[1].required = None;
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    let need = &result[0].needs[0];
    assert!(need.required.is_none() && need.shortfall.is_none());
    assert!(need.usdc_conversion_target("Solana").is_none());
}

#[test]
fn stock_funding_source_conversion_also_restores_its_own_reserve_deficit() {
    let (s, mut a, mut w) = fixture();
    a.balances.get_mut("USDC").unwrap().available = "5".into();
    w.usdc_raw = Some("0".into());
    let rows = [row(
        StockChainDirection::Buy,
        &[
            ("Backpack", "USDC", "6"),
            ("Backpack", "USDC / SOL 补仓", "2"),
            ("Solana", "USDC", "20"),
        ],
    )];
    let result = evaluate_funding(&s, &rows, Some(&a), Some(&w), 10_100);
    let mut need = result[0]
        .needs
        .iter()
        .find(|n| n.target == "Solana")
        .unwrap()
        .clone();
    assert_eq!(need.source_trade_reserve.as_deref(), Some("8"));
    assert_eq!(need.source_spare.as_deref(), Some("0"));
    assert_eq!(need.conservative_source_budget.as_deref(), Some("21"));
    assert_eq!(
        need.usdc_conversion_target("Backpack").as_deref(),
        Some("24")
    );
    need.source_trade_reserve = None;
    assert!(need.usdc_conversion_target("Backpack").is_none());
    let old = serde_json::to_value(&need).unwrap();
    assert!(
        old.get("sourceTradeReserve").is_none(),
        "preserve old stored plan bytes"
    );
    let restored: StockFundingNeed = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), old);
}
