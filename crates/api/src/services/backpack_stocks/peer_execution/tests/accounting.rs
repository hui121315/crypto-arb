use super::*;
use rust_decimal::Decimal;

fn n(s: &str) -> Decimal {
    stock_exact_decimal(s).unwrap()
}

#[tokio::test]
async fn stock_peer_pair_accounting_preserves_native_cash_exposure_and_restart_holds() {
    for direction in [StockChainDirection::Buy, StockChainDirection::Sell] {
        for (reject, failed) in [(false, false), (true, false), (false, true), (true, true)] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("peer.jsonl");
            let (s, p, peer, chain) = fixture(&path, direction, reject, failed, false).await;
            let hub = realtime::WsHub::default();
            s.execute_peer_owned(request(&p), hub.clone(), chain.clone(), Arc::new(|| Ok(())))
                .await
                .unwrap();
            if !reject {
                let unknown = wait_receipt(&s, &p.plan_id, false).await.accounting();
                assert_eq!(unknown.cex_fee_quote, None);
                assert_eq!(unknown.net_stock_shares, None);
                assert!(!unknown.cash_totals.contains_key("USD"));
                assert!(unknown.recovery_target.is_none());
                peer.emit_fill(true);
                wait_receipt(&s, &p.plan_id, true).await;
            }
            s.recheck_peer_with(&p.plan_id, &hub, chain.as_ref())
                .await
                .unwrap();
            let final_p = s.peer_plan_store.get(&p.plan_id).unwrap();
            let a = final_p.accounting();
            assert_eq!(s.snapshot().peer_accounting, vec![a.clone()]);
            assert_eq!(a.source_revision, final_p.revision);
            assert_eq!(a.fee_budget_matched, Some(true));
            let cex = final_p
                .cex_order
                .as_ref()
                .unwrap()
                .cash_settlement()
                .unwrap();
            assert_eq!(a.cash_totals["USD"], cex.quote_change);
            let receipt = final_p
                .chain_submission
                .as_ref()
                .unwrap()
                .receipt
                .as_ref()
                .unwrap();
            let usdc = receipt
                .asset_changes
                .iter()
                .find(|v| v.mint == shared_types::stocks::comparison::SOLANA_USDC)
                .unwrap();
            assert_eq!(
                a.cash_totals["USDC"],
                stock_chain_quantity(&usdc.raw_change, 6).unwrap()
            );
            assert_eq!(
                a.wallet_sol_change.as_deref(),
                Some("0"),
                "sponsor fee is not a wallet debit"
            );
            assert_eq!(a.network_fee_sol.as_deref(), Some("0.000007"));
            let net = n(a.cex_stock_shares.as_deref().unwrap())
                + n(a.chain_stock_shares.as_deref().unwrap());
            assert_eq!(n(a.net_stock_shares.as_deref().unwrap()), net);
            if reject != failed {
                assert_eq!(a.status, StockAccountingStatus::NeedsReview, "{a:?}");
                let t = a.recovery_target.as_ref().unwrap();
                assert_eq!(
                    t.direction,
                    if net < Decimal::ZERO {
                        StockChainDirection::Buy
                    } else {
                        StockChainDirection::Sell
                    }
                );
                let equivalent =
                    n(
                        &stock_chain_quantity(&t.stock_raw, p.terms.basis.chain_cost.mint.decimals)
                            .unwrap(),
                    ) * n(&p.terms.basis.chain_cost.mint.ui_multiplier);
                assert_eq!(equivalent, net.abs());
            } else {
                assert_eq!(a.status, StockAccountingStatus::LegsReconciled, "{a:?}");
                assert!(a.recovery_target.is_none());
                assert!(net >= Decimal::ZERO);
            }
            assert!(a.cash_totals.keys().all(|s| s == "USD" || s == "USDC"));
            assert!(final_p.holds_funds(i64::MAX));
            if direction == StockChainDirection::Sell && !reject {
                if let Ok(file) = std::env::var(if failed {
                    "STOCK_PEER_ACCOUNTING_RECOVERY_CAPTURE_PATH"
                } else {
                    "STOCK_PEER_ACCOUNTING_CAPTURE_PATH"
                }) {
                    std::fs::write(file, serde_json::to_vec_pretty(&s.snapshot()).unwrap())
                        .unwrap();
                }
                if !failed {
                    check_missing_and_conflicting_evidence(&final_p);
                    check_failed_wallet_fee(&final_p);
                }
            }
            drop(s);
            let restored = BackpackStocks::new().unwrap().with_peer_plan_store(path);
            assert!(restored.peer_plan_store.problem().is_none());
            assert_eq!(restored.snapshot().peer_accounting, vec![a]);
            assert!(restored
                .peer_plan_store
                .get(&p.plan_id)
                .unwrap()
                .holds_funds(i64::MAX));
            assert_eq!(peer.sends.load(Ordering::SeqCst), 1);
            assert_eq!(chain.signs.load(Ordering::SeqCst), 1);
            assert_eq!(chain.sends.load(Ordering::SeqCst), 1);
        }
    }
}

fn check_missing_and_conflicting_evidence(p: &StockPeerPlan) {
    for case in 0..11 {
        let mut p = p.clone();
        match case {
            0 => p.cex_order.as_mut().unwrap().fills[0].fees = None,
            1 => p.chain_submission.as_mut().unwrap().receipt = None,
            2 => p
                .chain_submission
                .as_mut()
                .unwrap()
                .receipt
                .as_mut()
                .unwrap()
                .asset_changes
                .retain(|a| a.mint != p.terms.basis.chain_cost.mint.address),
            3 => {
                p.chain_submission
                    .as_mut()
                    .unwrap()
                    .receipt
                    .as_mut()
                    .unwrap()
                    .asset_changes[0]
                    .decimals = 8
            }
            4 => {
                let r = p
                    .chain_submission
                    .as_mut()
                    .unwrap()
                    .receipt
                    .as_mut()
                    .unwrap();
                r.asset_changes.push(r.asset_changes[0].clone());
            }
            5 => {
                p.chain_submission
                    .as_mut()
                    .unwrap()
                    .receipt
                    .as_mut()
                    .unwrap()
                    .transaction_id = "wrong-original".into()
            }
            6 => {
                p.chain_submission
                    .as_mut()
                    .unwrap()
                    .receipt
                    .as_mut()
                    .unwrap()
                    .slot = 1
            }
            7 => {
                p.cex_order.as_mut().unwrap().fills[0]
                    .fees
                    .as_mut()
                    .unwrap()[0]
                    .asset = "EUR".into()
            }
            8 => {
                p.cex_order.as_mut().unwrap().fills[0]
                    .fees
                    .as_mut()
                    .unwrap()[0]
                    .quantity = "1".into()
            }
            9 => p.terms.basis.peer.share_unit_verified = false,
            10 => p
                .chain_submission
                .as_mut()
                .unwrap()
                .receipt
                .as_mut()
                .unwrap()
                .asset_changes
                .push(StockChainAssetChange {
                    mint: "unexpected-asset".into(),
                    decimals: 6,
                    raw_change: "1".into(),
                }),
            _ => unreachable!(),
        }
        let a = p.accounting();
        assert_ne!(
            a.status,
            StockAccountingStatus::LegsReconciled,
            "case {case}: {a:?}"
        );
        assert!(a.recovery_target.is_none(), "case {case}");
        if case < 2 {
            assert_eq!(a.status, StockAccountingStatus::AwaitingReceipts);
            assert!(a.net_stock_shares.is_none());
        }
        if case == 8 {
            assert_eq!(a.fee_budget_matched, Some(false));
            assert_eq!(a.cex_fee_quote.as_deref(), Some("1"));
            assert_eq!(
                a.cash_totals["USD"], "-13.02",
                "actual fee is not replaced by budget"
            );
        }
    }
    let mut zero = p.clone();
    zero.cex_order.as_mut().unwrap().fills[0]
        .fees
        .as_mut()
        .unwrap()[0]
        .quantity = "0".into();
    assert_eq!(zero.accounting().cex_fee_quote.as_deref(), Some("0"));
    let mut same_quote = zero.clone();
    same_quote.request.selection.native_symbol = "MUx/USDC".into();
    same_quote.terms.draft.request.selection = same_quote.request.selection.clone();
    same_quote.terms.draft.quote_asset = "USDC".into();
    same_quote.terms.basis.account.quote_asset = "USDC".into();
    same_quote.terms.basis.account.native_symbol = "MUx/USDC".into();
    let r = same_quote.cex_order.as_mut().unwrap();
    r.draft = same_quote.terms.draft.clone();
    r.fills[0].fees.as_mut().unwrap()[0].asset = "USDC".into();
    let a = same_quote.accounting();
    assert_eq!(a.cash_totals.len(), 1);
    assert_eq!(
        a.cash_totals["USDC"], "1.98",
        "only the identical native currency can be summed"
    );
    assert_eq!(a.status, StockAccountingStatus::LegsReconciled);
}

fn check_failed_wallet_fee(p: &StockPeerPlan) {
    let mut p = p.clone();
    let c = &mut p.terms.basis.chain_cost;
    chain::tests::attach(c, false);
    let mut row = chain::intent(c, &chain::tests::signed(c).unwrap(), p.updated_at_ms).unwrap();
    let lookup = chain::tests::parsed_finalized(c, &row, true).unwrap();
    row.transaction_id = Some(lookup.receipt.as_ref().unwrap().transaction_id.clone());
    row.receipt = lookup.receipt;
    p.chain_submission = Some(row);
    let a = p.accounting();
    assert_eq!(a.wallet_sol_change.as_deref(), Some("-0.000007"));
    assert_eq!(a.network_fee_sol.as_deref(), Some("0.000007"));
    assert_eq!(
        a.cash_totals["USDC"], "0",
        "a failed swap rolls token movements back"
    );
    assert_eq!(a.net_stock_shares.as_deref(), Some("0.02"));
    assert_eq!(
        a.recovery_target.unwrap().direction,
        StockChainDirection::Sell
    );
    assert_eq!(
        a.movements.iter().filter(|a| a.asset == "SOL").count(),
        1,
        "network fee must not be deducted twice"
    );
    p.chain_submission
        .as_mut()
        .unwrap()
        .receipt
        .as_mut()
        .unwrap()
        .wallet_native_change_lamports = "-14000".into();
    assert!(
        p.accounting().recovery_target.is_none(),
        "invalid failed balance delta cannot authorize recovery"
    );
}
