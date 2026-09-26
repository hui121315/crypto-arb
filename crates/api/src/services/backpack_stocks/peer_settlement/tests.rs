use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner};

pub(in crate::services::backpack_stocks) fn verify_completed_inventory(
    original: &StockPeerPlan,
    path: &std::path::Path,
) {
    let temp = tempfile::tempdir().unwrap();
    let journal = temp.path().join("settlement.jsonl");
    std::fs::copy(path, &journal).unwrap();
    let s = BackpackStocks::new()
        .unwrap()
        .with_peer_plan_store(journal.clone());
    assert!(
        s.peer_plan_store.problem().is_none(),
        "{:?}",
        s.peer_plan_store.problem()
    );
    let now = common::time::now_ms();
    let p = s.peer_plan_store.get(&original.plan_id).unwrap();
    assert_eq!(p, *original);
    assert_eq!(p.peer_settlement_problem(now), None, "{:?}", p.accounting());
    let bytes = std::fs::read(&journal).unwrap();
    let req = StockPlanRevisionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
    };
    let hub = realtime::WsHub::default();
    let occupied = || {
        s.wallet_claims
            .check("solana", &p.request.wallet_address, i64::MAX)
            .is_err()
    };
    assert!(occupied());
    let other_owner = Owner::new(Module::Recovery, "fixture-after-settlement");
    let other_hold = Hold::wallet("solana", &p.request.wallet_address, None)
        .unwrap()
        .with_account("kraken_stocks", "configured-account")
        .unwrap();
    assert!(s
        .wallet_claims
        .commit(
            other_owner.clone(),
            Some(other_hold.clone()),
            now,
            || Ok(())
        )
        .is_err());
    for lock in [&*s.submission_lock, &s.preflight_lock, &s.quote_lock] {
        let _guard = lock.try_lock().unwrap();
        assert!(s.settle_peer_plan(req.clone(), &hub).is_err());
        assert!(occupied());
        assert_eq!(std::fs::read(&journal).unwrap(), bytes);
    }
    let mut stale = req.clone();
    stale.revision -= 1;
    assert!(s.settle_peer_plan(stale, &hub).is_err());
    for case in 0..5 {
        let mut bad = p.clone();
        match case {
            0 => bad.cex_order.as_mut().unwrap().fills[0].fees = None,
            1 => bad
                .cex_order
                .as_mut()
                .unwrap()
                .mark_conflict("fixture conflict"),
            2 => {
                bad.recoveries
                    .last_mut()
                    .unwrap()
                    .submission
                    .as_mut()
                    .unwrap()
                    .receipt = None
            }
            3 => {
                bad.inventory_orders
                    .last_mut()
                    .unwrap()
                    .order
                    .as_mut()
                    .unwrap()
                    .fills[0]
                    .fees = None
            }
            _ => bad.inventory_orders.clear(),
        }
        assert!(bad.peer_settlement_problem(now).is_some(), "case {case}");
    }
    // A failed durable write must keep both holds, even when receipts are complete.
    let failed_path = temp.path().join("failed.jsonl");
    std::fs::write(&failed_path, &bytes).unwrap();
    let failed = BackpackStocks::new()
        .unwrap()
        .with_peer_plan_store(failed_path.clone());
    std::fs::rename(&failed_path, temp.path().join("failed-original.jsonl")).unwrap();
    std::fs::create_dir(&failed_path).unwrap();
    assert!(failed.settle_peer_plan(req.clone(), &hub).is_err());
    assert!(failed
        .wallet_claims
        .check("solana", &p.request.wallet_address, i64::MAX)
        .is_err());
    assert_eq!(
        failed.snapshot().peer_plans[0].phase,
        StockPeerPlanPhase::SubmissionUnknown
    );

    let snapshot = s.settle_peer_plan(req.clone(), &hub).unwrap();
    let final_p = snapshot.peer_plans[0].clone();
    assert_eq!(final_p.phase, StockPeerPlanPhase::Settled);
    assert!(!final_p.holds_funds(i64::MAX));
    assert_eq!(
        final_p.accounting(),
        p.accounting(),
        "no fabricated conversion or profit"
    );
    assert!(!final_p.peer_conversion_available(now));
    assert!(!final_p.peer_inventory_available(now));
    assert!(!final_p.peer_recovery_available(now));
    assert!(!final_p.peer_native_available(now));
    assert!(!occupied());
    assert!(
        snapshot.peer_preflight.is_none()
            && snapshot.preflight.is_none()
            && snapshot.chain_costs.is_empty()
    );
    let settled_bytes = std::fs::read(&journal).unwrap();
    assert_eq!(
        settled_bytes.iter().filter(|&&b| b == b'\n').count(),
        bytes.iter().filter(|&&b| b == b'\n').count() + 1
    );
    s.wallet_claims
        .commit(other_owner, Some(other_hold), now, || Ok(()))
        .unwrap();
    s.snapshot
        .write()
        .chain_costs
        .push(p.terms.basis.chain_cost.clone());
    s.settle_peer_plan(req.clone(), &hub).unwrap();
    assert_eq!(
        s.snapshot().chain_costs.len(),
        1,
        "duplicate settle cannot erase a newer plan's evidence"
    );
    assert!(
        occupied(),
        "duplicate settle cannot release another module's hold"
    );
    assert_eq!(std::fs::read(&journal).unwrap(), settled_bytes);
    assert!(s
        .peer_plan_store
        .receipt(&p.plan_id, p.cex_order.as_ref().unwrap())
        .is_err());
    if let Ok(output) = std::env::var("STOCK_PEER_SETTLEMENT_CAPTURE_PATH") {
        std::fs::write(output, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
    }
    drop(s);
    let restored = BackpackStocks::new().unwrap().with_peer_plan_store(journal);
    assert!(
        restored.peer_plan_store.problem().is_none(),
        "{:?}",
        restored.peer_plan_store.problem()
    );
    assert_eq!(restored.peer_plan_store.get(&p.plan_id).unwrap(), final_p);
    assert!(restored
        .wallet_claims
        .check("solana", &p.request.wallet_address, i64::MAX)
        .is_ok());
    restored.settle_peer_plan(req, &hub).unwrap();
    // A tampered archive cannot release funds when replayed.
    let corrupt_path = temp.path().join("corrupt.jsonl");
    let mut entries = String::from_utf8(settled_bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    entries.last_mut().unwrap()["plan"]["settlement"]["accounting"]["cashTotals"]["USDC"] =
        serde_json::json!("999999");
    let corrupt = entries
        .into_iter()
        .map(|v| format!("{}\n", serde_json::to_string(&v).unwrap()))
        .collect::<String>();
    std::fs::write(&corrupt_path, corrupt).unwrap();
    let blocked = BackpackStocks::new()
        .unwrap()
        .with_peer_plan_store(corrupt_path);
    assert!(blocked.peer_plan_store.problem().is_some());
    assert!(blocked
        .wallet_claims
        .check("solana", &p.request.wallet_address, i64::MAX)
        .is_err());
}
