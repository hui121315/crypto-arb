use super::*;
use axum::{routing::get, Router};
use shared_types::WebhookConfigPatch;
use std::{path::Path, sync::atomic::AtomicUsize};

fn fixture(now: i64) -> StockMarketSnapshot {
    let mut s = comparison::tests::snapshot();
    let mut c = comparison::tests::comparison();
    c.mint.checked_at_ms = now;
    c.mint.chain_time_ms = now;
    c.buy.requested_at_ms = now;
    c.buy.received_at_ms = now;
    c.sell = Some(StockDexQuote {
        input_mint: c.mint.address.clone(),
        output_mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
        input_raw: "16000".into(),
        output_raw: "13100000".into(),
        minimum_output_raw: "13000000".into(),
        ..c.buy.clone()
    });
    s.connected = true;
    s.comparison = Some(c);
    s.token_metadata_at_ms = Some(now);
    s.books = vec![StockBookQuote {
        symbol: "MU.US_USDC".into(),
        bid: Some("600".into()),
        ask: Some("601".into()),
        bid_quantity: Some("10".into()),
        ask_quantity: Some("10".into()),
        update_id: 1,
        source_at_ms: now,
        received_at_ms: now,
    }];
    s.monitor = StockMonitorStatus {
        enabled: true,
        alerts: StockAlertConfig {
            enabled: true,
            ..Default::default()
        },
        request: Some(StockQuoteRequest {
            asset: "MU.US".into(),
            budget_usdc: "10".into(),
            keyed: false,
        }),
        ..Default::default()
    };
    s
}

async fn dispatcher(path: &Path) -> Arc<webhook::WebhookDispatcher> {
    let w = Arc::new(
        webhook::WebhookDispatcher::initialize(Some(path.to_owned()))
            .await
            .unwrap(),
    );
    w.bootstrap_event_ids(vec![]).await.unwrap();
    w.update_config(WebhookConfigPatch {
        enabled: Some(true),
        url: Some("https://example.com/fixture-only-never-deliver".into()),
        secret: Some("local-fixture".into()),
        event_kinds: Some(vec![WebhookEventKind::StockSpread]),
        ..Default::default()
    })
    .unwrap();
    w
}

pub(in crate::services::backpack_stocks) fn peer_service(now: i64) -> BackpackStocks {
    use crate::services::{
        instrument_registry::InstrumentRegistry,
        market_data::{MarketDataCache, MarketSource},
    };
    use shared_types::{SpotTick, VenueInstrument};
    let registry = Arc::new(InstrumentRegistry::default());
    let spec: VenueInstrument = serde_json::from_value(serde_json::json!({
        "venue":"kraken","nativeSymbol":"MUx/USD","canonicalSymbol":"MUX","displaySymbol":"MUx/USD",
        "assetClass":"equity","productType":"spot","quoteAsset":"USD","settleAsset":"USD",
        "executionSupported":false,"priceTick":0.01,"qtyStep":0.000001,"minQty":0.001,"minNotional":0.5,
        "listingStatus":"trading","source":"official_endpoint","checkedAtMs":now,
        "sourceUrl":"https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument",
        "schemaVersion":"kraken-spot-ws-v2-instrument-2026-08-06"
    })).unwrap();
    registry.upsert(spec).unwrap();
    let cache = Arc::new(MarketDataCache::default());
    let q = SpotTick {
        venue: "kraken".into(),
        symbol: "MUX/USD".into(),
        bid: "600".parse().unwrap(),
        ask: "601".parse().unwrap(),
        last: "600".parse().unwrap(),
        bid_size: Some(10.into()),
        ask_size: Some(10.into()),
        volume_24h: 0.into(),
        exchange_ts_ms: Some(now),
        received_at_ms: now,
    };
    let mut fx = q.clone();
    fx.symbol = "USDC/USD".into();
    fx.bid = "0.999".parse().unwrap();
    fx.ask = "1.001".parse().unwrap();
    fx.bid_size = Some(1000.into());
    fx.ask_size = Some(1000.into());
    cache.store_spot_ticks(&[q.clone(), fx], MarketSource::WsPush);
    let s = BackpackStocks::new()
        .unwrap()
        .with_peer_markets(registry, cache);
    *s.snapshot.write() = fixture(now);
    s.watch_peer(
        StockPeerWatchRequest {
            asset: "MU.US".into(),
            selection: Some(StockPeerSelection {
                venue: "kraken".into(),
                product: StockPeerProduct::Spot,
                native_symbol: "MUx/USD".into(),
            }),
        },
        &realtime::WsHub::new(8),
    )
    .unwrap();
    s.refresh_peer(now);
    s
}

#[test]
fn stock_peer_alert_requires_opt_in_exact_fresh_pair_and_keeps_costs_unknown() {
    let now = common::time::now_ms();
    let service = peer_service(now);
    let mut s = service.snapshot();
    assert!(s.peer.as_ref().unwrap().share_unit_verified);
    assert_eq!(candidates(&s, now).unwrap().len(), 2);
    s.monitor.alerts.include_peer = true;
    let all = candidates(&s, now).unwrap();
    assert_eq!(all.len(), 4);
    for c in all.iter().filter(|c| c.peer.is_some()) {
        let payload = observation(&s, c, now);
        assert_eq!(payload["classification"], "stock_peer_spread_observation");
        assert_eq!(payload["peer"]["nativeSymbol"], "MUx/USD");
        assert_eq!(payload["transfer"]["directTransferSupported"], false);
        assert!(payload["transfer"]["venueDepositEnabled"].is_null());
        assert!(payload["afterKnownCostsUsdc"].is_null());
        assert!(payload["completeNetUsdc"].is_null());
        assert!(payload["inventory"].as_array().unwrap().is_empty());
        assert_eq!(payload["executable"], false);
        assert_eq!(payload["fundAction"], false);
        assert!(c.direction().contains("kraken · MUx/USD"));
    }
    for case in 0..7 {
        let mut bad = s.clone();
        let p = bad.peer.as_mut().unwrap();
        match case {
            0 => p.quote.as_mut().unwrap().source_at_ms = Some(now - 3001),
            1 => p.quote.as_mut().unwrap().source_at_ms = None,
            2 => p.quote_conversion.as_mut().unwrap().symbol = "USDC/USDT".into(),
            3 => p.share_unit_verified = false,
            4 => p.identity.underlying_verified = false,
            5 => p.problem = Some("venue disabled".into()),
            _ => {
                p.quote.as_mut().unwrap().ask_quantity = None;
                p.quote.as_mut().unwrap().bid_quantity = None;
            }
        }
        assert!(
            candidates(&bad, now)
                .unwrap()
                .iter()
                .all(|c| c.peer.is_none()),
            "case {case}"
        );
    }
    let before = service.generation.load(Ordering::SeqCst);
    service
        .watch_peer(
            StockPeerWatchRequest {
                asset: "MU.US".into(),
                selection: None,
            },
            &realtime::WsHub::new(8),
        )
        .unwrap();
    assert!(service.generation.load(Ordering::SeqCst) > before);
    assert!(service.ensure_generation(before, "MU.US").is_err());
    assert!(service.snapshot().plans.is_empty());
}

#[tokio::test]
async fn stock_peer_alert_dedupes_independently_and_recovers_local_outbox_without_delivery() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("peer-outbox.sqlite");
    let w = dispatcher(&path).await;
    let service = peer_service(common::time::now_ms()).with_webhook(w.clone());
    service.snapshot.write().monitor.alerts.include_peer = true;
    let hub = realtime::WsHub::new(8);
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(w.status(0).await.queue_depth, 4);
    assert_eq!(service.snapshot().alerts.recent.len(), 4);
    assert_eq!(
        service
            .snapshot()
            .alerts
            .recent
            .iter()
            .filter(|r| r.direction.contains("kraken"))
            .count(),
        2
    );
    if let Ok(path) = std::env::var("STOCK_PEER_ALERT_CAPTURE_PATH") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&service.snapshot()).unwrap(),
        )
        .unwrap();
    }
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(w.status(0).await.queue_depth, 4);
    assert!(service.rfq_worker.lock().is_none());
    assert_eq!(service.account_tracking_until_ms.load(Ordering::SeqCst), 0);
    drop(service);
    drop(w);
    let w = dispatcher(&path).await;
    let service = peer_service(common::time::now_ms()).with_webhook(w.clone());
    service.snapshot.write().monitor.alerts.include_peer = true;
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(w.status(0).await.queue_depth, 4);
    assert_eq!(service.snapshot().alerts.phase, StockAlertPhase::Cooldown);
    {
        let mut snapshot = service.snapshot.write();
        let p = snapshot.peer.as_mut().unwrap();
        p.selection.native_symbol = "MUx/USDC".into();
        let i = p.instrument.as_mut().unwrap();
        i.native_symbol = "MUx/USDC".into();
        i.quote_asset = Some("USDC".into());
        i.settle_asset = Some("USDC".into());
        p.quote.as_mut().unwrap().symbol = "MUX/USDC".into();
        p.quote_conversion = None;
    }
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(
        w.status(0).await.queue_depth,
        6,
        "a different native quote has independent cooldown"
    );
    service.snapshot.write().monitor.enabled = false;
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(service.snapshot().alerts.phase, StockAlertPhase::Disabled);
    assert_eq!(w.status(0).await.queue_depth, 6);
}

pub(in crate::services::backpack_stocks) struct PeerWsFixture(pub(in crate::services::backpack_stocks) Arc<AtomicUsize>);
#[async_trait::async_trait]
impl exchange::ExchangeAdapter for PeerWsFixture {
    fn stock_account_fingerprint(&self) -> Option<String> { Some("1234567890abcdef12345678".into()) }
    async fn validate_stock_order(&self,draft:&StockPeerOrderDraft)->exchange::ExchangeResult<StockPeerOrderCheck> {
        let index=self.0.fetch_add(1,Ordering::SeqCst);
        let frame=draft.kraken_validation("fixture-token",index as u64,common::time::now_ms()).unwrap();
        assert_eq!(frame["params"]["validate"],true);assert_eq!(frame["params"]["symbol"],"MUx/USD");
        tokio::time::sleep(Duration::from_millis(25)).await;
        Ok(StockPeerOrderCheck {draft:draft.clone(),completed_at_ms:Some(common::time::now_ms()),status:StockPeerOrderCheckStatus::Passed,
            message:"交易所仅验证当次股票参数通过；未成交，不代表套利执行已就绪".into()})
    }
    async fn stock_funding_methods(&self,native:&str)->exchange::ExchangeResult<Vec<StockPeerFundingRoute>> {
        assert_eq!(native,"MUx/USD");self.0.fetch_add(1,Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(25)).await;
        let now=common::time::now_ms();
        let mut routes:Vec<StockPeerFundingRoute>=if let Ok(path)=std::env::var("STOCK_PEER_FUNDING_ADAPTER_CAPTURE_PATH") {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        } else {
            [("MUx","tokenized_asset"),("USDC","currency")].into_iter().flat_map(|(asset,class)|
                [StockPeerFundingDirection::Deposit,StockPeerFundingDirection::Withdraw].into_iter().map(move |direction| {
                    StockPeerFundingRoute{asset:asset.into(),asset_class:class.into(),direction,amount_unit:"base".into(),checked_at_ms:now,
                    source_url:"local-fixture".into(),problem:None,methods:vec![StockPeerFundingMethod{method_id:format!("{asset}-{}",direction.as_str()),network_id:"local-solana".into(),network_name:"Solana".into(),
                        contract_address:Some(if asset=="USDC"{shared_types::stocks::comparison::SOLANA_USDC}else{"different-issuer-mint"}.into()),minimum_amount:Some("0.001".into()),maximum_amount:None,
                        fees:StockPeerFundingFees{base:StockPeerFundingAmount{asset_class:class.into(),asset:asset.into(),amount:"0.01".into()},included:true,percentage:None,minimum:None,maximum:None}}]}
                })).collect()
        };
        for route in &mut routes {route.checked_at_ms=now;}
        Ok(routes)
    }
    async fn stock_cash_account(&self,native:&str)->exchange::ExchangeResult<StockPeerAccount> {
        self.0.fetch_add(1,Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(25)).await;
        Ok(StockPeerAccount{venue:"kraken".into(),native_symbol:native.into(),stock_asset:"MUx".into(),quote_asset:"USD".into(),
            stock_available:Some("0.01".into()),quote_available:Some("100".into()),usdc_available:Some("10".into()),
            stock_taker_pct:Some("0.1".into()),fx_taker_pct:Some("0.2".into()),observed_at_ms:common::time::now_ms(),sources:vec![],problems:vec![]})
    }
    fn name(&self) -> &'static str {
        "kraken"
    }
    fn normalize_symbol(&self, s: &str) -> String {
        s.into()
    }
    fn to_exchange_symbol(&self, s: &str) -> String {
        s.into()
    }
    async fn get_funding_rate(
        &self,
        _: &str,
    ) -> exchange::ExchangeResult<shared_types::FundingRateData> {
        panic!("no funding reads")
    }
    async fn get_funding_rates(
        &self,
        _: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<shared_types::FundingRateData>> {
        panic!("no funding reads")
    }
    async fn get_ticker(&self, _: &str) -> exchange::ExchangeResult<shared_types::TickerInfo> {
        panic!("no REST ticker")
    }
    async fn get_tickers(
        &self,
        _: Option<&[String]>,
    ) -> exchange::ExchangeResult<Vec<shared_types::TickerInfo>> {
        panic!("no REST tickers")
    }
    async fn get_orderbook(
        &self,
        _: &str,
        _: u32,
    ) -> exchange::ExchangeResult<shared_types::OrderBookInfo> {
        panic!("no depth reads")
    }
    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> exchange::ExchangeResult<exchange::PublicWsSnapshot<shared_types::SpotTick>> {
        assert_eq!(symbols, &["MUx/USD".to_owned(), "USDC/USD".to_owned()]);
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(exchange::PublicWsSnapshot::Pending)
    }
}

#[tokio::test]
async fn stock_peer_background_runtime_keeps_shared_ws_demand_only_with_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let w = dispatcher(&temp.path().join("background.sqlite")).await;
    let count = Arc::new(AtomicUsize::new(0));
    let aggregator = Arc::new(exchange::Aggregator::new());
    aggregator.register(Arc::new(PeerWsFixture(count.clone())));
    let s = Arc::new(
        peer_service(common::time::now_ms())
            .with_webhook(w)
            .with_peer_feed(
                aggregator,
                Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None)),
            ),
    );
    s.snapshot.write().monitor.alerts.include_peer = true;
    let hub = realtime::WsHub::new(8);
    assert_eq!(hub.subscriber_count(realtime::channels::STOCKS), 0);
    // Only a closed loopback endpoint is used; the selected peer adapter is an in-memory WS fixture.
    let task = tokio::spawn(runtime::run(
        Arc::downgrade(&s),
        hub,
        "ws://127.0.0.1:1".into(),
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    s.snapshot.write().monitor.alerts.include_peer = false;
    let n = count.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert_eq!(count.load(Ordering::SeqCst), n);
    assert!(s.rfq_worker.lock().is_none());
    assert!(s.snapshot().plans.is_empty());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn stock_peer_preflight_reads_only_selected_adapter_and_discards_changed_market() {
    let count=Arc::new(AtomicUsize::new(0));
    let aggregator=Arc::new(exchange::Aggregator::new());aggregator.register(Arc::new(PeerWsFixture(count.clone())));
    let s=Arc::new(peer_service(common::time::now_ms()).with_peer_feed(aggregator.clone(),Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None))));
    let request=StockPeerPreflightRequest{asset:"MU.US".into(),selection:s.snapshot().peer.unwrap().selection,wallet_address:None};
    let hub=realtime::WsHub::new(8);
    let result=s.peer_preflight(request.clone(),&hub).await.unwrap();assert_eq!(count.load(Ordering::SeqCst),1);
    let report=result.peer_preflight.as_ref().unwrap();assert!(report.wallet.is_none());assert!(result.preflight.is_none());
    assert_eq!(report.account.as_ref().unwrap().stock_available.as_deref(),Some("0.01"));
    assert!(s.rfq_worker.lock().is_none() && result.plans.is_empty());
    let mut alert=result.clone();alert.monitor.alerts.include_peer=true;
    let candidate=candidates(&alert,common::time::now_ms()).unwrap().into_iter().find(|c|c.peer.is_some()).unwrap();
    let payload=observation(&alert,&candidate,common::time::now_ms());
    assert_eq!(payload["inventory"][0]["sufficient"],false);assert_eq!(payload["executable"],false);assert!(payload["completeNetUsdc"].is_null());
    if let Ok(path)=std::env::var("STOCK_PEER_PREFLIGHT_CAPTURE_PATH") {std::fs::write(path,serde_json::to_vec_pretty(&result).unwrap()).unwrap();}
    let job=tokio::spawn({let s=s.clone();let request=request.clone();let hub=hub.clone();async move {s.peer_preflight(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async {while count.load(Ordering::SeqCst)<2 {tokio::task::yield_now().await;}}).await.unwrap();
    s.watch_peer(StockPeerWatchRequest{asset:"MU.US".into(),selection:None},&hub).unwrap();
    assert!(job.await.unwrap().is_err());assert!(s.snapshot().peer_preflight.is_none());
    s.watch_peer(StockPeerWatchRequest{asset:"MU.US".into(),selection:Some(request.selection.clone())},&hub).unwrap();
    let job=tokio::spawn({let s=s.clone();let hub=hub.clone();async move {s.peer_preflight(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async {while count.load(Ordering::SeqCst)<3 {tokio::task::yield_now().await;}}).await.unwrap();
    aggregator.register(Arc::new(PeerWsFixture(Arc::new(AtomicUsize::new(0)))));
    assert!(job.await.unwrap().is_err());assert!(s.snapshot().peer_preflight.is_none());
}

#[tokio::test]
async fn stock_peer_order_check_tracks_pending_cooldown_and_configuration_change_without_funds() {
    let count=Arc::new(AtomicUsize::new(0));
    let aggregator=Arc::new(exchange::Aggregator::new());aggregator.register(Arc::new(PeerWsFixture(count.clone())));
    let s=Arc::new(peer_service(common::time::now_ms()).with_peer_feed(aggregator.clone(),Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None))));
    let request=StockPeerOrderCheckRequest{asset:"MU.US".into(),selection:s.snapshot().peer.unwrap().selection,direction:StockChainDirection::Buy};
    let hub=realtime::WsHub::new(8);
    let run=tokio::spawn({let s=s.clone();let request=request.clone();let hub=hub.clone();async move {s.check_peer_order(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async{while count.load(Ordering::SeqCst)<1{tokio::task::yield_now().await;}}).await.unwrap();
    let pending=s.snapshot();assert_eq!(pending.peer_order_checks.len(),1);assert!(pending.peer_order_checks[0].completed_at_ms.is_none());
    assert!(s.check_peer_order(request.clone(),&hub).await.is_err());
    let result=run.await.unwrap().unwrap();assert_eq!(result.peer_order_checks[0].status,StockPeerOrderCheckStatus::Passed);
    assert_eq!(result.peer_order_checks[0].draft.quantity,"0.02125");assert_eq!(result.peer_order_checks[0].draft.limit_price,"600");
    assert!(!result.peer.as_ref().unwrap().instrument.as_ref().unwrap().execution_supported);
    assert!(s.check_peer_order(request.clone(),&hub).await.unwrap_err().contains("5 秒"));assert_eq!(count.load(Ordering::SeqCst),1);
    assert!(result.plans.is_empty() && result.funding_plans.is_empty() && result.peer_preflight.is_none());
    assert!(s.rfq_worker.lock().is_none());assert_eq!(s.account_tracking_until_ms.load(Ordering::SeqCst),0);
    if let Ok(path)=std::env::var("STOCK_PEER_ORDER_CAPTURE_PATH") {std::fs::write(path,serde_json::to_vec_pretty(&result).unwrap()).unwrap();}
    s.snapshot.write().peer_order_checks.clear();
    let run=tokio::spawn({let s=s.clone();let request=request.clone();let hub=hub.clone();async move {s.check_peer_order(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async{while count.load(Ordering::SeqCst)<2{tokio::task::yield_now().await;}}).await.unwrap();
    aggregator.register(Arc::new(PeerWsFixture(Arc::new(AtomicUsize::new(0)))));
    let result=run.await.unwrap().unwrap();assert_eq!(result.peer_order_checks[0].status,StockPeerOrderCheckStatus::Unknown);
    assert!(result.peer_order_checks[0].completed_at_ms.is_some());assert!(result.peer_order_checks[0].message.contains("配置已变化"));
    aggregator.register(Arc::new(PeerWsFixture(count.clone())));s.snapshot.write().peer_order_checks.clear();
    let run=tokio::spawn({let s=s.clone();let request=request.clone();let hub=hub.clone();async move {s.check_peer_order(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async{while count.load(Ordering::SeqCst)<3{tokio::task::yield_now().await;}}).await.unwrap();
    s.watch_peer(StockPeerWatchRequest{asset:request.asset,selection:None},&hub).unwrap();
    assert!(run.await.unwrap().is_err());assert!(s.snapshot().peer_order_checks.is_empty());
}

#[tokio::test]
async fn stock_peer_funding_reports_exact_contracts_to_ui_and_webhook_without_transfer() {
    let count=Arc::new(AtomicUsize::new(0));
    let aggregator=Arc::new(exchange::Aggregator::new());aggregator.register(Arc::new(PeerWsFixture(count.clone())));
    let s=Arc::new(peer_service(common::time::now_ms()).with_peer_feed(aggregator.clone(),Arc::new(crate::services::market_subscriptions::MarketSubscriptions::load(None))));
    let request=StockPeerFundingRequest{asset:"MU.US".into(),selection:s.snapshot().peer.unwrap().selection};
    let hub=realtime::WsHub::new(8);
    let result=s.peer_funding(request.clone(),&hub).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst),1);
    let f=result.peer_funding.as_ref().unwrap();assert_eq!(f.routes.len(),4);assert!(f.current(&result,common::time::now_ms()));
    assert_eq!(peer_funding_contract_matches(&result,&f.routes[0],&f.routes[0].methods[0]),Some(false));
    assert_eq!(peer_funding_contract_matches(&result,&f.routes[2],&f.routes[2].methods[0]),Some(true));
    assert!(!f.current(&result,f.checked_at_ms+60_001));
    let mut other=result.clone();other.peer.as_mut().unwrap().selection.native_symbol="MUx/USDT".into();assert!(!f.current(&other,f.checked_at_ms));
    let mut duplicate=f.clone();duplicate.routes[1]=duplicate.routes[0].clone();assert!(!duplicate.current(&result,f.checked_at_ms));
    assert!(result.preflight.is_none() && result.peer_preflight.is_none() && result.plans.is_empty() && result.funding_plans.is_empty());assert!(s.rfq_worker.lock().is_none());
    let mut alert=result.clone();alert.monitor.alerts.include_peer=true;
    let candidate=candidates(&alert,common::time::now_ms()).unwrap().into_iter().find(|c|c.peer.is_some()).unwrap();
    let payload=observation(&alert,&candidate,common::time::now_ms());
    assert_eq!(payload["peerFunding"]["routes"].as_array().unwrap().len(),4);assert_eq!(payload["transfer"]["directTransferSupported"],false);assert_eq!(payload["fundAction"],false);
    assert!(payload["message"].as_str().unwrap().contains("非当前链上合约"));
    assert!(observation(&alert,&candidate,f.checked_at_ms+60_001)["peerFunding"].is_null());
    if let Ok(path)=std::env::var("STOCK_PEER_FUNDING_CAPTURE_PATH") {std::fs::write(path,serde_json::to_vec_pretty(&result).unwrap()).unwrap();}
    let job=tokio::spawn({let s=s.clone();let request=request.clone();let hub=hub.clone();async move{s.peer_funding(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async{while count.load(Ordering::SeqCst)<2 {tokio::task::yield_now().await;}}).await.unwrap();
    s.watch_peer(StockPeerWatchRequest{asset:"MU.US".into(),selection:None},&hub).unwrap();
    assert!(job.await.unwrap().is_err());assert!(s.snapshot().peer_funding.is_none());
    s.watch_peer(StockPeerWatchRequest{asset:"MU.US".into(),selection:Some(request.selection.clone())},&hub).unwrap();
    let job=tokio::spawn({let s=s.clone();let hub=hub.clone();async move{s.peer_funding(request,&hub).await}});
    tokio::time::timeout(Duration::from_secs(1),async{while count.load(Ordering::SeqCst)<3 {tokio::task::yield_now().await;}}).await.unwrap();
    aggregator.register(Arc::new(PeerWsFixture(Arc::new(AtomicUsize::new(0)))));
    assert!(job.await.unwrap().is_err());assert!(s.snapshot().peer_funding.is_none());
}

#[test]
fn stock_alert_uses_conservative_two_sided_quotes_not_reference_or_stale_data() {
    let s = fixture(1000);
    let rows = candidates(&s, 1000).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].row.gross_usdc.as_deref(), Some("2"));
    assert_eq!(rows[0].spread_pct, "20");
    assert_eq!(rows[1].input_usdc, "12.02");
    assert_eq!(rows[1].row.gross_usdc.as_deref(), Some("0.98"));
    let payload = observation(&s, &rows[0], 1000);
    assert_eq!(payload["completeNetUsdc"], serde_json::Value::Null);
    assert_eq!(payload["executable"], false);
    assert!(payload["inventory"].as_array().unwrap().is_empty());
    assert!(payload["message"].as_str().unwrap().contains("未读取账户"));
    let mut closed = s.clone();
    closed.tokens[0].deposit_enabled = Some(false);
    assert!(observation(&closed, &rows[0], 1000)["message"]
        .as_str()
        .unwrap()
        .contains("充值关闭"));
    let mut unknown = s.clone();
    unknown.token_metadata_at_ms = Some(0);
    assert_eq!(
        observation(&unknown, &rows[0], 40_000)["transfer"],
        serde_json::Value::Null
    );
    for case in 0..6 {
        let mut bad = s.clone();
        match case {
            0 => bad.connected = false,
            1 => bad.books.clear(),
            2 => {
                let c = bad.comparison.as_mut().unwrap();
                c.buy.requested_at_ms = -30_000;
                c.sell.as_mut().unwrap().requested_at_ms = -30_000;
            }
            3 => bad.trading_route.as_mut().unwrap().kind = StockRouteKind::Rfq,
            4 => bad.monitor.request.as_mut().unwrap().budget_usdc = "20".into(),
            _ => bad.monitor.alerts.min_spread_pct = "100".into(),
        }
        assert!(candidates(&bad, 1000).unwrap().is_empty(), "case {case}");
    }
    let mut wrong = s.clone();
    wrong.security.as_mut().unwrap().cusip = None;
    assert!(candidates(&wrong, 1000).is_err());
    assert!(candidates(&s, 50_000).unwrap().is_empty());
    if let Ok(path) = std::env::var("STOCK_ALERT_SNAPSHOT") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&fixture(common::time::now_ms())).unwrap(),
        )
        .unwrap();
    }
}

#[tokio::test]
async fn stock_alert_outbox_deduplicates_both_directions_across_restart_and_bucket_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("outbox.sqlite");
    let w = dispatcher(&path).await;
    let service = BackpackStocks::new().unwrap().with_webhook(w.clone());
    *service.snapshot.write() = fixture(common::time::now_ms());
    let hub = realtime::WsHub::new(8);
    let mut cursor = Cursor::default();
    service.alert_once(&hub, &mut cursor).await;
    assert_eq!(w.status(0).await.queue_depth, 2);
    assert_eq!(service.snapshot().alerts.recent.len(), 2);
    service.alert_once(&hub, &mut cursor).await;
    assert_eq!(service.snapshot().alerts.phase, StockAlertPhase::Cooldown);
    assert_eq!(w.status(0).await.queue_depth, 2);
    assert!(service.background_monitoring());
    assert!(service.rfq_worker.lock().is_none());
    assert_eq!(service.account_tracking_until_ms.load(Ordering::SeqCst), 0);
    drop(service);
    drop(w);
    let replay = dispatcher(&path).await;
    let service = BackpackStocks::new().unwrap().with_webhook(replay.clone());
    *service.snapshot.write() = fixture(common::time::now_ms());
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(service.snapshot().alerts.phase, StockAlertPhase::Cooldown);
    assert_eq!(replay.status(0).await.queue_depth, 2);
    let c = candidates(&fixture(59_999), 59_999).unwrap().remove(0);
    replay
        .enqueue(
            WebhookEvent {
                id: event_id(&c, 60, 59_999),
                version: WEBHOOK_EVENT_VERSION.into(),
                kind: WebhookEventKind::StockSpread,
                occurred_at_ms: 59_999,
                payload: serde_json::json!({}),
            },
            false,
        )
        .await
        .unwrap();
    assert!(cooling(&replay, &c, 60, 60_001));
    assert!(!cooling(&replay, &c, 60, 120_001));
    replay
        .update_config(WebhookConfigPatch {
            enabled: Some(false),
            ..Default::default()
        })
        .unwrap();
    assert!(!service.background_monitoring());
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(
        service.snapshot().alerts.phase,
        StockAlertPhase::NeedsWebhook
    );
}

#[test]
fn stock_alert_funding_is_explicitly_a_recent_check_not_current_execution_permission() {
    let now = 50_000;
    let mut s = fixture(now);
    s.funding_assets =
        protocol::asset_context(include_bytes!("../funding/fixtures/assets.json"), "MU.US")
            .unwrap()
            .1;
    let mut p = plans::tests::fixture(now - 100).0.preflight.unwrap();
    p.funding = vec![StockFundingDirection {
        direction: StockChainDirection::Buy,
        needs: vec![StockFundingNeed {
            asset: "USDC".into(),
            target: "Solana".into(),
            source: "Backpack".into(),
            required: Some("10".into()),
            available: Some("0".into()),
            shortfall: Some("10".into()),
            source_available: Some("12".into()),
            source_spare: Some("12".into()),
            source_trade_reserve: Some("0".into()),
            conservative_source_budget: Some("11".into()),
            source_sufficient: Some(true),
            token: None,
            metadata_at_ms: None,
            blockers: vec!["没有发起转账".into()],
        }],
    }];
    s.preflight = Some(p);
    let c = candidates(&s, now).unwrap().remove(0);
    let payload = observation(&s, &c, now);
    assert!(payload["inventory"].as_array().unwrap().is_empty());
    assert_eq!(payload["fundingAssets"].as_array().unwrap().len(), 2);
    assert_eq!(payload["lastFundingCheck"]["checkedAtMs"], now - 100);
    assert_eq!(
        payload["lastFundingCheck"]["currentExecutionPermission"],
        false
    );
    assert_eq!(payload["executable"], false);
    assert_eq!(payload["fundAction"], false);
    for text in ["上次补库检查", "USDC 缺 10", "需按当前金额复核，未转币"] {
        assert!(payload["message"].as_str().unwrap().contains(text));
    }
    s.token_metadata_problem = Some("fixture offline".into());
    assert!(observation(&s, &c, now)["fundingAssets"].is_null());
    s.preflight.as_mut().unwrap().checked_at_ms = now - 30_001;
    assert!(observation(&s, &c, now)["lastFundingCheck"].is_null());
}

#[tokio::test]
async fn stock_alert_refreshes_public_transfer_status_only_for_candidate_and_discards_late_response(
) {
    let tmp = tempfile::tempdir().unwrap();
    let w = dispatcher(&tmp.path().join("outbox.sqlite")).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = release.clone();
    let body = serde_json::json!([{"symbol":"MU.US","tokens":fixture(0).tokens}]).to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/api/v1/assets",
        get(move || {
            count.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            let gate = gate.clone();
            async move {
                gate.notified().await;
                body
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut service = BackpackStocks::new().unwrap().with_webhook(w.clone());
    service.root = root;
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(8);
    let mut s = fixture(common::time::now_ms());
    s.token_metadata_at_ms = None;
    s.monitor.alerts.min_spread_pct = "100".into();
    *service.snapshot.write() = s;
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    service.snapshot.write().monitor.alerts.min_spread_pct = "0.5".into();
    let worker = service.clone();
    let output = hub.clone();
    let job = tokio::spawn(async move {
        worker.alert_once(&output, &mut Cursor::default()).await;
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    service
        .watch(StockWatchRequest { asset: None }, hub.clone())
        .await
        .unwrap();
    release.notify_one();
    job.await.unwrap();
    assert!(service.snapshot().security.is_none());
    assert!(service.snapshot().token_metadata_at_ms.is_none());
    assert_eq!(w.status(0).await.queue_depth, 0);
    let mut fresh = fixture(common::time::now_ms());
    fresh.token_metadata_at_ms = None;
    *service.snapshot.write() = fresh;
    release.notify_one();
    service.alert_once(&hub, &mut Cursor::default()).await;
    assert!(service.snapshot().token_metadata_at_ms.is_some());
    assert_eq!(w.status(0).await.queue_depth, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn stock_alert_full_queue_backs_off_and_disabled_history_remains_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let w = dispatcher(&tmp.path().join("outbox.sqlite")).await;
    w.update_config(WebhookConfigPatch {
        queue_capacity: Some(1),
        ..Default::default()
    })
    .unwrap();
    let service = BackpackStocks::new().unwrap().with_webhook(w.clone());
    *service.snapshot.write() = fixture(common::time::now_ms());
    let hub = realtime::WsHub::new(8);
    let mut cursor = Cursor::default();
    service.alert_once(&hub, &mut cursor).await;
    assert_eq!(service.snapshot().alerts.phase, StockAlertPhase::Degraded);
    assert_eq!(w.status(0).await.dropped_total, 1);
    for _ in 0..4 {
        service.alert_once(&hub, &mut cursor).await;
    }
    assert_eq!(w.status(0).await.dropped_total, 1);
    service.snapshot.write().monitor.enabled = false;
    service.alert_once(&hub, &mut cursor).await;
    assert_eq!(service.snapshot().alerts.phase, StockAlertPhase::Disabled);
    let now = common::time::now_ms();
    assert!(needs_worker(&service.snapshot(), now));
    assert!(!needs_worker(&service.snapshot(), now + 120_001));
    assert!(!service.background_monitoring());
}
