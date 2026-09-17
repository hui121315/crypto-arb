use super::*;
use crate::state::load_state::LoadState;

#[test]
fn stock_peer_panel_keeps_native_quote_identity_and_no_execution_button() {
    Owner::new().with(|| {
        let mut snapshot = StockMarketSnapshot::default();
        let now = 1100;
        let security = StockSecurity {
            asset: "MU.US".into(),
            ticker: "MU".into(),
            name: "Micron Technology".into(),
            cusip: Some("595112103".into()),
            sessions: vec![],
            order_books: vec![],
            rfq_symbol: "MU.US_USDC_RFQ".into(),
        };
        snapshot.security = Some(security.clone());
        let quote = StockPeerQuote {
            symbol: "MUX/USD".into(),
            bid: "110".into(),
            ask: "111".into(),
            bid_quantity: None,
            ask_quantity: Some("2.5".into()),
            source: "ws_push".into(),
            source_at_ms: Some(1000),
            received_at_ms: 1000,
        };
        snapshot.peer = Some(StockPeerComparison {
            selection: StockPeerSelection {
                venue: "kraken".into(),
                product: StockPeerProduct::Spot,
                native_symbol: "MUx/USD".into(),
            },
            instrument: None,
            identity: StockPeerIdentity {
                underlying_verified: true,
                underlying_isin: Some("US5951121038".into()),
                product_isin: Some("CH1473121320".into()),
                issuer: Some("Backed Assets (JE) Limited".into()),
                sources: vec![],
                reason: "同一经济标的、不同发行方，不能互相充值".into(),
            },
            share_unit_verified: false,
            quote: Some(quote),
            quote_conversion: None,
            problem: None,
        });
        let data = StockData {
            peers: super::super::data::PeerData::defaults(),
            market: RwSignal::new(LoadState::Ready(snapshot)),
            catalog: RwSignal::new(LoadState::Ready(StockCatalog {
                rows: vec![security],
                observed_at_ms: now,
            })),
            search: RwSignal::new(String::new()),
            page: RwSignal::new(0),
            pending: RwSignal::new(false),
            notice: RwSignal::new(None),
            clock: RwSignal::new(now),
            watch: Callback::new(|_| {}),
            refresh: Callback::new(|_| {}),
            budget: RwSignal::new("100".into()),
            keyed: RwSignal::new(false),
            quote_pending: RwSignal::new(false),
            quote: Callback::new(|_| {}),
            monitor_pending: RwSignal::new(false),
            monitor: Callback::new(|_| {}),
            rfq: super::super::data::RfqData::fixture(),
            preflight: super::super::data::PreflightData::fixture(),
            alerts: super::super::data::AlertData::defaults(),
        };
        check_stock_identity_catalog(data);
        check_stock_order_validation(data);
        check_stock_peer_plans(data);
        check_stock_peer_execution(data);
        let html = peers::panel(data).to_html();
        for text in [
            "其他交易所对比",
            "MUx/USD",
            "请选择，不会自动配对",
            "未知计价币",
            "未知",
            "2.5",
            "不同发行方",
            "US5951121038",
            "CH1473121320",
            "股数换算待核实",
        ] {
            assert!(html.contains(text), "missing {text}");
        }
        assert!(!html.contains("构建并预留") && !html.contains("提交订单"));
        assert!(!html.contains("费用前试算"));
        data.peers.venue.set("bitget".into());
        let other_venue = peers::panel(data).to_html();
        assert!(other_venue.contains("kraken · MUx/USD"));
        data.peers.venue.set("kraken".into());
        data.clock.set(4001);
        assert!(peers::panel(data).to_html().contains("仅历史报价"));
        data.market.update(|state| {
            let mut snapshot = state.value().unwrap().clone();
            snapshot.peer.as_mut().unwrap().share_unit_verified = true;
            state.apply_result(Ok(snapshot));
        });
        let checked = peers::panel(data).to_html();
        assert!(checked.contains("费用前试算") && checked.contains("不是净利润"));
        assert!(checked.contains("检查充提") && checked.contains("不会自动转账"));
        if let Ok(path) = std::env::var("STOCK_PEER_API_CAPTURE_PATH") {
            let snapshot: StockMarketSnapshot =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let instrument = snapshot.peer.as_ref().unwrap().instrument.clone().unwrap();
            data.clock.set(snapshot.observed_at_ms);
            data.market.set(LoadState::Ready(snapshot));
            data.peers.catalog.set(Some(StockPeerCatalog {
                request: StockPeerCatalogRequest {
                    venue: "kraken".into(),
                    product: StockPeerProduct::Spot,
                    search: "MU".into(),
                },
                rows: vec![instrument],
                matched: 1,
                registry_count: 1,
            }));
            let html = page(data, || ()).to_html();
            assert!(
                html.contains("买价 / USD")
                    && html.contains("USDC/USD")
                    && (html.contains("股数换算待核实") || html.contains("行情按股数对比"))
                    && html.contains("股票价差 Webhook")
                    && html.contains("同时监控所选交易所")
                    && html.contains("链上 / Backpack")
            );
            if let Ok(path) = std::env::var("STOCK_PEER_RENDER_PATH") {
                write_stock_html(&path, &html);
            }
        }
        if let Ok(path) = std::env::var("STOCK_PEER_PREFLIGHT_CAPTURE_PATH") {
            let snapshot:StockMarketSnapshot=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            data.clock.set(snapshot.observed_at_ms);
            let spec=snapshot.peer.as_ref().unwrap().instrument.clone().unwrap();
            data.peers.catalog.set(Some(StockPeerCatalog{request:StockPeerCatalogRequest{venue:"kraken".into(),product:StockPeerProduct::Spot,search:"MU".into()},rows:vec![spec],matched:1,registry_count:1}));
            data.market.set(LoadState::Ready(snapshot));
            let html=peers::panel(data).to_html();
            for text in ["检查账户与费用","股票 taker 费率","换汇 taker 费率","0.1","0.2","不足","完整差额保持未知","执行未接通"] {assert!(html.contains(text),"missing {text}");}
            assert!(!html.contains("提交订单") && !html.contains("构建并预留"));
            if let Ok(path)=std::env::var("STOCK_PEER_PREFLIGHT_RENDER_PATH") {write_stock_html(&path,&html);}
            data.clock.update(|n|*n+=16_000);
            assert!(peers::panel(data).to_html().contains("账户未检查或已过期"));
        }
        if let Ok(path) = std::env::var("STOCK_PEER_FUNDING_CAPTURE_PATH") {
            let snapshot:StockMarketSnapshot=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let now=snapshot.observed_at_ms;
            data.clock.set(now);
            let spec=snapshot.peer.as_ref().unwrap().instrument.clone().unwrap();
            data.peers.catalog.set(Some(StockPeerCatalog{request:StockPeerCatalogRequest{venue:"kraken".into(),product:StockPeerProduct::Spot,search:"MU".into()},rows:vec![spec],matched:1,registry_count:1}));
            data.market.set(LoadState::Ready(snapshot));
            let html=peers::panel(data).to_html();
            for text in ["检查充提","MUx · 充值","USDC · 提现","未复权 Token 数量","非当前链上合约","合约匹配","费用计入方法金额","不是本次精确费用报价","读取未完成"] {
                assert!(html.contains(text),"missing {text}");
            }
            assert!(!html.contains("提交订单") && !html.contains("构建并预留"));
            if let Ok(path)=std::env::var("STOCK_PEER_FUNDING_RENDER_PATH") {write_stock_html(&path,&html);}
            data.clock.set(now+60_001);
            assert!(peers::panel(data).to_html().contains("历史资料，需重新检查"));
        }
        if let Ok(path) = std::env::var("STOCK_PEER_ALERT_CAPTURE_PATH") {
            let snapshot: StockMarketSnapshot = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let instrument = snapshot.peer.as_ref().unwrap().instrument.clone().unwrap();
            let config = snapshot.monitor.alerts.clone();
            assert!(snapshot.peer.as_ref().unwrap().share_unit_verified);
            assert_eq!(snapshot.alerts.recent.len(), 4);
            data.clock.set(snapshot.observed_at_ms);
            data.market.set(LoadState::Ready(snapshot));
            data.alerts.enabled.set(config.enabled);
            data.alerts.include_peer.set(config.include_peer);
            data.alerts.threshold.set(config.min_spread_pct);
            data.alerts.cooldown.set(config.cooldown_secs.to_string());
            data.peers.catalog.set(Some(StockPeerCatalog {request:StockPeerCatalogRequest {
                venue:"kraken".into(),product:StockPeerProduct::Spot,search:"MU".into()},rows:vec![instrument],matched:1,registry_count:1}));
            let html = view!{<div class="stock-section">{peers::panel(data)}{alerts::panel("MU.US".into(),data)}</div>}.to_html();
            for text in ["买量 / 股","卖量 / 股","费用前试算","同时监控所选交易所","kraken · MUx/USD","不是净利润","尚未确认投递"] {
                assert!(html.contains(text),"missing {text}");
            }
            assert!(!html.contains("提交订单") && !html.contains("构建并预留"));
            if let Ok(path) = std::env::var("STOCK_PEER_ALERT_RENDER_PATH") { write_stock_html(&path,&html); }
        }
    });
}

fn check_stock_order_validation(data: StockData) {
    let original = data.market.get_untracked();
    let clock = data.clock.get_untracked();
    let catalog = data.peers.catalog.get_untracked();
    let mut s = if let Ok(path) = std::env::var("STOCK_PEER_ORDER_CAPTURE_PATH") {
        serde_json::from_slice::<StockMarketSnapshot>(&std::fs::read(path).unwrap()).unwrap()
    } else {
        original.value().unwrap().clone()
    };
    let now = if s.observed_at_ms > 0 {
        s.observed_at_ms
    } else {
        clock
    };
    if let Some(spec) = s.peer.as_ref().and_then(|p| p.instrument.clone()) {
        data.peers.catalog.set(Some(StockPeerCatalog {
            request: StockPeerCatalogRequest {
                venue: "kraken".into(),
                product: StockPeerProduct::Spot,
                search: "MU".into(),
            },
            rows: vec![spec],
            matched: 1,
            registry_count: 1,
        }));
    }
    if s.peer_order_checks.is_empty() {
        s.peer_order_checks.push(StockPeerOrderCheck {
            draft: StockPeerOrderDraft {
                purpose: StockPeerOrderPurpose::Equity,
                request: StockPeerOrderCheckRequest {
                    asset: "MU.US".into(),
                    selection: s.peer.as_ref().unwrap().selection.clone(),
                    direction: StockChainDirection::Buy,
                },
                quantity: "0.02125".into(),
                limit_price: "600".into(),
                quote_asset: "USD".into(),
                prepared_at_ms: now,
                source_at_ms: now,
                metadata_at_ms: now,
            },
            completed_at_ms: Some(now),
            status: StockPeerOrderCheckStatus::Passed,
            message: "交易所仅验证当次股票参数通过；未成交，不代表套利执行已就绪".into(),
        });
    }
    let mut rejected = s.peer_order_checks[0].clone();
    rejected.draft.request.direction = StockChainDirection::Sell;
    rejected.status = StockPeerOrderCheckStatus::Rejected;
    rejected.message = "交易所验证拒绝：该股票腿可用余额不足".into();
    s.peer_order_checks.push(rejected);
    data.clock.set(now);
    data.market.set(LoadState::Ready(s.clone()));
    let html = peers::panel(data).to_html();
    for text in [
        "股票订单验证",
        "不成交、不换汇",
        "最近一次 · 参数验证通过",
        "最近一次 · 验证拒绝",
        "0.02125 股 / 600 USD",
        "可用余额不足",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(!html.contains("提交订单") && !html.contains("构建并预留"));
    if let Ok(path) = std::env::var("STOCK_PEER_ORDER_RENDER_PATH") {
        write_stock_html(&path, &html);
    }
    let r = &mut s.peer_order_checks[0];
    r.completed_at_ms = None;
    r.status = StockPeerOrderCheckStatus::Unknown;
    data.clock.set(now + 16_000);
    data.market.set(LoadState::Ready(s));
    let html = peers::panel(data).to_html();
    assert!(html.contains("未取得回复") && html.contains("未收到完整验证回复"));
    assert!(!html.contains("最近一次 · 参数验证通过"));
    if let Ok(path) = std::env::var("STOCK_PEER_ORDER_TIMEOUT_RENDER_PATH") {
        write_stock_html(&path, &html);
    }
    data.market.set(original);
    data.clock.set(clock);
    data.peers.catalog.set(catalog);
}

fn check_stock_peer_plans(data: StockData) {
    let Ok(path) = std::env::var("STOCK_PEER_PLAN_CAPTURE_PATH") else {
        return;
    };
    let original = data.market.get_untracked();
    let clock = data.clock.get_untracked();
    let wallet = data.preflight.wallet.get_untracked();
    let snapshot: StockMarketSnapshot =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let p = snapshot.peer_plans[0].clone();
    data.clock.set(p.terms.created_at_ms);
    data.preflight.wallet.set(p.request.wallet_address.clone());
    data.market.set(LoadState::Ready(snapshot));
    let render = || {
        view!{<section class="stock-section stock-peers">{peer_plans::builder(data)}{peer_plans::history(data)}</section>}.to_html()
    };
    let html = render();
    for text in [
        "先预留 · 确认后双边提交",
        "保存双边计划",
        "Kraken 双边计划记录",
        "kraken · USD",
        "资金预留",
        "费用后差额估算",
        "取消预留",
        "不是已锁定利润",
        "确认本次真实双边交易",
        "提交双边计划",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(!html.contains("提交订单") && !html.contains("已经成交"));
    assert!(html.contains(&p.terms.allocations[0].quantity));
    if let Ok(path) = std::env::var("STOCK_PEER_PLAN_RENDER_PATH") {
        write_stock_html(&path, &html);
    }
    data.clock.set(p.terms.reserved_until_ms);
    let expired = render();
    assert!(expired.contains("预留已到期 · 未下单") && expired.contains("已过期，需重建"));
    if let Ok(path) = std::env::var("STOCK_PEER_PLAN_EXPIRED_RENDER_PATH") {
        write_stock_html(&path, &expired);
    }
    data.market.update(|m| {
        let mut s = m.value().unwrap().clone();
        s.security = None;
        s.peer = None;
        m.apply_result(Ok(s));
    });
    let history = peer_plans::history(data).to_html();
    assert!(
        history.contains(&p.plan_id),
        "switching stocks must not hide outstanding records"
    );
    data.market.set(original);
    data.clock.set(clock);
    data.preflight.wallet.set(wallet);
}

fn check_stock_peer_execution(data: StockData) {
    let Ok(path) = std::env::var("STOCK_PEER_EXECUTION_CAPTURE_PATH") else {
        return;
    };
    let original = data.market.get_untracked();
    let clock = data.clock.get_untracked();
    let mut snapshot: StockMarketSnapshot =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let p = snapshot.peer_plans[0].clone();
    data.clock.set(p.terms.reserved_until_ms + 100000);
    snapshot.security = None;
    snapshot.peer = None;
    data.market.set(LoadState::Ready(snapshot.clone()));
    let render = || {
        view! {<section class="stock-section stock-peers">{peer_plans::history(data)}</section>}
            .to_html()
    };
    let html = render();
    for text in [
        "提交已记录",
        "资金保持占用",
        "双边原始回执",
        "已成交 · 费用已核实",
        "链上回执已确认",
        "实际收支",
        "USD 已核实费后收支",
        "核对原双边交易",
        "已冻结 · 仅核对原交易",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(html.contains(
        &p.cex_order
            .as_ref()
            .unwrap()
            .cash_settlement()
            .unwrap()
            .quote_change
    ));
    for wrong in ["提交双边计划", "取消预留", "已取消 · 未下单", "预留已到期"]
    {
        assert!(!html.contains(wrong), "unsafe submitted state {wrong}");
    }
    if let Ok(path) = std::env::var("STOCK_PEER_EXECUTION_RENDER_PATH") {
        write_stock_html(&path, &html);
    }
    let p = &mut snapshot.peer_plans[0];
    p.cex_order = Some(
        StockPeerOrderReceipt::pending(
            p.terms.draft.clone(),
            p.cex_order.as_ref().unwrap().client_order_id.clone(),
        )
        .unwrap(),
    );
    p.chain_submission.as_mut().unwrap().receipt = None;
    p.chain_submission.as_mut().unwrap().provider_acknowledged = true;
    snapshot.peer_accounting.clear();
    data.market.set(LoadState::Ready(snapshot));
    let pending = render();
    assert!(
        pending.contains("结果未明 · 只查询原订单")
            && pending.contains("Provider 已接收 · 等待链上回执")
    );
    assert!(!pending.contains("已成交 · 费用已核实") && !pending.contains("提交双边计划"));
    assert!(pending.contains("收支待核齐") && !pending.contains("USD 已核实费后收支"));
    if let Ok(path) = std::env::var("STOCK_PEER_EXECUTION_PENDING_RENDER_PATH") {
        write_stock_html(&path, &pending);
    }
    for (capture, output, expected) in [
        (
            "STOCK_PEER_HISTORY_CAPTURE_PATH",
            "STOCK_PEER_HISTORY_RENDER_PATH",
            "已核齐 · 查询 1 次",
        ),
        (
            "STOCK_PEER_HISTORY_PENDING_CAPTURE_PATH",
            "STOCK_PEER_HISTORY_PENDING_RENDER_PATH",
            "等待核齐 · 查询 1 次，不重发订单",
        ),
        (
            "STOCK_PEER_ACCOUNTING_CAPTURE_PATH",
            "STOCK_PEER_ACCOUNTING_RENDER_PATH",
            "原交易收支已核齐 · 未结算",
        ),
        (
            "STOCK_PEER_ACCOUNTING_RECOVERY_CAPTURE_PATH",
            "STOCK_PEER_ACCOUNTING_RECOVERY_RENDER_PATH",
            "补偿参考：卖出多余的 0.016 个链上股票代币",
        ),
    ] {
        if let Ok(path) = std::env::var(capture) {
            let snapshot: StockMarketSnapshot =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            data.market.set(LoadState::Ready(snapshot));
            let html = render();
            assert!(html.contains(expected), "missing history state {expected}");
            assert!(html.contains("核对原双边交易") && !html.contains("提交双边计划"));
            if capture.starts_with("STOCK_PEER_ACCOUNTING") {
                for text in [
                    "-12.03202 USD",
                    "USD 已核实费后收支",
                    "USDC 已核实费后收支",
                    "股票份额净变化",
                    "资产位置与原始数量",
                    "不能把两种币直接相加",
                ] {
                    assert!(html.contains(text), "missing actual accounting {text}");
                }
                assert!(!html.contains("已结算") && !html.contains("已实现收益"));
            }
            if let Ok(path) = std::env::var(output) {
                write_stock_html(&path, &html);
            }
        }
    }
    for (state, expected) in [
        ("READY", "待确认 · 未提交"),
        ("PENDING", "提交结果待核对 · 不重发"),
        ("COMPLETED", "补偿到账已核实"),
    ] {
        let Ok(path) = std::env::var(format!("STOCK_PEER_RECOVERY_{state}_CAPTURE_PATH")) else {
            continue;
        };
        let snapshot: StockMarketSnapshot =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let p = &snapshot.peer_plans[0];
        let now = p.updated_at_ms;
        let expiry = p.recoveries.last().unwrap().cost.valid_until_ms;
        data.clock.set(now);
        data.market.set(LoadState::Ready(snapshot));
        let html = render();
        for text in ["股票差额补偿", expected, "资金保持占用", "本次 USDC 限额"] {
            assert!(
                html.contains(text),
                "missing recovery state {state}: {text}"
            );
        }
        assert!(!html.contains("提交双边计划") && !html.contains("已实现收益"));
        if state == "READY" {
            assert!(html.contains("inputmode=\"decimal\"") && html.contains("最低收到 USDC"));
            let marker = html.find("stock-peer-recovery-submit").unwrap();
            let start = html[..marker].rfind('<').unwrap();
            let end = marker + html[marker..].find('>').unwrap();
            let tag = &html[start..=end];
            assert!(
                tag.contains("disabled"),
                "unconfirmed recovery enabled: {tag}"
            );
            assert!(!html.contains("核对原补偿交易"));
        } else {
            assert!(!html.contains("提交补偿") && !html.contains("取消补偿预留"));
            assert_eq!(html.contains("核对原补偿交易"), state == "PENDING");
        }
        if state == "COMPLETED" {
            for text in [
                "网络费 · 含补偿交易",
                "0 股",
                "11 USDC",
                "0.000021 SOL",
                "Solana · 补偿 2",
            ] {
                assert!(html.contains(text), "missing recovery receipt {text}");
            }
            assert!(!html.contains("补偿参考："));
        }
        if let Ok(path) = std::env::var(format!("STOCK_PEER_RECOVERY_{state}_RENDER_PATH")) {
            write_stock_html(&path, &html);
        }
        if state == "READY" {
            data.clock.set(expiry + 1);
            let expired = render();
            assert!(expired.contains("报价已过期 · 不能提交"));
        }
    }
    for (state, expected) in [
        ("READY", "待确认 · 未提交"),
        ("PENDING", "换汇回执待核对 · 不重发"),
        ("COMPLETED", "换汇成交与费用已核实"),
    ] {
        let Ok(path) = std::env::var(format!("STOCK_PEER_CONVERSION_{state}_CAPTURE_PATH")) else {
            continue;
        };
        let snapshot: StockMarketSnapshot =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let p = &snapshot.peer_plans[0];
        let expiry = p.conversions.last().unwrap().valid_until_ms;
        data.clock.set(p.updated_at_ms);
        data.market.set(LoadState::Ready(snapshot));
        let html = render();
        for text in [
            "原币换汇",
            expected,
            "资金保持占用",
            "原币净收支预算",
            "其中手续费预算",
        ] {
            assert!(
                html.contains(text),
                "missing conversion state {state}: {text}"
            );
        }
        assert!(!html.contains("提交双边计划") && !html.contains("已实现收益"));
        if state == "READY" {
            assert!(html.contains("inputmode=\"decimal\"") && html.contains("最多支出 USDC"));
            let marker = html.find("stock-peer-conversion-submit").unwrap();
            let start = html[..marker].rfind('<').unwrap();
            let end = marker + html[marker..].find('>').unwrap();
            assert!(
                html[start..=end].contains("disabled"),
                "unconfirmed conversion enabled"
            );
            assert!(!html.contains("核对原换汇订单"));
        } else {
            assert!(!html.contains("提交换汇") && !html.contains("取消换汇报价"));
            assert_eq!(html.contains("核对原换汇订单"), state == "PENDING");
        }
        if state == "COMPLETED" {
            for text in ["实际 USDC 收支", "实际 USD 收支", "Kraken · 换汇 1"] {
                assert!(html.contains(text), "missing actual conversion {text}");
            }
            assert!(!html.contains("尚无实际换汇回执"));
            assert!(html.contains("保留原币余款") && !html.contains("获取换汇报价"));
        }
        if let Ok(path) = std::env::var(format!("STOCK_PEER_CONVERSION_{state}_RENDER_PATH")) {
            write_stock_html(&path, &html);
        }
        if state == "READY" {
            data.clock.set(expiry + 1);
            assert!(render().contains("报价已过期 · 不能提交"));
        }
    }
    for (state, expected) in [
        ("READY", "待确认 · 未提交"),
        ("PENDING", "提交结果待核对 · 不重发"),
        ("COMPLETED", "SOL 补回到账已核实"),
    ] {
        let Ok(path) = std::env::var(format!("STOCK_PEER_NATIVE_{state}_CAPTURE_PATH")) else {
            continue;
        };
        let snapshot: StockMarketSnapshot =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        data.clock.set(snapshot.peer_plans[0].updated_at_ms);
        data.market.set(LoadState::Ready(snapshot));
        let html = render();
        for text in ["SOL 费用补回", expected, "资金保持占用", "最低净补回 / SOL"] {
            assert!(html.contains(text), "missing native state {state}: {text}");
        }
        assert!(!html.contains("提交双边计划") && !html.contains("已实现收益"));
        if state == "READY" {
            assert!(
                html.contains("inputmode=\"decimal\"") && html.contains("SOL 补回最多支出 USDC")
            );
            let marker = html.find("stock-peer-native-submit").unwrap();
            let start = html[..marker].rfind('<').unwrap();
            let end = marker + html[marker..].find('>').unwrap();
            assert!(
                html[start..=end].contains("disabled"),
                "unconfirmed SOL submit enabled"
            );
        } else {
            assert!(!html.contains("提交 SOL 补回") && !html.contains("取消 SOL 补回"));
            assert_eq!(html.contains("核对原 SOL 补回"), state == "PENDING");
        }
        if state == "COMPLETED" {
            for text in [
                "实际 USDC 收支",
                "实际钱包 SOL 净变化",
                "0.000021 SOL",
                "Solana · SOL 补回 2",
            ] {
                assert!(html.contains(text), "missing native receipt {text}");
            }
            assert!(!html.contains("待补回 ") && !html.contains("获取 SOL 补回报价"));
        }
        if let Ok(path) = std::env::var(format!("STOCK_PEER_NATIVE_{state}_RENDER_PATH")) {
            write_stock_html(&path, &html);
        }
    }
    for (state, expected) in [
        ("READY", "待确认 · 未提交"),
        ("PENDING", "原订单或费用待核对 · 不重发"),
        ("COMPLETED", "库存订单成交与费用已核实"),
    ] {
        let Ok(path) = std::env::var(format!("STOCK_PEER_INVENTORY_{state}_CAPTURE_PATH")) else {
            continue;
        };
        let snapshot: StockMarketSnapshot =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let end = snapshot.peer_plans[0]
            .inventory_orders
            .last()
            .unwrap()
            .valid_until_ms;
        data.clock.set(snapshot.peer_plans[0].updated_at_ms);
        data.market.set(LoadState::Ready(snapshot));
        let html = render();
        for text in [
            "交易所库存恢复",
            expected,
            "资金保持占用",
            "手续费预算 / USD",
            "不是再次套利",
        ] {
            assert!(html.contains(text), "missing inventory {state}: {text}");
        }
        assert!(!html.contains("提交双边计划") && !html.contains("已实现收益"));
        if state == "READY" {
            assert!(html.contains("inputmode=\"decimal\"") && html.contains("最低收到 USD"));
            let marker = html.find("stock-peer-inventory-submit").unwrap();
            let start = html[..marker].rfind('<').unwrap();
            let stop = marker + html[marker..].find('>').unwrap();
            assert!(html[start..=stop].contains("disabled"));
        } else {
            assert!(!html.contains("提交库存恢复"));
            assert_eq!(html.contains("核对原库存订单"), state == "PENDING");
        }
        if state == "COMPLETED" {
            assert!(!html.contains("获取库存恢复报价"));
            assert!(html.contains("Kraken · 库存恢复 1"));
        }
        if let Ok(path) = std::env::var(format!("STOCK_PEER_INVENTORY_{state}_RENDER_PATH")) {
            write_stock_html(&path, &html);
        }
        if state == "READY" {
            data.clock.set(end + 1);
            assert!(render().contains("报价已过期 · 不能提交"));
        }
    }
    data.market.set(original);
    data.clock.set(clock);
}

fn check_stock_identity_catalog(data: StockData) {
    let old_catalog = data.catalog.get_untracked();
    let old_market = data.market.get_untracked();
    let mut rows = Vec::new();
    for (asset, name, cusip) in [
        ("MU.US", "Micron", Some("595112103")),
        ("SNDK.US", "Sandisk Corporation", Some("80004C200")),
        ("SPCX.US", "SpaceX", None),
        ("AAPL.US", "Apple", Some("037833100")),
    ] {
        let mut security = old_catalog.value().unwrap().rows[0].clone();
        security.asset = asset.into();
        security.ticker = asset.trim_end_matches(".US").into();
        security.name = name.into();
        security.cusip = cusip.map(str::to_owned);
        security.rfq_symbol = format!("{asset}_USDC_RFQ");
        rows.push(security);
    }
    let mut catalog = StockCatalog {
        rows,
        observed_at_ms: 1100,
    };
    if let Ok(path) = std::env::var("STOCK_IDENTITY_PUBLIC_CAPTURE_PATH") {
        let capture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        catalog = serde_json::from_value(capture["catalog"].clone()).unwrap();
    }
    assert_eq!(catalog_rows(&catalog, "", true).len(), 3);
    assert_eq!(catalog_rows(&catalog, "SNDK", true)[0].asset, "SNDK.US");
    assert!(catalog_rows(&catalog, "AAPL", true).is_empty());
    let rows = catalog_rows(&catalog, "", false);
    assert!(rows
        .iter()
        .take(3)
        .all(|s| identity::backpack_issuer(s).is_ok()));
    data.catalog.set(LoadState::Ready(catalog));
    let filtered = RwSignal::new(false);
    let render = || catalog_with_filter(data, filtered).to_html();
    assert!(render().contains("链上资料已核实 3 /"));
    assert!(render().contains("链上资料待核实"));
    data.page.set(99);
    filtered.set(true);
    let narrow = render();
    assert!(narrow.contains("SNDK") && narrow.contains("SPCX") && narrow.contains("1 / 1"));
    assert!(!narrow.contains("链上资料待核实"));
    data.search.set("AAPL".into());
    assert!(render().contains("没有符合条件的证券"));
    data.search.set(String::new());
    data.page.set(0);
    filtered.set(false);
    if let Ok(path) = std::env::var("STOCK_IDENTITY_PEER_CAPTURE_PATH") {
        let snapshot: StockMarketSnapshot =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        data.clock.set(snapshot.observed_at_ms);
        let instrument = snapshot.peer.as_ref().unwrap().instrument.clone().unwrap();
        data.peers.catalog.set(Some(StockPeerCatalog {
            request: StockPeerCatalogRequest {
                venue: "kraken".into(),
                product: StockPeerProduct::Spot,
                search: "SNDK".into(),
            },
            rows: vec![instrument],
            matched: 1,
            registry_count: 1,
        }));
        data.market.set(LoadState::Ready(snapshot));
        let html = page(data, || ()).to_html();
        for text in ["SNDKx/USD", "股数对比", "只看链上资料已核实", "更新询价"] {
            assert!(html.contains(text), "missing {text}");
        }
        assert!(!html.contains("stock-identity-problem"));
        if let Ok(path) = std::env::var("STOCK_IDENTITY_RENDER_PATH") {
            write_stock_html(&path, &html);
        }
        data.market.update(|m| {
            let mut snapshot = m.value().unwrap().clone();
            snapshot.tokens[0].contract_address = Some("wrong-token".into());
            m.apply_result(Ok(snapshot));
        });
        let blocked = comparison::panel("SNDK.US".into(), data).to_html();
        assert!(blocked.contains("官方资产目录与发行方合约/精度不一致"));
        assert!(blocked
            .split("type=\"submit\"")
            .nth(1)
            .unwrap()
            .split('>')
            .next()
            .unwrap()
            .contains("disabled"));
        if let Ok(path) = std::env::var("STOCK_IDENTITY_BLOCKED_RENDER_PATH") {
            write_stock_html(&path, &page(data, || ()).to_html());
        }
    }
    data.catalog.set(old_catalog);
    data.market.set(old_market);
    data.clock.set(1100);
    data.peers.catalog.set(None);
}

#[test]
fn stock_arbitrage_page_renders_exact_contracts_reference_only_and_unknown_without_zero() {
    Owner::new().with(||{
        let security=StockSecurity {asset:"AAPL.US".into(),ticker:"AAPL".into(),name:"Apple Inc.".into(),cusip:Some("037833100".into()),sessions:vec![],
            order_books:vec![StockOrderBookMarket {symbol:"AAPL.US_USDC".into(),quote:"USDC".into(),state:"Open".into(),tick_size:"0.01".into(),min_quantity:"0.01".into(),step_size:"0.01".into()}],rfq_symbol:"AAPL.US_USDC_RFQ".into()};
        let token=StockChainToken {blockchain:"Solana".into(),contract_address:Some("AAPLEDt8RpzPgXyhvFzkMBofvFSQw9gpeMCoUdPdLnB8".into()),native_decimals:Some(6),deposit_enabled:Some(false),withdraw_enabled:None,minimum_deposit:None,minimum_withdrawal:None,maximum_withdrawal:None,withdrawal_fee:None};
        let snapshot=StockMarketSnapshot {security:Some(security.clone()),tokens:vec![token],connected:true,
            reference:Some(StockReferenceQuote {ticker:"AAPL".into(),bid:None,ask:None,mid:"207.2".into(),session:None,source_at_ms:1000,received_at_ms:1050}),..Default::default()};
        let data=StockData {peers:super::super::data::PeerData::defaults(),market:RwSignal::new(LoadState::Ready(snapshot)),catalog:RwSignal::new(LoadState::Ready(StockCatalog{rows:vec![security],observed_at_ms:1000})),search:RwSignal::new(String::new()),page:RwSignal::new(0),pending:RwSignal::new(false),notice:RwSignal::new(None),clock:RwSignal::new(1100),watch:Callback::new(|_|{}),refresh:Callback::new(|_|{}),budget:RwSignal::new("10".into()),keyed:RwSignal::new(false),quote_pending:RwSignal::new(false),quote:Callback::new(|_|{}),monitor_pending:RwSignal::new(false),monitor:Callback::new(|_|{}),rfq:super::super::data::RfqData::fixture(),preflight:super::super::data::PreflightData::fixture(),alerts:super::super::data::AlertData::defaults()};
        let render=||page(data,||()).to_html();
        let html=render();
        for text in ["股票套利","AAPL.US_USDC","外部股票参考","207.2","未知","关闭","未计算","仅观察","AAPLEDt8RpzPgXyhvFzkMBofvFSQw9gpeMCoUdPdLnB8"] {assert!(html.contains(text),"missing {text}");}
        assert_eq!(quote_age(false,Some(1000),1100),"已断开 · 不可执行");
        assert!(quote_age(true,Some(1000),5000).contains("陈旧"));
        assert!(html.contains("链买预算 / USDC"));
        assert!(html.contains("更新询价"));
        assert!(html.contains("持续询价已关闭"));
        for text in ["价差 Webhook","报价差额 ≥ / %","最短提醒间隔 / 秒","提醒已关闭"] {assert!(html.contains(text),"missing {text}");}
        assert!(html.contains("交易通道待核验"));
        assert!(html.contains("检查库存与成本"));
        assert!(html.contains("Solana 钱包地址"));
        assert!(html.contains("读取 Backpack 充值地址"));
        assert_eq!(html.matches("构建并预留").count(), 2);
        assert!(!html.contains("试算链上费用"));
        assert!(html.contains("暂无已保存计划"));
        for text in ["发送询价（不成交）","询价股数","暂无股票询价记录"]{assert!(html.contains(text),"missing {text}");}
        let baseline=data.market.get_untracked();
        data.market.update(|state| {
            let mut snapshot=state.value().unwrap().clone();
            snapshot.monitor.enabled=true;
            snapshot.monitor.phase=StockMonitorPhase::Backoff;
            snapshot.monitor.problem=Some("反向报价暂时失败，保留链买快照".into());
            snapshot.trading_route=Some(StockTradingRoute{kind:StockRouteKind::Rfq,session:Some(StockSession{name:"US_EQUITIES_OVERNIGHT".into(),min_quantity:"1".into(),max_quantity:None,step_size:"1".into()}),symbol:Some("AAPL.US_USDC_RFQ".into()),reason:"股票时段内使用 RFQ".into(),timezone:Some("America/New_York".into()),calendar_at_ms:Some(1000),valid_until_ms:2000});
            state.apply_result(Ok(snapshot));
        });
        let monitoring=render();
        for text in ["当前通道 · RFQ","美股夜盘","最少 1 股","等待重试","应用监控参数","反向报价暂时失败"] {assert!(monitoring.contains(text),"missing {text}");}
        data.market.update(|state| {
            let mut snapshot=state.value().unwrap().clone();
            snapshot.monitor.phase=StockMonitorPhase::QuantityLimited;
            snapshot.monitor.problem=Some("本次最低到账 0.010658 股，按 1 股步长对齐后不足当前时段最小股数 1".into());
            snapshot.monitor.next_attempt_at_ms=Some(31_100);
            snapshot.reference=None;
            state.apply_result(Ok(snapshot));
        });
        let limited=render();
        for text in ["金额与当前时段不匹配","30s 后重查数量限制","0.010658 股","外部参考源尚未返回报价","WS 已连接"] {assert!(limited.contains(text),"missing {text}");}
        assert!(!limited.contains("等待重试"));
        if let Ok(path)=std::env::var("STOCK_LIMIT_RENDER_PATH") { write_stock_html(&path,&limited); }
        data.market.update(|state|{let mut snapshot=state.value().unwrap().clone();add_rfq_fixture(&mut snapshot,1100);state.apply_result(Ok(snapshot));});
        let quoting=render();
        for text in ["已收到报价","101.05","剩余 60s","核对原询价","取消询价"]{assert!(quoting.contains(text),"missing {text}");}
        data.clock.set(2100);
        assert!(render().contains("交易日历状态已陈旧"));
        data.clock.set(1100);
        data.market.set(baseline);
        data.market.update(|m|{let mut s=m.value().unwrap().clone();s.deposit_address=Some(StockDepositAddress{
            asset:"AAPL.US".into(),address:"Hc2D2As4vz9DZVYd3jJMCkiDEKjbUc1W8cf8vrGFfULz".into(),blockchain:"Solana".into(),account_fingerprint:"local-fixture".into(),checked_at_ms:1100,
        });m.apply_result(Ok(s));});
        assert!(render().contains("官方账户地址 · Solana"));
        data.clock.set(31_101);assert!(render().contains("历史地址 · 使用前重新读取"));data.clock.set(1100);
        data.market.update(|m|{let mut s=m.value().unwrap().clone();s.deposit_address=None;m.apply_result(Ok(s));});
        let html = if let Ok(snapshot_path)=std::env::var("STOCK_RENDER_SNAPSHOT") {
            let mut snapshot:StockMarketSnapshot=serde_json::from_slice(&std::fs::read(snapshot_path).unwrap()).unwrap();
            let clock=snapshot.comparison.as_ref().map(|c|c.sell.as_ref().map(|q|q.received_at_ms).unwrap_or(c.buy.received_at_ms)).unwrap_or(snapshot.observed_at_ms);
            data.clock.set(clock);
            add_rfq_fixture(&mut snapshot,clock);
            let mut settling=snapshot.rfqs[0].clone();
            settling.request.request_id="local-fixture-stock-filled-002".into();
            settling.phase=StockRfqPhase::Filled;
            settling.candidate=None;
            settling.executed_quantity=Some("0.5".into());
            settling.executed_quote_quantity=Some("50.525".into());
            settling.fills=vec![StockRfqFill{quote_id:"9007199254740998".into(),quantity:"0.5".into(),quote_quantity:"50.525".into(),price:"101.05".into()}];
            settling.needs_recheck=true;
            settling.settlement=StockRfqSettlement{attempts:6,next_at_ms:Some(clock+30_000),paused:true};
            snapshot.rfqs.push(settling);
            add_preflight_fixture(&mut snapshot,clock);
            let mut unprepared = snapshot.clone();
            unprepared.preflight = None;
            unprepared.chain_costs.clear();
            data.preflight.wallet.set("11111111111111111111111111111111".into());
            data.budget.set(unprepared.comparison.as_ref().unwrap().budget_usdc.clone());
            data.keyed.set(unprepared.comparison.as_ref().unwrap().keyed);
            data.market.set(LoadState::Ready(unprepared));
            let buttons = |html: String| html.split("<button").filter_map(|s| s.split_once("</button>").map(|(b,_)| b))
                .filter(|b| b.contains("构建并预留")).map(|b| b.contains("disabled")).collect::<Vec<_>>();
            assert_eq!(buttons(render()), vec![false, false], "build does not require a prior cost or preflight click");
            data.preflight.pending.set(true);
            assert!(render().contains("构建中…"));
            data.preflight.pending.set(false);
            let budget=data.budget.get_untracked();data.budget.set("98765".into());
            assert_eq!(buttons(render()),vec![true,true],"changed draft amount must first update its comparison");
            data.budget.set(budget);
            add_plan_fixture(&mut snapshot,clock);
            snapshot.monitor.enabled=true;
            snapshot.monitor.alerts.enabled=true;
            snapshot.alerts.phase=StockAlertPhase::Cooldown;
            snapshot.alerts.recent=vec![StockAlertSummary {event_id:"local-stock-observation".into(),direction:"链买 / Backpack 卖".into(),gross_usdc:"2".into(),spread_pct:"20".into(),queued_at_ms:clock,delivery:None}];
            data.alerts.enabled.set(true);
            data.preflight.wallet.set("11111111111111111111111111111111".into());
            data.catalog.set(LoadState::Ready(StockCatalog{rows:snapshot.security.clone().into_iter().collect(),observed_at_ms:clock}));
            data.market.set(LoadState::Ready(snapshot));
            let rendered=render();
            for text in ["执行计划","已预留 · 未下单","取消预留","计划凭据","备款 · 双腿收支","全部成交或取消","不借款","已成交 · 核验已暂停","成交金额不等于扣费后到账","自动核验 6/6"] {assert!(rendered.contains(text),"missing {text}");}
            data.market.update(|m|{let mut s=m.value().unwrap().clone();s.plans[0].phase=StockPlanPhase::Cancelled;s.plans[0].revision+=1;m.apply_result(Ok(s));});
            let cancelled=render();
            assert!(cancelled.contains("已取消"));
            assert!(!cancelled.contains("已预留 · 未下单"));
            data.market.update(|m|{let mut s=m.value().unwrap().clone();s.plans[0].phase=StockPlanPhase::Reserved;s.plans[0].revision+=1;m.apply_result(Ok(s));});
            data.clock.set(clock+60_000);
            assert!(render().contains("预留已到期"));
            data.market.update(|m|{let mut s=m.value().unwrap().clone();s.plans[0].phase=StockPlanPhase::SubmissionUnknown;s.plans[0].revision+=1;m.apply_result(Ok(s));});
            let unresolved=render();
            assert!(unresolved.contains("提交待核对 · 保留占用"));
            assert!(!unresolved.contains("预留已到期"));
            data.market.update(|m|{let mut s=m.value().unwrap().clone();s.plans[0].phase=StockPlanPhase::Reserved;s.plans[0].revision+=1;m.apply_result(Ok(s));});
            data.clock.set(clock);
            for text in ["模拟钱包净扣 / SOL", "保守周转余额 / SOL", "0.00293716", "不代替周转余额"] {assert!(rendered.contains(text),"missing {text}");}
            for text in ["最低到账", "股数倍率", "余量", "不是可执行净利润", "已知费用后差额", "只读预检", "0.1%", "0.00204628", "账户租金估算", "其他付款方", "历史费用快照", "SOL 补回预算 / USDC", "0.215", "最低报价到账 0.0021 SOL", "提醒冷却中", "已入队 · 尚未确认投递"] {assert!(rendered.contains(text),"missing {text}");}
            data.clock.set(clock+5000);
            assert!(render().contains("预算已失效"));
            data.clock.set(clock);
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let cost=&mut s.chain_costs[0];
                let valuation=cost.native_valuation.as_mut().unwrap();
                valuation.replenishment=Some(StockNativeReplenishment {
                    wallet_address:cost.wallet_address.clone(),
                    transaction:shared_types::OnchainUnsignedTransaction::SolanaVersioned {
                        transaction_base64:"local-unsigned-fixture".into(),request_id:"native-fixture".into(),router:valuation.quote.router.clone(),mode:"manual".into(),last_valid_block_height:Some(100),expire_at_ms:valuation.quote.expires_at_ms,
                    },
                    transaction_fingerprint:"native-fixture".into(),network_fee_lamports:"7000".into(),wallet_outflow_lamports:"7000".into(),wallet_required_lamports:"897880".into(),minimum_credit_lamports:"2093000".into(),simulation_slot:12,checked_at_ms:clock,valid_until_ms:clock+4000,
                });
                m.apply_result(Ok(s));
            });
            for text in ["交易已模拟，未兑换","补仓消息网络费 0.000007 SOL","扣补仓支出后最低增加 0.002093 SOL","补仓自身支出已计入","0.00294416"] {assert!(render().contains(text),"missing {text}");}
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let plan=&mut s.plans[0];
                let mut order=StockCexOrder::intent(clock);
                order.order_id=Some("900719925474099333".into());order.phase=StockCexOrderPhase::Filled;
                order.executed_quantity=Some("0.02".into());order.executed_quote_quantity=Some("12.02".into());
                order.fills=vec![StockCexFill{trade_id:"9007199254740993".into(),quantity:"0.01".into(),price:"600".into(),fee:Some(StockTradeFee{asset:"USDC".into(),quantity:"0.006".into()})},StockCexFill{trade_id:"9007199254740994".into(),quantity:"0.01".into(),price:"602".into(),fee:None}];
                order.recheck=StockOrderRecheck{attempts:6,paused:true,next_at_ms:None};
                plan.cex_order=Some(order);plan.phase=StockPlanPhase::SubmissionUnknown;plan.revision+=1;m.apply_result(Ok(s));
            });
            let pending_order=render();
            for text in ["核对原订单","交易所已成交 · 明细/费用待核","自动核对已暂停","扣费后到账待核实，未计为套利收益","费用待核实"] {assert!(pending_order.contains(text),"missing {text}");}
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let plan=&mut s.plans[0];
                let order=plan.cex_order.as_mut().unwrap();order.fills[1].fee=Some(StockTradeFee{asset:plan.request.asset.clone(),quantity:"0.00001".into()});order.recheck.paused=false;plan.revision+=1;m.apply_result(Ok(s));
            });
            let settled=render();
            for text in ["交易所已成交 · 收支已核对","12.014","-0.02001","链上腿未完成 · 资金仍保留占用","900719925474099333"] {assert!(settled.contains(text),"missing {text}");}
            assert!(!settled.contains("扣费后到账待核实，未计为套利收益"));
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let mut plan=s.plans[0].clone();
                plan.plan_id="stock-plan-local-rfq-acceptance-0002".into();plan.request.request_id="local-rfq-plan-0002".into();plan.cex_order=None;
                let mut r=s.rfqs[0].clone();r.candidate=None;r.phase=StockRfqPhase::AwaitingQuotes;r.request.quantity="0.02".into();
                r.acceptance=Some(StockRfqAcceptance{plan_id:plan.plan_id.clone(),quote_id:"9007199254740997".into(),taker_price:"600".into(),submitted_at_ms:clock,acknowledged:false,rejected:false,evidence_conflict:false});
                plan.terms.cex_instruction=Some(StockCexInstruction::AcceptRfq{rfq_id:r.rfq_id.clone().unwrap(),quote_id:"9007199254740997".into(),symbol:r.symbol.clone(),side:r.request.side,quantity:r.request.quantity.clone(),taker_price:"600".into()});
                plan.rfq_acceptance=Some(r.clone());s.rfqs[0]=r;s.plans.push(plan);m.apply_result(Ok(s));
            });
            let unknown=render();
            for text in ["接受结果待确认 · 不重发","核对原 RFQ","RFQ 接受与结算回执","已提交的报价"] {assert!(unknown.contains(text),"missing {text}");}
            data.market.update(|m|{let mut s=m.value().unwrap().clone();let p=&mut s.plans[1];let r=p.rfq_acceptance.as_mut().unwrap();r.acceptance.as_mut().unwrap().acknowledged=true;r.phase=StockRfqPhase::AcceptedBinding;s.rfqs[0]=r.clone();p.revision+=1;m.apply_result(Ok(s));});
            assert!(render().contains("已锁资 · 待结算"));
            data.market.update(|m|{let mut s=m.value().unwrap().clone();let p=&mut s.plans[1];let r=p.rfq_acceptance.as_mut().unwrap();r.phase=StockRfqPhase::Filled;r.executed_quantity=Some("0.02".into());r.executed_quote_quantity=Some("12.02".into());r.fills=vec![StockRfqFill{quote_id:"9007199254740997".into(),quantity:"0.02".into(),quote_quantity:"12.02".into(),price:"601".into()}];s.rfqs[0]=r.clone();p.revision+=1;m.apply_result(Ok(s));});
            let complete=render();assert!(complete.contains("成交数量已核对 · 费用与净到账待核实"));
            data.market.update(|m|{
                let mut s=m.value().unwrap().clone();let mut p=s.plans[0].clone();
                p.plan_id="stock-plan-chain-receipt-0003".into();p.cex_order=None;p.rfq_acceptance=None;
                p.chain_submission=Some(StockChainSubmission{submitted_at_ms:clock,wallet_signature:"local-fixture-signature".into(),transaction_id:None,provider_transaction_id:None,provider_acknowledged:true,receipt:None,recheck_attempts:1,next_recheck_at_ms:clock+5000,search_before:None,problem:None});
                s.plans.push(p);m.apply_result(Ok(s));
            });
            let pending=render();assert!(pending.contains("Provider 已回复 · 链上结果待核对"));assert!(pending.contains("核对原链上交易"));
            data.market.update(|m|{
                let mut s=m.value().unwrap().clone();let p=&mut s.plans[2];let c=&p.terms.chain_cost;
                let row=p.chain_submission.as_mut().unwrap();row.transaction_id=Some("5owCWNSsaMc9YGjVCurXYiTwyKA3cjYLjpMWehsUYuXGDztRTdaECbHT7qJFPKWpEAj9TuFiVSNDkvUFgxqmK6RF".into());
                row.receipt=Some(StockChainReceipt{transaction_id:row.transaction_id.clone().unwrap(),slot:123456,succeeded:true,fee_payer:p.request.wallet_address.clone(),network_fee_lamports:"7000".into(),wallet_native_change_lamports:"-7000".into(),asset_changes:vec![StockChainAssetChange{mint:c.quote.input_mint.clone(),decimals:6,raw_change:"-10000000".into()},StockChainAssetChange{mint:c.mint.address.clone(),decimals:6,raw_change:"19000".into()}],within_plan:true,problems:vec![]});p.revision+=1;m.apply_result(Ok(s));
            });
            let chain=render();
            for text in ["链上已最终确认 · 原币收支已核对","-0.000007","0.019","两腿资金闭环尚未完成","网络费付款方"]{assert!(chain.contains(text),"missing {text}");}
            data.market.update(|m|{
                let mut s=m.value().unwrap().clone();let mut p=s.plans[2].clone();
                p.plan_id="stock-plan-pair-receipts-0004".into();p.two_leg_started_at_ms=Some(clock);
                let mut order=s.plans[0].cex_order.clone().unwrap();order.fills[1].fee=Some(StockTradeFee{asset:"USDC".into(),quantity:"0.00602".into()});
                p.cex_order=Some(order);s.plans.push(p);m.apply_result(Ok(s));
            });
            let paired=render();
            for text in ["核对两腿回执","双腿实际收支","USDC 净变化（含补偿与补回）","2.00798","实际资产位置","实际净扣 SOL 尚未补回","试算 SOL 补回"] {assert!(paired.contains(text),"missing {text}");}
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let mut p=s.plans[3].clone();
                p.plan_id="stock-plan-ready-settlement-0005".into();
                let c=&p.terms.chain_cost;
                let valuation=StockNativeValuation {native_lamports:"7000".into(),quote:StockDexQuote{input_mint:shared_types::stocks::comparison::SOLANA_USDC.into(),output_mint:STOCK_WRAPPED_SOL.into(),input_raw:"10000".into(),output_raw:"14000".into(),minimum_output_raw:"14000".into(),..c.quote.clone()},replenishment:None};
                let mut submission=p.chain_submission.clone().unwrap();let receipt=submission.receipt.as_mut().unwrap();
                receipt.wallet_native_change_lamports="7000".into();receipt.asset_changes=vec![StockChainAssetChange{mint:shared_types::stocks::comparison::SOLANA_USDC.into(),decimals:6,raw_change:"-10000".into()},StockChainAssetChange{mint:STOCK_WRAPPED_SOL.into(),decimals:9,raw_change:"0".into()}];
                p.native_topups=vec![StockNativeTopup{source_revision:p.revision,prepared_at_ms:clock,valuation,wallet:StockWalletEvidence{owner:p.request.wallet_address.clone(),mint:c.mint.address.clone(),stock_raw:Some("19000".into()),usdc_raw:Some("15000000".into()),sol_lamports:Some("9993000".into()),checked_at_ms:clock,problems:vec![]},submission:Some(submission)}];
                p.revision+=1;assert!(p.accounting().can_settle(),"{:?}",p.accounting());s.plans.push(p);m.apply_result(Ok(s));
            });
            let ready=render();assert!(ready.contains("结束并释放预留"));assert!(ready.contains("1.99798"));
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let mut p=s.plans[4].clone();p.plan_id="stock-plan-settled-0006".into();
                p.settlement=Some(StockPlanSettlement{source_revision:p.revision,settled_at_ms:clock,accounting:p.accounting()});
                p.phase=StockPlanPhase::Settled;p.revision+=1;s.plans.push(p);m.apply_result(Ok(s));
            });
            let finished=render();assert!(finished.contains("交易已收尾 · 预留已释放"));
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let mut p=s.plans[0].clone();
                p.plan_id="stock-plan-ready-submit-0007".into();p.phase=StockPlanPhase::Reserved;
                p.cex_order=None;p.rfq_acceptance=None;p.chain_submission=None;p.two_leg_started_at_ms=None;
                p.terms.market_valid_until_ms=clock+3000;
                let replacement=s.chain_costs[0].native_valuation.as_ref().unwrap().replenishment.clone().unwrap();
                p.terms.chain_cost=s.chain_costs[0].clone();
                p.terms.chain_cost.transaction=Some(replacement.transaction.clone());
                let sol=stock_chain_quantity(&p.terms.chain_cost.total_native_required_lamports(clock).unwrap().to_string(),9).unwrap();
                p.terms.allocations[2].quantity=sol.clone();
                p.terms.preflight_evidence.as_mut().unwrap().directions[0].inventory[2].required=Some(sol);
                p.submission_market_check(&s,clock).unwrap();
                s.plans.push(p);
                let mut p=s.plans[4].clone();p.plan_id="stock-plan-topup-submit-0008".into();
                p.native_topups[0].submission=None;p.native_topups[0].valuation.replenishment=Some(replacement);
                s.plans.push(p);m.apply_result(Ok(s));
            });
            let submitting=render();
            for text in ["提交两腿","提交 SOL 补回","确认本次实盘资金操作","两腿不是原子成交"] {assert!(submitting.contains(text),"missing {text}");}
            let pair_disabled=|html:String| {
                let section=html.split("<details class=\"stock-execution-confirmation\"").find(|s|s.contains("提交两腿")).unwrap();
                section.split("<input").nth(1).unwrap().split('>').next().unwrap().contains("disabled")
            };
            assert!(!pair_disabled(submitting));
            data.market.update(|m| {let mut s=m.value().unwrap().clone();s.books[0].update_id+=100;s.preflight=None;s.comparison.as_mut().unwrap().buy.input_raw="50000000".into();m.apply_result(Ok(s));});
            assert!(!pair_disabled(render()),"a harmless WS frame or draft quote must not disable confirmation");
            data.market.update(|m| {let mut s=m.value().unwrap().clone();s.books[0].bid=Some("599".into());m.apply_result(Ok(s));});
            assert!(pair_disabled(render()));
            assert!(render().contains("当前价格已差于原限价"));
            data.market.update(|m| {let mut s=m.value().unwrap().clone();s.books[0].bid=Some("600".into());s.comparison.as_mut().unwrap().buy.input_raw="10000000".into();m.apply_result(Ok(s));});
            assert!(!pair_disabled(render()));
            data.clock.set(clock+5000);assert!(render().contains("报价已过期，请重新构建"));data.clock.set(clock);
            data.market.update(|m| {
                let mut s=m.value().unwrap().clone();let mut p=s.plans[3].clone();
                p.plan_id="stock-plan-recovery-ready-0009".into();p.revision+=1;
                let receipt=p.chain_submission.as_mut().unwrap().receipt.as_mut().unwrap();
                receipt.succeeded=false;receipt.within_plan=false;receipt.problems=vec!["原链上交易已失败".into()];
                for c in &mut receipt.asset_changes {c.raw_change="0".into();}
                let target=p.recovery_target().unwrap();
                let mut cost=p.terms.chain_cost.clone();cost.direction=target.direction;cost.valid_until_ms=clock+4000;
                p.recoveries=vec![StockRecovery{source_revision:p.revision,prepared_at_ms:clock,max_loss_usdc:"1.5".into(),target,cost,
                    wallet:s.plans[4].native_topups[0].wallet.clone(),minimum_net_usdc:"-0.52".into(),cancelled_at_ms:None,submission:None}];
                s.plans.push(p.clone());
                p.plan_id="stock-plan-recovery-unresolved-0010".into();p.revision+=1;
                let mut submission=p.chain_submission.clone().unwrap();submission.receipt=None;submission.transaction_id=None;
                submission.problem=Some("尚未找到原补偿回执，保留占用，不重复发送".into());
                p.recoveries[0].submission=Some(submission);s.plans.push(p);m.apply_result(Ok(s));
            });
            let recovering=render();
            for text in ["股票差额处置","整笔损失上限 / USDC","试算补偿","提交补偿","取消本次补偿","核对原补偿交易","整笔保守净变化 / USDC","-0.52"]{assert!(recovering.contains(text),"missing {text}");}
            data.market.update(|m|{let mut s=m.value().unwrap().clone();add_funding_fixture(&mut s,clock);m.apply_result(Ok(s));});
            let funding=render();
            for text in ["股票补库路径","Solana · USDC 缺","来源可调数量足够 · 尚未转账","扣除本次备款后可调","补库保守备款 / 原币","充提限制与待办","最低充值 / 提现","官方账户地址 · Solana"] {
                assert!(funding.contains(text),"missing {text}");
            }
            let previous=data.market.get_untracked();
            let previous_wallet=data.preflight.wallet.get_untracked();
            let (mut snapshot,plan)=super::funding::tests::fixture(clock);
            data.preflight.wallet.set(plan.request.wallet_address.clone());
            data.market.set(LoadState::Ready(snapshot.clone()));
            let funding_buttons=|html:String|html.split("<button").filter_map(|s|s.split_once("</button>").map(|(b,_)|b)).filter(|b|b.contains("保存补库计划")).map(|b|b.contains("disabled")).collect::<Vec<_>>();
            assert_eq!(funding_buttons(render()),vec![false]);
            snapshot.funding_plans=vec![plan.clone()];data.market.set(LoadState::Ready(snapshot.clone()));
            assert_eq!(funding_buttons(render()),vec![true],"active local reserve blocks another prepare");
            snapshot.security=None;snapshot.preflight=None;data.market.set(LoadState::Ready(snapshot));
            assert!(render().contains("取消补库预留"),"restored funding history is visible with no selected stock");
            data.market.set(previous);data.preflight.wallet.set(previous_wallet);
            data.market.update(|m|{let mut s=m.value().unwrap().clone();
                let mut cancelled=plan.clone();cancelled.plan_id="stock-funding-local-visual-cancelled-0002".into();cancelled.phase=StockFundingPlanPhase::Cancelled;cancelled.revision=2;
                let mut expired=plan.clone();expired.plan_id="stock-funding-local-visual-expired-0003".into();expired.terms.valid_until_ms=clock-1;
                let mut sending=plan.clone();sending.plan_id="stock-funding-local-visual-withdrawing-0004".into();sending.revision=2;sending.phase=StockFundingPlanPhase::Withdrawing;
                sending.withdrawal=Some(StockFundingWithdrawal{client_id:"bp-stock-funding-local-visual-withdrawing-0004".into(),submitted_at_ms:clock-6000,query_count:1,last_query_at_ms:Some(clock-6000),remote:None,receipt:None,problem:Some("回复未确认，只查询原提现，不重复提交".into()),evidence_conflict:None});
                let mut received=sending.clone();received.plan_id="stock-funding-local-visual-received-0005".into();received.revision=4;received.phase=StockFundingPlanPhase::Received;
                let w=received.withdrawal.as_mut().unwrap();w.client_id="bp-stock-funding-local-visual-received-0005".into();
                w.remote=Some(StockWithdrawalRecord{id:43,client_id:w.client_id.clone(),blockchain:"Solana".into(),symbol:"USDC".into(),to_address:plan.terms.destination.clone(),quantity:"10.5".into(),fee:Some("0.5".into()),status:"confirmed".into(),transaction_hash:Some("local-signature-fixture".into()),created_at:"2026-09-16T00:00:00Z".into(),is_internal:false});
                w.receipt=Some(StockFundingReceipt{transaction_hash:"local-signature-fixture".into(),destination:plan.terms.destination.clone(),mint:shared_types::stocks::comparison::SOLANA_USDC.into(),decimals:6,credited_raw:"10000000".into(),slot:123457,block_time_ms:clock-5000,network_fee_lamports:5000,fee_payer:plan.terms.destination.clone(),checked_at_ms:clock});
                w.problem=Some("链上已到账；交易所扣账尚未核清，保留占用".into());
                let mut inbound=plan.clone();inbound.plan_id="stock-funding-visual-inbound-0006".into();inbound.request.target=StockFundingTarget::Backpack;
                inbound.terms.need.source="Solana".into();inbound.terms.need.target="Backpack".into();inbound.terms.withdrawal_capacity=None;
                inbound.terms.quantity="10".into();inbound.terms.source_budget="10".into();inbound.terms.destination="cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN".into();
                let mut prepared=inbound.clone();prepared.plan_id="stock-funding-visual-prepared-0007".into();prepared.revision=2;
                prepared.transfer=Some(StockFundingTransfer{preparation:StockFundingTransferPreparation{
                    transaction_base64:"unsigned-render-fixture".into(),blockhash:"local-blockhash".into(),last_valid_block_height:400,
                    source_token_account:None,destination_token_account:None,token_program:None,account_creation:Some(StockFundingAccountCreation{account_size:165,rent_budget_lamports:2039280}),network_fee_lamports:5000,retained_sol_lamports:890880,slot:123457,prepared_at_ms:clock-1000,
                },submitted_at_ms:None,transaction_hash:None,acknowledged:false,query_count:0,last_query_at_ms:None,receipt:None,deposit:None,problem:None});
                let mut submitted=prepared.clone();submitted.plan_id="stock-funding-visual-submitted-0008".into();submitted.revision=3;submitted.phase=StockFundingPlanPhase::Transferring;
                let t=submitted.transfer.as_mut().unwrap();t.submitted_at_ms=Some(clock-6000);t.transaction_hash=Some("local-stock-transfer-signature".into());t.query_count=1;t.last_query_at_ms=Some(clock-6000);t.problem=Some("广播回复未确认，只查询原交易，不会再次发送".into());
                let mut pending=submitted.clone();pending.plan_id="stock-funding-visual-pending-0009".into();pending.revision=4;pending.phase=StockFundingPlanPhase::DepositPending;
                let t=pending.transfer.as_mut().unwrap();t.receipt=Some(StockFundingTransferReceipt{transaction_hash:"local-stock-transfer-signature".into(),succeeded:true,within_plan:true,source_debit_raw:10000000,destination_credit_raw:10000000,wallet_debit_lamports:2044280,network_fee_lamports:5000,account_creation_lamports:2039280,slot:123458,block_time_ms:clock-5000,checked_at_ms:clock});t.problem=Some("链上已转出，Backpack 入账记录尚未出现；只查询原交易".into());
                let mut deposited=pending.clone();deposited.plan_id="stock-funding-visual-deposited-0010".into();deposited.revision=5;deposited.phase=StockFundingPlanPhase::Deposited;
                let t=deposited.transfer.as_mut().unwrap();t.deposit=Some(StockFundingDepositRecord{id:17,source:"solana".into(),status:"confirmed".into(),symbol:"USDC".into(),quantity:"10".into(),created_at:"2026-09-16T00:00:00Z".into(),transaction_hash:"local-stock-transfer-signature".into(),from_address:None,to_address:None});t.problem=Some("Backpack 已确认原转账入账；下一笔交易仍须重新预检".into());
                let mut failed=pending.clone();failed.plan_id="stock-funding-visual-failed-0011".into();failed.phase=StockFundingPlanPhase::TransferFailed;
                let t=failed.transfer.as_mut().unwrap();let r=t.receipt.as_mut().unwrap();r.succeeded=false;r.source_debit_raw=0;r.destination_credit_raw=0;r.account_creation_lamports=0;r.wallet_debit_lamports=5000;t.problem=Some("原链上交易失败，实际网络费已记录；转账金额未扣除，原计划不重发".into());
                s.funding_plans=vec![plan,cancelled,expired,sending,received,inbound,prepared,submitted,pending,deposited,failed];m.apply_result(Ok(s));
            });
            let saved=render();
            for text in ["已预留 · 未转账","已取消 · 未转账","预留已到期 · 未转账","账户不借款可提上限","10.5 USDC","11 USDC","提交本次提现","查询原提现与到账","链上已到账 · 扣账待核清","10000000","核验期间保留"]{assert!(saved.contains(text),"missing {text}");}
            let states=saved.split("<button").filter_map(|s|s.split_once("</button>").map(|(b,_)|b)).filter(|b|b.contains("取消补库预留")).map(|b|b.contains("disabled")).collect::<Vec<_>>();
            assert_eq!(states,vec![false,true,true,true,true,false,false,true,true,true,true]);
            for text in ["核算 Solana 转账与费用","确认转入 Backpack","0.000005000","提交本次链上转账","核对原转账与 Backpack 入账","链上已转出 · Backpack 入账待核验","Backpack 已确认入账","实际网络费 / lamports","链上失败 · 已记录实际网络费"]{assert!(saved.contains(text),"missing {text}");}
            for text in ["接收账户创建上限 / SOL","0.002039280","实际接收账户支出 / lamports","2044280","不计作可退回备款"]{assert!(saved.contains(text),"missing {text}");}
            let transfer_buttons=saved.split("<button").filter_map(|s|s.split_once("</button>").map(|(b,_)|b)).filter(|b|b.contains("提交本次链上转账")).collect::<Vec<_>>();
            assert_eq!(transfer_buttons.len(),1);assert!(transfer_buttons[0].contains("disabled"),"chain transfer needs separate unchecked confirmation");
            saved
        } else { html };
        if let Ok(path)=std::env::var("STOCK_RENDER_PATH") {
            write_stock_html(&path,&html);
        }
        if let Ok(path)=std::env::var("STOCK_PUBLIC_RENDER_PATH") {
            let input=std::env::var("STOCK_PUBLIC_CAPTURE_PATH").expect("public snapshot path required");
            let snapshot:StockMarketSnapshot=serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
            assert!(snapshot.preflight.is_none() && snapshot.plans.is_empty() && snapshot.rfqs.is_empty(),"public probe must not contain private account or order state");
            let security=snapshot.security.clone().unwrap();
            let comparison=snapshot.comparison.as_ref().unwrap();
            let contract=comparison.mint.address.clone();
            data.clock.set(snapshot.observed_at_ms);
            data.budget.set(comparison.budget_usdc.clone());
            data.keyed.set(comparison.keyed);
            data.catalog.set(LoadState::Ready(StockCatalog{rows:vec![security.clone()],observed_at_ms:snapshot.observed_at_ms}));
            data.market.set(LoadState::Ready(snapshot));
            let public=render();
            for text in [security.asset.as_str(),contract.as_str(),"Jupiter · 只读"] {
                assert!(public.contains(text),"missing public evidence {text}");
            }
            write_stock_html(&path,&public);
        }
    });
}

pub(super) fn write_stock_html(path: &str, html: &str) {
    let css_path = std::env::var("STOCK_RENDER_CSS_PATH").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/preview.css").into()
    });
    let css = std::fs::read_to_string(css_path)
        .expect("compile Tailwind CSS before exporting a stock preview");
    assert!(
        !css.contains("@tailwind "),
        "uncompiled Tailwind input cannot verify the real theme"
    );
    std::fs::write(path,format!("<!doctype html><html lang=zh-CN class=dark><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>股票模块离线采样验证</title><style>{css}</style><body class=\"bg-ink-950 text-slate-100 antialiased\">{html}</body></html>")).unwrap();
}

fn add_funding_fixture(s: &mut StockMarketSnapshot, now: i64) {
    let c = s.comparison.as_ref().unwrap();
    let asset = c.asset.clone();
    let balance = |n: &str| StockAccountBalance {
        available: n.into(),
        locked: "0".into(),
        staked: "0".into(),
        observed_at_ms: now,
        source_at_us: None,
    };
    let account = StockAccountEvidence {
        fingerprint: "local-funding-fixture".into(),
        spot_maker_fee_bps: "8".into(),
        spot_taker_fee_bps: "10".into(),
        liquidating: false,
        fees_at_ms: now,
        balances_at_ms: now,
        balances: [
            (asset.clone(), balance("2")),
            ("USDC".into(), balance("12")),
            ("SOL".into(), balance("1")),
        ]
        .into_iter()
        .collect(),
    };
    let owner = "Hc2D2As4vz9DZVYd3jJMCkiDEKjbUc1W8cf8vrGFfULz";
    let wallet = StockWalletEvidence {
        owner: owner.into(),
        mint: c.mint.address.clone(),
        stock_raw: Some("0".into()),
        usdc_raw: Some("0".into()),
        sol_lamports: Some("0".into()),
        checked_at_ms: now,
        problems: vec![],
    };
    let token = |mint: &str, decimals, deposit: &str, withdraw: &str, fee: &str| StockChainToken {
        blockchain: "Solana".into(),
        contract_address: Some(mint.into()),
        native_decimals: Some(decimals),
        deposit_enabled: Some(true),
        withdraw_enabled: Some(true),
        minimum_deposit: Some(deposit.into()),
        minimum_withdrawal: Some(withdraw.into()),
        maximum_withdrawal: None,
        withdrawal_fee: Some(fee.into()),
    };
    s.funding_assets = vec![
        StockFundingAsset {
            asset: "USDC".into(),
            tokens: vec![token(
                shared_types::stocks::comparison::SOLANA_USDC,
                6,
                "0.5",
                "1",
                "0.5",
            )],
        },
        StockFundingAsset {
            asset: "SOL".into(),
            tokens: vec![token("So1", 9, "0.006", "0.012", "0.006")],
        },
    ];
    s.token_metadata_at_ms = Some(now);
    s.token_metadata_problem = None;
    let directions = evaluate_preflight(s, Some(&account), Some(&wallet), now);
    let funding = shared_types::stocks::funding::evaluate_funding(
        s,
        &directions,
        Some(&account),
        Some(&wallet),
        now,
    );
    s.preflight = Some(StockPreflight {
        asset: asset.clone(),
        wallet_address: Some(owner.into()),
        checked_at_ms: now,
        valid_until_ms: now + 5000,
        price_basis: StockPriceBasis::from_snapshot(s),
        spot_taker_fee_pct: Some("0.1".into()),
        account_at_ms: Some(now),
        wallet_at_ms: Some(now),
        directions,
        funding,
        problems: vec![],
    });
    s.deposit_address = Some(StockDepositAddress {
        asset,
        address: owner.into(),
        blockchain: "Solana".into(),
        account_fingerprint: account.fingerprint,
        checked_at_ms: now,
    });
}

fn add_rfq_fixture(snapshot: &mut StockMarketSnapshot, now: i64) {
    let Some(security) = snapshot.security.as_ref() else {
        return;
    };
    snapshot.rfq_connected = true;
    snapshot.rfq_problem = None;
    snapshot.rfqs = vec![StockRfq {
        request: StockRfqRequest {
            request_id: "local-fixture-stock-rfq-001".into(),
            asset: security.asset.clone(),
            side: StockRfqSide::Ask,
            quantity: "1".into(),
        },
        client_id: 123,
        account_fingerprint: "local-fixture".into(),
        symbol: security.rfq_symbol.clone(),
        rfq_id: Some("9007199254740993".into()),
        phase: StockRfqPhase::Candidate,
        candidate: Some(StockRfqCandidate {
            quote_id: "9007199254740997".into(),
            taker_price: "101.05".into(),
            source_at_us: now * 1000,
            received_at_ms: now,
        }),
        submission_time_ms: Some(now),
        expiry_time_ms: Some(now + 60_000),
        source_at_us: Some(now * 1000),
        fill_price: None,
        executed_quantity: None,
        executed_quote_quantity: None,
        fills: vec![],
        settlement: Default::default(),
        acceptance: None,
        needs_recheck: false,
        cancel_requested: false,
        created_at_ms: now,
        updated_at_ms: now,
        problem: None,
    }];
}

#[test]
fn stock_rfq_labels_separate_quote_window_binding_and_actual_fill() {
    let mut s = StockMarketSnapshot {
        security: Some(StockSecurity {
            asset: "MU.US".into(),
            ticker: "MU".into(),
            name: "Micron".into(),
            cusip: None,
            sessions: vec![],
            order_books: vec![],
            rfq_symbol: "MU.US_USDC_RFQ".into(),
        }),
        ..Default::default()
    };
    add_rfq_fixture(&mut s, 1000);
    let r = &mut s.rfqs[0];
    assert_eq!(rfq::phase_label(r, 1000), "已收到报价");
    assert_eq!(rfq::phase_label(r, 61_000), "报价已过期");
    r.needs_recheck = true;
    assert_eq!(rfq::phase_label(r, 1000), "报价待核对");
    r.phase = StockRfqPhase::AcceptedBinding;
    assert_eq!(rfq::phase_label(r, 1000), "已锁资 · 待结算");
    r.phase = StockRfqPhase::Filled;
    assert_eq!(rfq::phase_label(r, 1000), "已成交 · 金额待核");
    assert!(r.executed_quantity.is_none());
    r.settlement.paused = true;
    assert_eq!(rfq::phase_label(r, 1000), "已成交 · 核验已暂停");
    r.fills = vec![StockRfqFill {
        quote_id: "9007199254740997".into(),
        quantity: "1".into(),
        quote_quantity: "101.05".into(),
        price: "101.05".into(),
    }];
    r.executed_quantity = Some("1".into());
    r.executed_quote_quantity = Some("101.05".into());
    assert_eq!(rfq::phase_label(r, 1000), "成交额已核 · 费用待核");
    assert!(rfq::next_step_label(r)
        .unwrap()
        .contains("尚不能确认净利润"));
    r.phase = StockRfqPhase::NotSent;
    assert_eq!(rfq::phase_label(r, 1000), "询价未发送");
    r.phase = StockRfqPhase::Rejected;
    assert_eq!(rfq::phase_label(r, 1000), "询价被拒绝");
    r.phase = StockRfqPhase::AwaitingQuotes;
    r.acceptance = Some(StockRfqAcceptance {
        plan_id: "local-plan".into(),
        quote_id: "9007199254740997".into(),
        taker_price: "101.05".into(),
        submitted_at_ms: 1000,
        acknowledged: false,
        rejected: false,
        evidence_conflict: false,
    });
    assert_eq!(rfq::phase_label(r, 1000), "接受结果待确认 · 不重发");
    r.acceptance.as_mut().unwrap().acknowledged = true;
    assert_eq!(rfq::phase_label(r, 1000), "接受已回执 · 结算待确认");
    r.acceptance.as_mut().unwrap().evidence_conflict = true;
    assert_eq!(rfq::phase_label(r, 1000), "回执冲突 · 停止自动处理");
    assert!(rfq::next_step_label(r).unwrap().contains("不重复提交"));
    r.acceptance.as_mut().unwrap().evidence_conflict = false;
    r.phase = StockRfqPhase::Cancelled;
    assert_eq!(rfq::phase_label(r, 1000), "结算已取消 · 另一腿待核");
    assert!(rfq::next_step_label(r)
        .unwrap()
        .contains("不代表双边已撤销"));
    if let Ok(path) = std::env::var("STOCK_BP_RFQ_CONFLICT_CAPTURE_PATH") {
        let plan: StockExecutionPlan =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let receipt = plan.rfq_acceptance.as_ref().unwrap();
        assert!(receipt.acceptance.as_ref().unwrap().evidence_conflict);
        let html = Owner::new().with(|| {
            view! {
                <section class="stock-section stock-comparison">{plans::rfq_receipt(&plan)}</section>
            }
            .to_html()
        });
        for text in [
            "回执冲突 · 停止自动处理",
            "不重复提交",
            "待核实，未计为套利收益",
            receipt.executed_quantity.as_deref().unwrap(),
            receipt.executed_quote_quantity.as_deref().unwrap(),
        ] {
            assert!(html.contains(text), "missing {text}");
        }
        assert!(!html.contains("预留已释放"));
        if let Ok(path) = std::env::var("STOCK_BP_RFQ_CONFLICT_RENDER_PATH") {
            write_stock_html(&path, &html);
        }
    }
}

fn add_preflight_fixture(snapshot: &mut StockMarketSnapshot, now: i64) {
    let Some(comparison) = snapshot.comparison.as_ref() else {
        return;
    };
    let asset = comparison.asset.clone();
    let owner = "11111111111111111111111111111111";
    let balance = |amount: &str| StockAccountBalance {
        available: amount.into(),
        locked: "0".into(),
        staked: "0".into(),
        observed_at_ms: now,
        source_at_us: None,
    };
    let account = StockAccountEvidence {
        fingerprint: "local-visual-fixture".into(),
        spot_maker_fee_bps: "8".into(),
        spot_taker_fee_bps: "10".into(),
        liquidating: false,
        fees_at_ms: now,
        balances_at_ms: now,
        balances: [
            (asset.clone(), balance("2")),
            ("USDC".into(), balance("1000")),
        ]
        .into_iter()
        .collect(),
    };
    let wallet = StockWalletEvidence {
        owner: owner.into(),
        mint: comparison.mint.address.clone(),
        stock_raw: Some(10u64.pow(u32::from(comparison.mint.decimals)).to_string()),
        usdc_raw: Some("25000000".into()),
        sol_lamports: Some("100000000".into()),
        checked_at_ms: now,
        problems: vec![],
    };
    snapshot.chain_costs = [StockChainDirection::Buy, StockChainDirection::Sell]
        .into_iter()
        .filter_map(|direction| {
            let quote = direction.quote(comparison)?.clone();
            Some(StockChainCost {
                transaction: None,
                asset: asset.clone(),
                direction,
                wallet_address: owner.into(),
                mint: comparison.mint.clone(),
                quote,
                transaction_fingerprint: "local-visual-fixture".into(),
                checked_at_ms: now,
                valid_until_ms: if direction == StockChainDirection::Buy {
                    now + 5000
                } else {
                    now - 1
                },
                provider_fees: vec![
                    StockNativeFee {
                        kind: "signature".into(),
                        lamports: Some("5000".into()),
                        payer: Some(owner.into()),
                    },
                    StockNativeFee {
                        kind: "priority".into(),
                        lamports: Some("2000".into()),
                        payer: Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into()),
                    },
                    StockNativeFee {
                        kind: "rent".into(),
                        lamports: Some("2039280".into()),
                        payer: Some(owner.into()),
                    },
                ],
                network_fee_lamports: Some("7000".into()),
                wallet_debit_lamports: Some("2046280".into()),
                wallet_budget_lamports: Some("2046280".into()),
                wallet_required_lamports: Some("2937160".into()),
                native_valuation: Some(StockNativeValuation {
                    replenishment: None,
                    native_lamports: "2046280".into(),
                    quote: StockDexQuote {
                        input_mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
                        output_mint: STOCK_WRAPPED_SOL.into(),
                        input_raw: "215000".into(),
                        output_raw: "2200000".into(),
                        minimum_output_raw: "2100000".into(),
                        router: "metis".into(),
                        fee_bps: Some(2),
                        fee_mint: Some(STOCK_WRAPPED_SOL.into()),
                        requested_at_ms: now - 1000,
                        received_at_ms: now - 500,
                        expires_at_ms: Some(now + 4000),
                    },
                }),
                simulation_slot: Some(comparison.mint.slot),
                simulation_passed: true,
                problems: vec![],
            })
        })
        .collect();
    snapshot.preflight = Some(StockPreflight {
        funding: vec![],
        asset,
        wallet_address: Some(owner.into()),
        checked_at_ms: now,
        valid_until_ms: now + 5000,
        price_basis: StockPriceBasis::from_snapshot(snapshot),
        spot_taker_fee_pct: Some("0.1".into()),
        account_at_ms: Some(now),
        wallet_at_ms: Some(now),
        directions: evaluate_preflight(snapshot, Some(&account), Some(&wallet), now),
        problems: vec![],
    });
}

fn add_plan_fixture(snapshot: &mut StockMarketSnapshot, now: i64) {
    let security = snapshot.security.as_ref().unwrap();
    let report = snapshot.preflight.as_ref().unwrap();
    snapshot.plans = vec![StockExecutionPlan {
        native_topups: vec![],
        recoveries: vec![],
        settlement: None,
        chain_submission: None,
        two_leg_started_at_ms: None,
        plan_id: "stock-plan-local-visual-fixture-0001".into(),
        request: StockPlanRequest {
            request_id: "local-visual-stock-plan-0001".into(),
            asset: security.asset.clone(),
            direction: StockChainDirection::Buy,
            wallet_address: report.wallet_address.clone().unwrap(),
            preflight_at_ms: now,
            build: None,
        },
        terms: StockPlanTerms {
            preflight_evidence: Some(report.clone()),
            cex_fee_budget: StockCexFeeBudget::calculate(
                "12",
                StockRfqSide::Ask,
                StockCexFeeBasis::OrderBookQuote {
                    taker_bps: "10".into(),
                    observed_at_ms: now,
                },
            ),
            account_fingerprint: "local-visual-fixture".into(),
            security: security.clone(),
            chain_cost: snapshot.chain_costs[0].clone(),
            route: snapshot.trading_route.clone().unwrap(),
            cex_shares: "0.02".into(),
            cex_notional_usdc: "12".into(),
            rfq: None,
            allocations: report.directions[0]
                .inventory
                .iter()
                .map(|i| StockPlanAllocation {
                    location: i.location.clone(),
                    asset: i.asset.clone(),
                    quantity: i.required.clone().unwrap_or_else(|| "0.02".into()),
                    available_at_reservation: i.available.clone().unwrap_or_else(|| "25".into()),
                })
                .collect(),
            after_known_costs_usdc: "1.773".into(),
            created_at_ms: now,
            market_valid_until_ms: now + 3000,
            reserved_until_ms: now + 60_000,
            cex_instruction: Some(StockCexInstruction::OrderBook {
                client_id: 12345,
                symbol: security.order_books[0].symbol.clone(),
                side: StockRfqSide::Ask,
                quantity: "0.02".into(),
                limit_price: "600".into(),
            }),
        },
        phase: StockPlanPhase::Reserved,
        revision: 1,
        updated_at_ms: now,
        cex_order: None,
        rfq_acceptance: None,
    }];
}
