use super::*;
use crate::state::load_state::LoadState;

#[test]
fn stock_funding_followup_displays_next_pause_and_actual_deposit_without_resubmit() {
    Owner::new().with(|| {
        let now = chrono::Utc::now().timestamp_millis();
        let (_, mut p) = super::super::funding::tests::fixture(now - 60_000);
        assert!(followup_status(&p, now).is_none());
        p.phase = StockFundingPlanPhase::Withdrawing;
        p.withdrawal = Some(StockFundingWithdrawal {
            evidence_conflict: None,
            client_id: "local".into(),
            submitted_at_ms: now - 10_000,
            query_count: 1,
            last_query_at_ms: Some(now - 1000),
            remote: None,
            receipt: None,
            problem: None,
        });
        assert!(followup_status(&p, now).unwrap().contains("下次约 4 秒"));
        p.followup = Some(StockFundingFollowup {
            attempts: 6,
            last_at_ms: now,
            next_at_ms: None,
            paused: true,
            problem: None,
        });
        assert!(followup_status(&p, now)
            .unwrap()
            .contains("自动核验已暂停 · 6/6"));
        p.phase = StockFundingPlanPhase::Received;
        assert!(followup_status(&p, now)
            .unwrap()
            .contains("扣账与费用仍待核清"));
        p.withdrawal.as_mut().unwrap().evidence_conflict = Some("原提现费用冲突".into());
        assert!(followup_status(&p, now).unwrap().contains("回执冲突需人工核对"));
        let data = StockData {
            peers: crate::panels::modules::stocks::data::PeerData::defaults(),
            market: RwSignal::new(LoadState::Ready(StockMarketSnapshot::default())),
            catalog: RwSignal::new(LoadState::Ready(StockCatalog {
                rows: vec![],
                observed_at_ms: now,
            })),
            search: RwSignal::new(String::new()),
            page: RwSignal::new(0),
            pending: RwSignal::new(false),
            notice: RwSignal::new(None),
            clock: RwSignal::new(now),
            watch: Callback::new(|_| {}),
            refresh: Callback::new(|_| {}),
            budget: RwSignal::new("10".into()),
            keyed: RwSignal::new(false),
            quote_pending: RwSignal::new(false),
            quote: Callback::new(|_| {}),
            monitor_pending: RwSignal::new(false),
            monitor: Callback::new(|_| {}),
            rfq: crate::panels::modules::stocks::data::RfqData::fixture(),
            preflight: crate::panels::modules::stocks::data::PreflightData::fixture(),
            alerts: crate::panels::modules::stocks::data::AlertData::defaults(),
        };
        p.withdrawal.as_mut().unwrap().receipt = Some(StockFundingReceipt {
            transaction_hash: "original-finalized-signature".into(),
            destination: p.terms.destination.clone(),
            mint: p.terms.token.contract_address.clone().unwrap(),
            decimals: 6,
            credited_raw: "10000000".into(),
            slot: 10,
            block_time_ms: now - 9000,
            network_fee_lamports: 5000,
            fee_payer: p.request.wallet_address.clone(),
            checked_at_ms: now - 5000,
        });
        data.market.set(LoadState::Ready(StockMarketSnapshot {
            funding_plans: vec![p],
            ..Default::default()
        }));
        let html = panel(data).to_html();
        assert!(html.contains("原提现回执冲突"));
        assert!(html.contains("重新核对原提现"));
        assert!(html.contains("10000000"));
        assert!(!html.contains("提交本次提现"));
        for (input, output) in [
            (
                "STOCK_FUNDING_FOLLOWUP_CAPTURE_PATH",
                "STOCK_FUNDING_FOLLOWUP_RENDER_PATH",
            ),
            (
                "STOCK_FUNDING_FOLLOWUP_DEPOSIT_CAPTURE_PATH",
                "STOCK_FUNDING_FOLLOWUP_DEPOSIT_RENDER_PATH",
            ),
            (
                "STOCK_FUNDING_RECONCILIATION_CAPTURE_PATH",
                "STOCK_FUNDING_RECONCILIATION_RENDER_PATH",
            ),
        ] {
            let Ok(path) = std::env::var(input) else {
                continue;
            };
            let snapshot: StockMarketSnapshot =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let deposited = snapshot.funding_plans[0].phase == StockFundingPlanPhase::Deposited;
            let conflict = snapshot.funding_plans[0].withdrawal.as_ref()
                .is_some_and(|w|w.evidence_conflict.is_some());
            data.market.set(LoadState::Ready(snapshot));
            let html = panel(data).to_html();
            assert!(html.contains(if deposited {
                "Backpack 已确认入账"
            } else {
                "自动核验已暂停"
            }));
            assert!(!html.contains("提交本次提现") && !html.contains("提交本次链上转账"));
            if conflict {
                assert!(html.contains("原提现回执冲突"));
                assert!(html.contains("重新核对原提现"));
                assert!(html.contains("10000000"));
                assert!(html.contains("尚未核清 · 未释放占用"));
            } else if deposited {
                assert!(!html.contains("核对原转账与 Backpack 入账"));
            } else {
                assert!(html.contains("查询原提现与到账"));
            }
            if let Ok(path) = std::env::var(output) {
                super::super::tests::write_stock_html(&path, &html);
            }
        }
    });
}
