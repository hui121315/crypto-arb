use super::*;
use crate::services::onchain_wallet_claims::WalletClaims;

pub(super) async fn roundtrip(
    service: Arc<BackpackStocks>,
    plan: &StockExecutionPlan,
    hub: &realtime::WsHub,
) {
    let now = common::time::now_ms();
    let (mut funding, mut snapshot, mut account, mut wallet) = funding_plan::tests::inputs(now);
    snapshot.plans = vec![plan.clone()];
    snapshot.security = Some(plan.terms.security.clone());
    let old_mint = plan.terms.chain_cost.mint.clone();
    let mint = &mut snapshot.comparison.as_mut().unwrap().mint;
    *mint = old_mint;
    mint.checked_at_ms = now;
    let sell = plan.request.direction == StockChainDirection::Sell;
    if sell {
        // Explicit mock fee: the public metadata fixture leaves this unknown.
        snapshot.tokens[0].withdrawal_fee = Some("0.0006".into());
        snapshot.tokens[0].minimum_withdrawal = Some("0.001".into());
        wallet.stock_raw = Some("0".into());
        wallet.usdc_raw = Some("25000000".into());
    }
    wallet.owner = plan.request.wallet_address.clone();
    wallet.mint = mint.address.clone();
    account.fingerprint = plan.terms.account_fingerprint.clone();
    let request = StockPreflightRequest {
        asset: plan.request.asset.clone(),
        wallet_address: Some(wallet.owner.clone()),
        source_plan: Some(plan.inventory_source()),
    };
    *service.snapshot.write() = snapshot;
    service.account.write().evidence = Some(account.clone());
    for case in 0..3 {
        let mut bad = request.clone();
        match case {
            0 => bad.source_plan.as_mut().unwrap().revision += 1,
            1 => bad.wallet_address = Some(bs58::encode([99; 32]).into_string()),
            _ => bad.asset = "OTHER.US".into(),
        }
        assert!(service
            .preflight_with(bad, hub.clone(), |_, _, _| async {
                panic!("invalid source must fail before account reads")
            })
            .await
            .is_err());
    }
    let input_wallet = wallet.clone();
    let fingerprint = account.fingerprint.clone();
    let ready = service
        .preflight_with(request.clone(), hub.clone(), move |_, r, _| async move {
            assert!(r.source_plan.is_some());
            Ok(preflight::Inputs {
                fingerprint: Some(fingerprint),
                wallet: Some(input_wallet),
                problems: vec![],
            })
        })
        .await
        .unwrap();
    let report = ready.preflight.as_ref().unwrap();
    assert_eq!(report.source_plan, Some(plan.inventory_source()));
    assert_eq!(report.directions.len(), 1);
    assert!(!report.current(&ready, now));
    assert!(!report.directions[0].executable);
    assert!(report.directions[0].after_known_costs_usdc.is_none());
    assert!(report.directions[0]
        .inventory
        .iter()
        .any(|i| i.sufficient == Some(false)));
    for case in 0..5 {
        let mut s = ready.clone();
        let mut a = account.clone();
        let mut w = wallet.clone();
        match case {
            0 => s.comparison.as_mut().unwrap().mint.ui_multiplier = "2".into(),
            1 => a.fingerprint = "other".into(),
            2 => w.checked_at_ms = now - 30_001,
            3 => s.comparison.as_mut().unwrap().mint.next_change_at_ms = Some(now),
            _ => w.owner = bs58::encode([88; 32]).into_string(),
        }
        assert!(plan.restock_report(&s, &a, &w, now).is_err(), "case {case}");
    }
    let mut unknown = wallet.clone();
    unknown.usdc_raw = None;
    unknown.stock_raw = None;
    let uncertain = plan
        .restock_report(&ready, &account, &unknown, now)
        .unwrap();
    assert!(uncertain.funding[0]
        .needs
        .iter()
        .any(|n| n.shortfall.is_none()));

    funding.source_plan = Some(plan.inventory_source());
    funding.direction = plan.request.direction;
    funding.funding_asset = if sell {
        plan.request.asset.clone()
    } else {
        "USDC".into()
    };
    funding.wallet_address = wallet.owner.clone();
    funding.preflight_at_ms = report.checked_at_ms;
    funding.request_id = format!("restock-{}", plan.plan_id);
    let new_wallet = wallet.clone();
    let fingerprint = account.fingerprint.clone();
    let reserved = service
        .build_funding_plan_with(funding.clone(), hub, move |_, r, _| async move {
            assert!(r.source_plan.is_some());
            Ok(preflight::Inputs {
                fingerprint: Some(fingerprint),
                wallet: Some(new_wallet),
                problems: vec![],
            })
        })
        .await
        .unwrap();
    let saved = reserved
        .funding_plans
        .iter()
        .find(|p| p.request == funding)
        .unwrap()
        .clone();
    service
        .build_funding_plan_with(funding.clone(), hub, |_, _, _| async {
            panic!("same restock request must not re-read or re-reserve")
        })
        .await
        .unwrap();
    assert_eq!(saved.terms.source_plan.as_deref(), Some(plan));
    service
        .cancel_funding_plan(
            StockPlanRevisionRequest {
                plan_id: saved.plan_id.clone(),
                revision: saved.revision,
            },
            hub,
        )
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let now = saved.terms.created_at_ms;
    let path = tmp.path().join("restock.jsonl");
    let claims = Arc::new(WalletClaims::default());
    let store = funding_store::FundingStore::load(Some(path.clone()), claims.clone());
    store.insert(saved.clone(), now).unwrap();
    assert!(claims
        .check("solana", &wallet.owner, now)
        .is_err());
    let original = std::fs::read(&path).unwrap();
    assert!(store
        .previous(&funding, &account.fingerprint)
        .unwrap()
        .is_some());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    drop(store);
    let claims = Arc::new(WalletClaims::default());
    let store = funding_store::FundingStore::load(Some(path), claims.clone());
    assert!(store.problem().is_none());
    let mut bad = saved.clone();
    bad.terms.source_plan.as_mut().unwrap().revision += 1;
    bad.plan_id = funding_plan::plan_id(&bad.request, &bad.terms).unwrap();
    assert!(funding_plan::validate(&bad).is_err());
    store
        .cancel(
            &StockPlanRevisionRequest {
                plan_id: saved.plan_id.clone(),
                revision: saved.revision,
            },
            now + 1,
        )
        .unwrap();
    assert!(claims
        .check("solana", &wallet.owner, now + 1)
        .is_ok());
    if !sell {
        if let Ok(path) = std::env::var("STOCK_RESTOCK_CAPTURE_PATH") {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path);
            let mut before = ready.clone();
            before.preflight = None;
            let mut reserved = ready.clone();
            reserved.funding_plans = vec![saved];
            std::fs::write(
                path,
                serde_json::to_vec_pretty(
                    &json!({"before":before,"ready":ready,"reserved":reserved}),
                )
                .unwrap(),
            )
            .unwrap();
        }
    }
}
