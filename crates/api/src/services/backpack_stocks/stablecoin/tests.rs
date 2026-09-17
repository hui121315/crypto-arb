use super::*;

impl BackpackStocks {
    pub(crate) fn stock_stablecoin_fixture(
        path: std::path::PathBuf,
        now: i64,
    ) -> (Self, StockStablecoinPlanRequest) {
        let service = Self::new().unwrap().with_stablecoin_store(path);
        *service.snapshot.write() = super::super::comparison::tests::snapshot();
        let p = super::super::stablecoin_store::tests::fixture(now);
        let r = super::super::stablecoin_store::tests::request(&p);
        *service.stablecoin_preview.write() = Some((0, p));
        (service, r)
    }
}

fn request() -> StockStablecoinRequest {
    StockStablecoinRequest {
        asset: "MU.US".into(),
        wallet_address: bs58::encode([3u8; 32]).into_string(),
        input_usdt: "10".into(),
        target_usdc: "10".into(),
        keyed: false,
    }
}

fn evidence(
    r: &StockStablecoinRequest,
    now: i64,
) -> (
    StockWalletEvidence,
    StockDexQuote,
    Option<StockChainCost>,
    Vec<String>,
) {
    (
        StockWalletEvidence {
            owner: r.wallet_address.clone(),
            mint: STOCK_SOLANA_USDT.into(),
            stock_raw: Some("20000000".into()),
            usdc_raw: Some("0".into()),
            sol_lamports: Some("10000000".into()),
            checked_at_ms: now,
            problems: vec![],
        },
        StockDexQuote {
            input_mint: STOCK_SOLANA_USDT.into(),
            output_mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
            input_raw: "10000000".into(),
            output_raw: "9950000".into(),
            minimum_output_raw: "9900000".into(),
            router: "fixture".into(),
            fee_bps: Some(10),
            fee_mint: Some(STOCK_SOLANA_USDT.into()),
            requested_at_ms: now,
            received_at_ms: now,
            expires_at_ms: None,
        },
        None,
        vec![],
    )
}

#[tokio::test]
async fn stock_stablecoin_service_checks_selection_and_never_creates_fund_plans() {
    let service = BackpackStocks::new().unwrap();
    *service.snapshot.write() = super::super::comparison::tests::snapshot();
    let snapshot = service.snapshot();
    let preview = service
        .preview_stablecoin_with(request(), |r, n| {
            Box::pin(async move {
                assert_eq!(n, 10_000_000);
                Ok(evidence(r, common::time::now_ms()))
            })
        })
        .await
        .unwrap();
    assert_eq!(preview.minimum_usdc, "9.9");
    assert_eq!(preview.shortfall_usdc, "0.1");
    assert_eq!(service.snapshot(), snapshot);
    assert!(service.snapshot().plans.is_empty() && service.snapshot().funding_plans.is_empty());
    let mut invalid = request();
    invalid.asset = "AAPL.US".into();
    assert!(service
        .preview_stablecoin_with(invalid, |_, _| Box::pin(async {
            panic!("selection must fail before reads")
        }))
        .await
        .is_err());
    let stale = service
        .preview_stablecoin_with(request(), |r, _| {
            service.generation.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(evidence(r, common::time::now_ms())) })
        })
        .await;
    assert_eq!(stale.unwrap_err(), "股票已切换或停止，旧询价已丢弃");
    assert_eq!(service.snapshot(), snapshot);
}

#[test]
fn stock_stablecoin_save_rejects_changed_input_and_selection_without_requoting() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stablecoin.jsonl");
    let now = common::time::now_ms();
    let (service, r) = BackpackStocks::stock_stablecoin_fixture(path.clone(), now);
    let hub = realtime::WsHub::new(4);
    let mut wrong = r.clone();
    wrong.conversion.input_usdt = "11".into();
    assert!(service.build_stablecoin_plan(wrong, &hub).is_err());
    assert!(!path.exists());
    service.generation.fetch_add(1, Ordering::SeqCst);
    assert!(service.build_stablecoin_plan(r, &hub).is_err());
    assert!(!path.exists());
}

#[test]
fn stock_stablecoin_mint_rejects_rebased_or_wrong_assets() {
    let mut mint = StockMintEvidence {
        address: STOCK_SOLANA_USDT.into(),
        decimals: 6,
        ui_multiplier: "1".into(),
        slot: 1,
        chain_time_ms: 1000,
        checked_at_ms: 1000,
        next_change_at_ms: None,
        extensions: vec![],
    };
    assert!(validate_mint(&mint).is_ok());
    mint.ui_multiplier = "1.01".into();
    assert!(validate_mint(&mint).is_err());
    mint.ui_multiplier = "1".into();
    mint.decimals = 9;
    assert!(validate_mint(&mint).is_err());
}
