use super::*;
use serde_json::json;

fn data() -> OnchainCrossChainData {
    OnchainCrossChainData {
        selected_approvals: RwSignal::new(Vec::new()),
        selected_replenishments: RwSignal::new(Vec::new()),
        build: RwSignal::new(None),
        building: RwSignal::new(false),
        build_preview: Callback::new(|_| {}),
        confirmation: RwSignal::new(String::new()),
        authorizing: RwSignal::new(false),
        authorize: Callback::new(|_| {}),
        recovery: RwSignal::new(Default::default()),
        refreshing: RwSignal::new(false),
        refresh: Callback::new(|_| {}),
        submitting: RwSignal::new(false),
        submit_next_leg: Callback::new(|_| {}),
        rechecking: RwSignal::new(false),
        recheck: Callback::new(|_| {}),
        recovery_preview: RwSignal::new(None),
        recovery_preview_request: RwSignal::new(None),
        recovery_previewing: RwSignal::new(false),
        preview_recovery: Callback::new(|_| {}),
        recovery_mutating: RwSignal::new(false),
        reserve_recovery: Callback::new(|_| {}),
        cancel_recovery: Callback::new(|_| {}),
    }
}

fn fixture() -> OnchainCrossChainRun {
    let mut run: OnchainCrossChainRun = serde_json::from_value(json!({
        "runId":"cross-chain-render-fixture", "idempotencyKey":"render-key", "status":"paused",
        "authorization":{"actor":"test", "authorizedAtMs":100, "validUntilMs":1000,"confirmationVersion":"v1"},
        "activePosition":2,"createdAtMs":100,"updatedAtMs":500,"nextAction":"目标链到账尚未确认，核对后再继续。",
        "problem":"Provider 暂时无法确认到账，已保留源链交易记录。",
        "build":{"buildId":"b1","provider":"lifi","sourceChain":"ethereum","peerChain":"base", "legs":[],
            "initialQuoteAmountRaw":"10000000","finalQuoteAmountRaw":"10010000",
            "quoteObservedAtMs":100,"builtAtMs":100,"validUntilMs":1000,
            "atomic":false,"monitorOnly":false,"previewReady":true,"submitReady":true},
        "legs":[]
    })).unwrap();
    for (index, kind) in [
        Kind::SourceSwap,
        Kind::OutboundBridge,
        Kind::TargetSwap,
        Kind::ReturnBridge,
    ]
    .into_iter()
    .enumerate()
    {
        let position = index + 1;
        run.build.legs.push(serde_json::from_value(json!({
            "position":position,"kind":kind,"provider":"fixture","fromChain":"ethereum","toChain":"base",
            "fromAsset":"TOKEN","toAsset":"USDC","fromToken":"token","toToken":"usdc",
            "inputAmountRaw":"1000","expectedOutputAmountRaw":"10000100","inputDecimals":0,"outputDecimals":6,
            "officialDocsUrl":"https://example.test/docs","observedAtMs":100
        })).unwrap());
        run.legs.push(serde_json::from_value(json!({
            "position":position,"kind":kind,"clientActionId":format!("step-{position}"),
            "status":if position == 1 {"completed"} else if position == 2 {"paused"} else {"requote_required"},
            "attempts":if position <= 2 {1} else {0}, "plannedInputAmountRaw":"1000",
            "actualInputAmountRaw":if position <= 2 {Some("1000")} else {None},
            "actualOutputAmountRaw":if position == 1 {Some("10000100")} else {None},
            "sourceTransactionId":if position <= 2 {Some(format!("0x{}", "0123456789abcdef".repeat(4)))} else {None}
        })).unwrap());
    }
    run
}

#[test]
fn cross_chain_authorization_displays_actual_valuation_not_a_default_peg() {
    Owner::new().with(|| {
        let mut build = fixture().build;
        let missing = authorization_form(build.clone(), data(), RwSignal::new(500)).to_html();
        assert!(missing.contains("美元汇率待确认"));
        assert!(!missing.contains("= $1.000000"));
        build.quote_usd_valuation = Some(shared_types::OnchainUsdValuation {
            asset: "USDC".to_owned(),
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            source: "ws_push".to_owned(),
            usd_bid: 0.9,
            usd_ask: 0.91,
            observed_at_ms: 100,
        });
        let valued = authorization_form(build, data(), RwSignal::new(500)).to_html();
        assert!(valued.contains("1 USDC = $0.900000"));
        assert!(valued.contains("KRAKEN USDC/USD"));
        if let Ok(path) = std::env::var("CROSS_CHAIN_VALUATION_RENDER_PATH") {
            let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
            std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>跨链估值组件测试数据</title><style>{css}</style><body><section class=onchain-cross-chain-execution>{valued}{missing}</section></body></html>")).unwrap();
        }
    });
}

#[test]
fn cross_chain_render_preserves_actual_amounts_and_blocks_paused_submission() {
    Owner::new().with(|| {
        let data = data();
        let run = fixture();
        data.recovery.update(|state| state.accept_snapshot(vec![run.clone()], 500));
        let html = cross_chain_control(data, RwSignal::new(500)).to_html();
        assert_eq!(html.matches("class=\"cross-chain-progress-leg\"").count(), 4);
        assert!(html.contains("1000 TOKEN"));
        assert!(html.contains("10.0001 USDC"));
        assert!(html.contains("disabled"));
        assert!(html.contains("当前无可提交步骤"));
        assert!(html.contains("刷新记录"));
        assert!(html.contains("重新核验到账"));
        assert!(html.contains("Provider 暂时无法确认到账"));
        if let Ok(path) = std::env::var("CROSS_CHAIN_RENDER_PATH") {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let css = std::fs::read_to_string(root.join("styles/.generated/input.css")).unwrap();
            std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>跨链执行验证</title><style>{css}</style><body>{html}</body></html>")).unwrap();
        }
    });
}

#[test]
fn cross_chain_verification_render_distinguishes_window_from_manual_rounds() {
    Owner::new().with(|| {
        let mut run = fixture();
        run.status = RunStatus::AwaitingDestinationEvidence;
        run.legs[1].source_submitted_at_ms = Some(100);
        let html = run_panel(run.clone(), data(), RwSignal::new(100)).to_html();
        assert!(html.contains("自动核验剩余 120 分钟"));
        assert!(html.contains("非到账承诺"));
        run.status = RunStatus::Paused;
        run.legs[1].recovery_started_at_ms = Some(200);
        run.legs[1].recovery_checks = 12;
        let html = run_panel(run, data(), RwSignal::new(100_000_000)).to_html();
        assert!(html.contains("只读核验 12/12 轮"));
        assert!(html.contains("不重发转账"));
        assert!(html.contains("重新核验到账"));
        assert!(html.contains("当前无可提交步骤"));
        assert!(html.contains("1000 TOKEN"));
        assert!(!html.contains("自动核验剩余"));
    });
}

#[test]
fn bridge_recovery_render_separates_provider_claim_from_verified_credit() {
    Owner::new().with(|| {
        let mut report: shared_types::OnchainCrossChainRecovery = serde_json::from_value(json!({
            "providerStatus":"FAILED","substatus":"REFUNDED","reportedAmountRaw":"99000000",
            "receivingChainId":1,"receivingToken":"0xusdc","observedAtMs":100,
            "officialDocsUrl":"https://docs.li.fi"
        })).unwrap();
        let missing = recovery::summary(&report).to_html();
        assert!(missing.contains("钱包到账未确认"));
        assert!(missing.contains("99000000 最小单位"));
        assert!(!missing.contains("钱包已核实到账"));
        report.receipt = Some(serde_json::from_value(json!({
            "basis":{"chain":"ethereum","wallet":"0xwallet","transactionId":"0xrefund","requireSender":false,
                "assets":[{"symbol":"USDC","address":"0xusdc","decimals":6}]},
            "status":"complete","assetChangesRaw":["98000000"],"additionalNativeChangeRaw":"0",
            "blockRef":"0xblock","observedAtMs":200,
            "networkCost":{"chain":"ethereum","transactionId":"0xrefund","blockRef":"0xblock",
                "payer":"0xsponsor","asset":"ETH","executionFeeExact":"0.000021","additionalFeeExact":"0",
                "totalFeeExact":"0.000021","source":"receipt","observedAtMs":200}
        })).unwrap());
        let confirmed = recovery::summary(&report).to_html();
        assert!(confirmed.contains("钱包已核实到账"));
        assert!(confirmed.contains("99 USDC"));
        assert!(confirmed.contains("98 USDC"));
        assert!(confirmed.contains("0xwallet"));
        assert!(confirmed.contains("0xrefund"));
        assert!(confirmed.contains("其他地址支付，不重复计入本钱包链费"));
        assert!(confirmed.contains("原套利路径已停止"));
    });
}

#[test]
fn cross_chain_disposition_renders_remaining_and_blocked_without_execution_control() {
    Owner::new().with(|| {
        let mut plan: shared_types::OnchainCrossChainDisposition = serde_json::from_value(json!({
            "sourceRunId":"original-run","receiptsObservedAtMs":200,
            "originalCapital":{"chain":"ethereum","wallet":"0xoriginal","asset":{"symbol":"USDC","address":"0xusdc","decimals":6},"amountExact":"100"},
            "remainingAssets":[
                {"change":{"chain":"ethereum","wallet":"0xoriginal","asset":{"symbol":"USDC","address":"0xusdc","decimals":6},"amountExact":"98"},"action":"keep"},
                {"change":{"chain":"base","wallet":"0xpeer","asset":{"symbol":"USDT","address":"0xusdt","decimals":6},"amountExact":"0.000001"},"action":"quote_bridge"}],
            "blockers":[],"submitReady":false,"requiresLiveAuthorization":true
        })).unwrap();
        let html = disposition::summary(&plan).to_html();
        assert!(html.contains("本次剩余资金"));
        assert!(html.contains("原投入 100 USDC"));
        assert!(html.contains("98"));
        assert!(html.contains("0.000001"));
        assert!(html.contains("重新询价跨链返回"));
        assert!(html.contains("不是当前钱包余额"));
        assert!(html.contains("重新获得实盘授权"));
        assert!(!html.contains("<button"));
        assert!(html.contains("cross-chain-disposition"));
        plan.blockers = vec!["第 2 步桥款仍可能在途".into()];
        plan.remaining_assets.clear();
        let blocked = disposition::summary(&plan).to_html();
        assert!(blocked.contains("待核齐原交易"));
        assert!(!blocked.contains("没有正的剩余资金"));
        let mut run = fixture();
        run.accounting = Some(serde_json::from_value(json!({"status":"pending_receipts","flows":[],"netAssets":[],"problems":[],"disposition":plan})).unwrap());
        assert!(run_panel(run, data(), RwSignal::new(500)).to_html().contains("本次剩余资金与处置建议"));
    });
}

#[test]
fn cross_chain_recovery_plans_render_reservation_expiry_and_release_without_submission() {
    Owner::new().with(|| {
        let mut plan: shared_types::OnchainCrossChainRecoveryPlan = serde_json::from_value(json!({
            "planId":"recovery-render-plan","status":"awaiting_authorization","createdAtMs":200,"updatedAtMs":200,
            "preview":{"planId":"recovery-render-plan","sourceRunId":"paused-run","sourceRunUpdatedAtMs":100,"assetIndex":0,
                "input":{"chain":"base","wallet":"0xpeer","asset":{"symbol":"USDT","address":"0xusdt","decimals":6},"amountExact":"50"},
                "target":{"chain":"ethereum","wallet":"0xoriginal","asset":{"symbol":"USDC","address":"0xusdc","decimals":6},"amountExact":"100"},
                "inputAmountRaw":"50000000","minimumOutputAmountRaw":"48500000","provider":"lifi",
                "gasUsd":0.00000001,"validUntilMs":500,"blockers":[],"quoteReady":true,"submitReady":false,
                "requiresLiveAuthorization":true,"officialDocsUrl":"https://docs.li.fi"}
        })).unwrap();
        let data = data();
        let clock = RwSignal::new(400);
        data.recovery.update(|state| {
            state.plans = vec![plan.clone()];
            let mut run = fixture();
            run.run_id = "paused-run".into();
            run.updated_at_ms = 100;
            state.accept_snapshot(vec![run], 400);
        });
        let html = recovery_plans::panel("paused-run".into(), data, clock).to_html();
        assert!(html.contains("确认计划并预留"));
        assert!(html.contains("取消计划 / 释放预留"));
        assert!(html.contains("48.5 USDC"));
        assert!(html.contains("未知"));
        assert!(html.contains("0.000001"));
        assert!(html.contains("不会在链上冻结资产"));
        assert!(!html.contains("disabled"));
        assert!(!html.contains("执行交易"));
        let unrelated = recovery_plans::panel("other-run".into(), data, clock).to_html();
        assert!(!unrelated.contains("<section"));
        assert!(!unrelated.contains("recovery-render-plan"));
        plan.status = shared_types::OnchainCrossChainRecoveryPlanStatus::Reserved;
        data.recovery.update(|state| state.plans = vec![plan.clone()]);
        let reserved = recovery_plans::row(plan.clone(), data, clock).to_html();
        assert!(reserved.contains("已预留，未提交"));
        assert_eq!(reserved.matches("disabled").count(), 1);
        clock.set(500);
        let expired = recovery_plans::row(plan.clone(), data, clock).to_html();
        assert!(expired.contains("已过期，未提交"));
        assert_eq!(expired.matches("disabled").count(), 2);
        plan.status = shared_types::OnchainCrossChainRecoveryPlanStatus::Cancelled;
        data.recovery.update(|state| state.plans = vec![plan.clone()]);
        let cancelled = recovery_plans::row(plan, data, clock).to_html();
        assert!(cancelled.contains("已取消"));
        assert_eq!(cancelled.matches("disabled").count(), 2);
    });
}

#[test]
fn cross_chain_recovery_preview_render_never_implies_execution_or_reuses_expired_quote() {
    Owner::new().with(|| {
        let mut preview: shared_types::OnchainCrossChainRecoveryPreview = serde_json::from_value(json!({
            "sourceRunId":"paused-run","sourceRunUpdatedAtMs":100,"assetIndex":0,
            "input":{"chain":"base","wallet":"0xpeer","asset":{"symbol":"USDT","address":"0xusdt","decimals":6},"amountExact":"50"},
            "target":{"chain":"ethereum","wallet":"0xoriginal","asset":{"symbol":"USDC","address":"0xusdc","decimals":6},"amountExact":"100"},
            "inputAmountRaw":"50000000","balanceAmountRaw":"99000000","balanceCheckedAtMs":100,
            "routeId":"quote-only","provider":"lifi","expectedOutputAmountRaw":"49000000","minimumOutputAmountRaw":"48500000",
            "gasUsd":0.00000001,"estimatedDurationSeconds":15,"validUntilMs":500,
            "blockers":[],"quoteReady":true,"submitReady":false,"requiresLiveAuthorization":true,
            "officialDocsUrl":"https://docs.li.fi/api-reference/get-a-quote-for-a-token-transfer"
        })).unwrap();
        let clock = RwSignal::new(400);
        let html = disposition::preview_result(preview.clone(), clock).to_html();
        assert!(html.contains("报价已核对 · 未锁定资金"));
        assert!(html.contains("50 USDT"));
        assert!(html.contains("48.5 USDC"));
        assert!(html.contains("未知"));
        assert!(!html.contains("<button"));
        assert!(html.contains("未签名、未提交"));
        clock.set(500);
        assert!(disposition::preview_result(preview.clone(), clock).to_html().contains("预检已过期"));
        preview.quote_ready = false;
        preview.valid_until_ms = None;
        preview.blockers.push("当前链上余额低于所选处置金额".into());
        let html = disposition::preview_result(preview.clone(), clock).to_html();
        assert!(html.contains("余额低于"));
        assert!(!html.contains("报价已核对 · 未锁定资金"));
        preview.provider = "none".into();
        preview.route_id = None;
        preview.blockers = vec!["资产已在原钱包且为原报价币，无需换币或转账".into()];
        assert!(disposition::preview_result(preview, clock).to_html().contains("余额已核对 · 无需交易"));
    });
}

#[test]
fn cross_chain_authorization_does_not_enable_without_explicit_phrase() {
    Owner::new().with(|| {
        let data = data();
        data.recovery.update(|state| state.loaded = true);
        let mut build = fixture().build;
        let clock = RwSignal::new(500);
        assert!(authorization_form(build.clone(), data, clock)
            .to_html()
            .contains("disabled"));
        data.confirmation
            .set(ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE.into());
        assert!(authorization_form(build.clone(), data, clock)
            .to_html()
            .contains("disabled"));
        build.quote_usd_valuation = Some(shared_types::OnchainUsdValuation {
            asset: "USDC".to_owned(),
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            source: "ws_push".to_owned(),
            usd_bid: 0.9,
            usd_ask: 0.91,
            observed_at_ms: 100,
        });
        assert!(!authorization_form(build.clone(), data, clock)
            .to_html()
            .contains("disabled"));
        clock.set(1000);
        assert!(authorization_form(build, data, clock)
            .to_html()
            .contains("disabled"));
    });
}

#[test]
fn cross_chain_submission_contract_requires_a_specific_step() {
    assert!(
        serde_json::from_value::<OnchainCrossChainSubmitRequest>(json!({"runId":"r1"})).is_err()
    );
    let request: OnchainCrossChainSubmitRequest =
        serde_json::from_value(json!({"runId":"r1","expectedPosition":2})).unwrap();
    assert_eq!(request.expected_position, 2);
    assert!(
        serde_json::from_value::<OnchainCrossChainRecheckRequest>(json!({"runId":"r1"})).is_err()
    );
}

#[test]
fn cross_chain_wallet_receipt_render_separates_submitted_debit_fees_and_profit() {
    Owner::new().with(|| {
        let mut run = fixture();
        if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_RECEIPT_FIXTURE") {
            run = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        } else {
            run.legs.truncate(1);
            run.build.legs.truncate(1);
            run.legs[0].submitted_input_amount_raw = Some("1100".into());
            run.status = RunStatus::Completed;
        }
        let html = run_panel(run, data(), RwSignal::new(500)).to_html();
        assert!(html.contains("提交数量"));
        assert!(html.contains("实际扣款"));
        assert!(html.contains("实际到账"));
        assert!(html.contains("资产路径已完成"));
        assert!(!html.contains("闭环已完成"));
        if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_RECEIPT_HTML") {
            assert!(html.contains("99 USDT"));
            assert!(html.contains("100 USDT"));
            assert!(html.contains("105 USDC"));
            assert!(html.contains("网络费 0.000005 SOL"));
            assert!(html.contains("-0.000009 SOL"));
            assert!(html.contains("本钱包支付"));
            std::fs::write(path, html).unwrap();
        }
    });
}

#[test]
fn cross_chain_wallet_receipt_render_does_not_charge_sponsor_or_hide_unknown_fee() {
    Owner::new().with(|| {
        let missing = wallet_receipt("目标链收支", None).to_html();
        assert!(missing.contains("网络费待确认"));
        assert!(!missing.contains("网络费 0"));
        let receipt: shared_types::OnchainWalletReceipt = serde_json::from_value(json!({
            "basis":{"chain":"ethereum","wallet":"0xwallet","transactionId":"0xhash","assets":[],"requireSender":false},
            "status":"complete","assetChangesRaw":["100"],
            "networkCost":{"chain":"ethereum","transactionId":"0xhash","blockRef":"0xblock","payer":"0xsponsor",
                "asset":"ETH","executionFeeExact":"0.000021","additionalFeeExact":"0","totalFeeExact":"0.000021",
                "source":"receipt","observedAtMs":500}
        })).unwrap();
        let html = wallet_receipt("目标链收支", Some(&receipt)).to_html();
        assert!(html.contains("其他地址支付，不重复计入本钱包链费"));
        assert!(!html.contains("本钱包支付"));
    });
}

#[test]
fn cross_chain_render_does_not_offer_recheck_without_a_transaction_hash() {
    Owner::new().with(|| {
        let data = data();
        let mut run = fixture();
        run.legs[1].source_transaction_id = None;
        data.recovery
            .update(|state| state.accept_snapshot(vec![run], 500));
        assert!(!cross_chain_control(data, RwSignal::new(500))
            .to_html()
            .contains("重新核验到账"));
    });
}

#[test]
fn cross_chain_recovery_problem_is_visible_without_any_runs() {
    Owner::new().with(|| {
        let data = data();
        data.recovery.update(|s| { s.loaded = true; s.recovery_problem = Some("恢复日志第 3 行损坏，已停止新增资金动作".into()); });
        let html = cross_chain_control(data, RwSignal::new(500)).to_html();
        assert!(html.contains("恢复日志第 3 行损坏"));
        assert!(html.contains("刷新记录"));
        assert!(html.contains("role=\"alert\""));
    });
}

#[test]
fn cross_chain_costs_render_lists_fees_separately_from_wallet_movements() {
    Owner::new().with(|| {
        let mut run = fixture();
        if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_COST_FIXTURE") {
            run = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        } else {
            run.accounting = Some(serde_json::from_value(json!({
                "status":"valued", "flows":[], "netAssets":[],
                "externalFlows":[
                    {"location":"chain:ethereum:0xwallet","sourceId":"approval:fixture","asset":"ETH","amountExact":"-0.000021","kind":"approval_fee"},
                    {"location":"cex:kraken","sourceId":"replenishment:fixture","asset":"USDC","amountExact":"-0.1","kind":"replenishment_fee"}
                ],
                "usdValue":{"netUsdExact":"3.383937","valuedAtMs":2000,"rates":[]},"problems":[]
            })).unwrap());
        }
        let html = run_panel(run, data(), RwSignal::new(2500)).to_html();
        for expected in ["+3.383937 USD", "授权费", "补库费", "0.000021", "0.1", "未选费用未包含", "路径钱包变化"] {
            assert!(html.contains(expected), "missing {expected}");
        }
        assert!(!html.contains("+3.515958 USD"));
        if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_COST_HTML") { std::fs::write(path, html).unwrap(); }
    });
}

#[test]
fn cross_chain_accounting_render_separates_partial_flows_from_valued_results() {
    Owner::new().with(|| {
        let mut run = fixture();
        if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_ACCOUNTING_FIXTURE") {
            run = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        } else {
            run.accounting = Some(serde_json::from_value(json!({
                "status":"valued", "flows":[], "netAssets":[{"chain":"ethereum","wallet":"0xwallet",
                    "asset":{"symbol":"USDC","address":"0xtoken","decimals":6},"amountExact":"4"}],
                "usdValue":{"netUsdExact":"3.515958","valuedAtMs":2000,"rates":[]},"problems":[]
            })).unwrap());
        }
        let html = run_panel(run.clone(), data(), RwSignal::new(2500)).to_html();
        assert!(html.contains("本次资产净变动"));
        assert!(html.contains("+3.515958 USD"));
        assert!(html.contains("+4"));
        assert!(html.contains("不代表完整交易利润"));
        assert!(html.contains("资产位置"));
        if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_ACCOUNTING_HTML") {
            assert!(html.contains("-0.000022"));
            assert!(html.contains("-0.00002"));
            assert!(html.contains("桥款"));
            std::fs::write(path, &html).unwrap();
        }
        let accounting = run.accounting.as_mut().unwrap();
        for status in [
            shared_types::OnchainExecutionAccountingStatus::PendingReceipts,
            shared_types::OnchainExecutionAccountingStatus::PendingValuation,
        ] {
            accounting.status = status;
            // Even a malformed/stale USD value must not appear when the status is pending.
            let pending = accounting::summary(accounting).to_html();
            assert!(pending.contains("暂不计总额"));
            assert!(pending.contains("+4"));
            assert!(!pending.contains("+3.515958 USD"));
        }
    });
}
