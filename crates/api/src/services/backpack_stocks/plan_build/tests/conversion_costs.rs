use super::*;
use rust_decimal::Decimal;
use serde_json::json;

#[tokio::test]
async fn stock_conversion_cost_build_restart_conflict_and_release() {
    let temp = tempfile::tempdir().unwrap();
    let plans = temp.path().join("plans.jsonl");
    let costs = temp.path().join("conversion.jsonl");
    let (service, mut request, inputs, cost) = fixture(plans.clone());
    let service = Arc::new(
        Arc::try_unwrap(service)
            .ok()
            .unwrap()
            .with_exchange_conversion_store(costs.clone()),
    );
    let now = common::time::now_ms();
    let source = exchange_conversion::tests::completed_cost(&service, now);
    request.conversion_cost_ids = vec![source.plan_id.clone()];
    let ready = service.snapshot();
    let hub = realtime::WsHub::default();
    let snapshot = service
        .build_plan_with(
            request.clone(),
            &hub,
            |_, _, _| async { Ok(inputs) },
            |_, _| async { Ok(cost) },
        )
        .await
        .unwrap();
    let plan = &snapshot.plans[0];
    assert_eq!(plan.terms.conversion_costs, vec![source.clone()]);
    let raw = Decimal::from_str_exact(
        plan.terms.preflight_evidence.as_ref().unwrap().directions[0]
            .after_known_costs_usdc
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        Decimal::from_str_exact(&plan.terms.after_known_costs_usdc).unwrap(),
        raw - source.confirmed_fee_usdc().unwrap()
    );
    plan.validate_preflight_evidence().unwrap();
    assert_eq!(
        snapshot.claimed_conversion_cost_ids,
        request.conversion_cost_ids
    );
    let before = std::fs::read(&plans).unwrap();
    service
        .build_plan_with(request.clone(), &hub, no_inputs, no_cost)
        .await
        .unwrap();
    assert_eq!(before, std::fs::read(&plans).unwrap());
    service.with_plan_costs(plan, || Ok(())).unwrap();
    if let Ok(path) = std::env::var("STOCK_CONVERSION_COST_CAPTURE_PATH") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&json!({"ready":ready,"reserved":snapshot})).unwrap(),
        )
        .unwrap();
    }
    drop(service);
    let (service, _) = BackpackStocks::stock_plan_fixture(plans.clone(), common::time::now_ms());
    let service = Arc::new(service.with_exchange_conversion_store(costs));
    assert!(service.plan_store.problem().is_none());
    assert!(service.exchange_conversion_store.problem().is_none());
    let restored = service
        .build_plan_with(request.clone(), &hub, no_inputs, no_cost)
        .await
        .unwrap();
    assert_eq!(restored.plans[0], *plan);
    service.with_plan_costs(plan, || Ok(())).unwrap();
    exchange_conversion::tests::conflict_cost(&service, &source.plan_id, common::time::now_ms());
    let mut called = false;
    assert!(service
        .with_plan_costs(plan, || {
            called = true;
            Ok(())
        })
        .is_err());
    assert!(
        !called,
        "changed costs must stop before a signer, send or settlement"
    );
    assert!(service
        .begin_stock_order(&plan.plan_id, &plan.terms.account_fingerprint)
        .is_err());
    assert_eq!(before, std::fs::read(&plans).unwrap());
    service.cancel_plan(&plan.plan_id, &hub).unwrap();
    assert!(service.snapshot().claimed_conversion_cost_ids.is_empty());
}

#[test]
fn stock_conversion_cost_sources_budget_claims_and_legacy_checks() {
    let temp = tempfile::tempdir().unwrap();
    let now = 10_000;
    let (service, _) = BackpackStocks::stock_plan_fixture(temp.path().join("plans.jsonl"), now);
    let service = service.with_exchange_conversion_store(temp.path().join("costs.jsonl"));
    let source = exchange_conversion::tests::completed_cost(&service, now);
    let (mut snapshot, account, mut request) = plans::tests::fixture(now);
    request.build = Some(StockPlanBuildRequest {
        request_id: request.request_id.clone(),
        asset: request.asset.clone(),
        direction: request.direction,
        wallet_address: request.wallet_address.clone(),
        input_raw: snapshot.chain_costs[0].quote.input_raw.clone(),
        keyed: snapshot.comparison.as_ref().unwrap().keyed,
        conversion_cost_ids: vec![source.plan_id.clone()],
    });
    snapshot.exchange_conversions = vec![source.clone()];
    let plan = plans::prepare(request.clone(), &snapshot, &account, now).unwrap();
    for case in 0..7 {
        let mut bad = snapshot.clone();
        let p = &mut bad.exchange_conversions[0];
        match case {
            0 => p.order.as_mut().unwrap().fills[0].fee = None,
            1 => p.order.as_mut().unwrap().evidence_conflict = true,
            2 => p.order.as_mut().unwrap().phase = StockCexOrderPhase::Open,
            3 => p.terms.account_fingerprint = "different-account".into(),
            4 => p.updated_at_ms = now + 1,
            5 => {
                p.order.as_mut().unwrap().fills[0]
                    .fee
                    .as_mut()
                    .unwrap()
                    .asset = "SOL".into()
            }
            _ => {
                bad.preflight.as_mut().unwrap().directions[0].after_known_costs_usdc =
                    Some("0.001".into())
            }
        }
        assert!(
            plans::prepare(request.clone(), &bad, &account, now).is_err(),
            "case {case}"
        );
    }
    let mut duplicate = request.clone();
    duplicate
        .build
        .as_mut()
        .unwrap()
        .conversion_cost_ids
        .push(source.plan_id.clone());
    assert!(plans::prepare(duplicate, &snapshot, &account, now).is_err());
    let mut bound = plan.clone();
    bound.terms.conversion_costs.push(source.clone());
    assert!(bound.terms.conversion_fee_usdc().is_err());
    let mut alias = source.clone();
    alias.plan_id = "alias-same-remote-order".into();
    bound.terms.conversion_costs[1] = alias.clone();
    assert!(bound.terms.conversion_fee_usdc().is_err());
    for phase in [
        StockPlanPhase::Reserved,
        StockPlanPhase::SubmissionUnknown,
        StockPlanPhase::Settled,
    ] {
        bound.phase = phase;
        assert!(bound.claims_conversion_cost(&alias, now));
    }
    bound.phase = StockPlanPhase::Cancelled;
    assert!(!bound.claims_conversion_cost(&source, now));
    bound.phase = StockPlanPhase::Reserved;
    assert!(!bound.claims_conversion_cost(&source, bound.terms.reserved_until_ms));
    let old = plans::tests::fixture_plan(now);
    let raw = serde_json::to_string(&old).unwrap();
    assert!(!raw.contains("conversionCost"));
    let accounting = serde_json::to_string(&old.accounting()).unwrap();
    assert!(!accounting.contains("conversionFee") && !accounting.contains("afterConversion"));
    assert_eq!(
        old.plan_id,
        plan_store::plan_id(&old.request, &old.terms).unwrap()
    );
}
