use super::*;
use crate::services::onchain_cross_chain_run_store::accounting as projection;
use crate::services::onchain_execution_run_store::{OnchainExecutionRunStore, test_checkpoint};

pub(crate) fn run() -> Run {
    let mut run = projection::tests::fixture();
    run.build.swap_executions = run.legs.iter().filter_map(|l| l.swap_execution.clone()).collect();
    run.build.bridge_executions = run.legs.iter().filter_map(|l| l.bridge_execution.clone()).collect();
    let mut cost = approval_allocation::tests::cost();
    let token = run.build.legs[0].from_token.clone();
    cost.plan.token_address = token.clone();
    if let Tx::EvmCall {to, ..} = &mut cost.plan.transactions[0] { *to = token.clone(); }
    let hash = format!("0x{:064x}", 9999);
    cost.run.transaction_ids = vec![hash.clone()];
    cost.run.fee_receipts[0].basis.transaction_id = hash.clone();
    cost.run.fee_receipts[0].basis.assets[0].address = token;
    cost.run.fee_receipts[0].network_cost.as_mut().unwrap().transaction_id = hash;
    if let Tx::EvmCall {allowance_spender, ..} = &mut run.build.swap_executions[0].transaction {
        *allowance_spender = Some(cost.plan.spender.clone());
    }
    run.build.approval_costs = vec![cost];
    run.build.replenishment_costs = vec![replenishment_allocation::test_cost()];
    run
}

fn store(path: &std::path::Path) -> crate::services::onchain_execution_run_store::OnchainExecutionRunReplay {
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_execution_run_ledger_path = Some(path.to_string_lossy().into());
    OnchainExecutionRunStore::load(&config)
}

#[test]
fn cross_chain_costs_claims_share_one_journal_with_primary_execution_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let run = run();
    let registry = store(&path).store;
    let claim = claim(&run).unwrap();
    registry.claim_cross_chain_costs(claim.clone()).unwrap();
    let before = std::fs::read(&path).unwrap();
    registry.claim_cross_chain_costs(claim.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let replay = store(&path);
    replay.store.readiness().unwrap();
    assert!(replay.runs.is_empty(), "a fee claim is not a CEX execution");
    replay.store.claim_cross_chain_costs(claim.clone()).unwrap();
    assert!(replay.store.check_approval_available(&run.build.approval_costs).is_err());
    assert!(replay.store.check_replenishment_available(&run.build.replenishment_costs).is_err());
    let mut second = claim.clone(); second.run_id = "other-cycle".into();
    assert!(replay.store.claim_cross_chain_costs(second).is_err());
    let mut changed = claim.clone(); changed.replenishment_costs.clear();
    assert!(replay.store.claim_cross_chain_costs(changed).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut primary = test_checkpoint();
    approval_allocation::tests::bind(&mut primary, run.build.approval_costs[0].clone());
    assert!(replay.store.append_pending(&primary).is_err());
    assert_eq!(replay.store.approval_cost_owner(&claim.approval_costs[0].run.run_id), Some(format!("cross-chain:{}",run.run_id)));
}

#[test]
fn cross_chain_costs_primary_and_cross_chain_concurrency_has_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let registry = store(&path).store;
    let run = run();
    let mut primary = test_checkpoint();
    approval_allocation::tests::bind(&mut primary, run.build.approval_costs[0].clone());
    let claim = claim(&run).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let wins = std::thread::scope(|s| {
        let a = s.spawn(|| { barrier.wait(); registry.append_pending(&primary).is_ok() });
        let b = s.spawn(|| { barrier.wait(); registry.claim_cross_chain_costs(claim).is_ok() });
        usize::from(a.join().unwrap()) + usize::from(b.join().unwrap())
    });
    assert_eq!(wins,1);
    store(&path).store.readiness().unwrap();
}

#[test]
fn cross_chain_costs_actual_accounting_uses_selected_receipts_not_estimated_gas() {
    let mut run = run();
    projection::project(&mut run);
    let accounting = run.accounting.as_mut().unwrap();
    assert_eq!(accounting.status, shared_types::OnchainExecutionAccountingStatus::PendingValuation);
    assert_eq!(accounting.external_flows.len(),2);
    let now = common::time::now_ms();
    let rates = [("ETH",2000.0,2001.0),("USDC",0.9,0.91)].into_iter().map(|(asset,bid,ask)| {
        let mut rate = usd_valuation::fixture(asset,bid,now); rate.usd_ask = ask; rate
    }).collect();
    let value = projection::value(accounting,rates,now).unwrap();
    assert_eq!(value.net_usd_exact,"3.383937");
    accounting.usd_value = Some(value);
    accounting.status = shared_types::OnchainExecutionAccountingStatus::Valued;
    projection::project(&mut run);
    assert_eq!(run.accounting.as_ref().unwrap().status, shared_types::OnchainExecutionAccountingStatus::Valued);
    if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_COST_FIXTURE") {
        std::fs::write(path,serde_json::to_vec_pretty(&run).unwrap()).unwrap();
    }
    let mut wrong = run.clone();
    wrong.build.approval_costs[0].run.fee_receipts[0].network_cost.as_mut().unwrap().total_fee_exact = None;
    projection::project(&mut wrong);
    assert!(wrong.accounting.unwrap().usd_value.is_none());
    let cost = &mut run.build.approval_costs[0];
    let hash = run.legs[0].source_transaction_id.clone().unwrap();
    cost.run.transaction_ids[0] = hash.clone();
    cost.run.fee_receipts[0].basis.transaction_id = hash.clone();
    cost.run.fee_receipts[0].network_cost.as_mut().unwrap().transaction_id = hash;
    projection::project(&mut run);
    assert!(run.accounting.unwrap().problems.iter().any(|p| p.contains("禁止重复")));
}

#[test]
fn cross_chain_costs_build_deducts_cost_and_rejects_wrong_contract_and_weak_profit() {
    let run = run();
    let now = common::time::now_ms();
    let mut config = Config::default(); config.quote_token = "USDC".into(); config.quote_decimals = 6;
    config.cross_chain.stablecoin_risk_bps = 0; config.spread_alert.min_net_spread_bps = 0.0;
    let mut build = run.build.clone(); build.net_return_bps = Some(100.0); build.total_cost_bps = Some(10.0);
    build.quote_usd_valuation = Some(usd_valuation::fixture("USDC",0.8,now));
    build.valid_until_ms = now + 5000;
    let cost = total_usd(&build).unwrap();
    apply_build_costs(&config,&mut build,now).unwrap();
    let expected = 100.0 - (cost / 0.8 * 1_000_000.0).ceil() / 100_000_000.0 * 10_000.0;
    assert!((build.net_return_bps.unwrap() - expected).abs() < 0.000001);
    let mut unprofitable = run.build.clone(); unprofitable.net_return_bps = Some(0.001); unprofitable.total_cost_bps = Some(10.0);
    unprofitable.quote_usd_valuation = build.quote_usd_valuation.clone();
    apply_build_costs(&config,&mut unprofitable,now).unwrap();
    assert!(!unprofitable.submit_ready);
    assert!(!unprofitable.blockers.is_empty());
    build.approval_costs[0].plan.spender = format!("0x{:040x}", 88);
    assert!(flows(&build).is_err());
}
