use super::*;
use std::sync::atomic::AtomicUsize;

fn service() -> Arc<BackpackStocks> {
    let service = Arc::new(BackpackStocks::new().unwrap());
    let now = common::time::now_ms();
    let mut snapshot = comparison::tests::snapshot();
    let security = calendar::tests::security();
    snapshot.security = Some(security.clone());
    snapshot.token_metadata_at_ms = Some(now);
    *service.snapshot.write() = snapshot;
    *service.catalog.write() = Some(StockCatalog {
        rows: vec![security],
        observed_at_ms: now,
    });
    *service.calendar.write() =
        Some(calendar::Calendar::parse(&calendar::tests::sessions(), b"[]", now).unwrap());
    service
}
fn request(enabled: bool, budget: &str) -> StockMonitorRequest {
    StockMonitorRequest {
        enabled,
        alerts: Default::default(),
        quote: StockQuoteRequest {
            asset: "MU.US".into(),
            budget_usdc: budget.into(),
            keyed: false,
        },
    }
}
async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn backpack_stock_monitor_shares_one_job_and_cancels_on_last_viewer_disable_and_stop() {
    let service = service();
    let hub = realtime::WsHub::new(16);
    service.configure_monitor(request(true, "10")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let started = calls.clone();
    let running = active.clone();
    let runner = tokio::spawn(run_with(
        Arc::downgrade(&service),
        hub.clone(),
        move |_, _, kind, _| {
            assert_eq!(kind, JobKind::Quote);
            let started = started.clone();
            let running = running.clone();
            Box::pin(async move {
                started.fetch_add(1, Ordering::SeqCst);
                running.fetch_add(1, Ordering::SeqCst);
                let _guard = Active(running);
                futures::future::pending::<JobResult>().await
            })
        },
    ));
    until(|| service.snapshot().monitor.phase == StockMonitorPhase::WaitingForViewers).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let viewer1 = hub.subscribe(realtime::channels::STOCKS);
    let viewer2 = hub.subscribe(realtime::channels::STOCKS);
    until(|| calls.load(Ordering::SeqCst) == 1).await;
    drop(viewer1);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 1);
    let generation = service.generation.load(Ordering::SeqCst);
    service.configure_monitor(request(true, "10")).unwrap();
    assert_eq!(service.generation.load(Ordering::SeqCst), generation);
    service
        .configure_monitor(request(false, "invalid draft must not prevent pausing"))
        .unwrap();
    assert_eq!(
        service.snapshot().monitor.request.unwrap().budget_usdc,
        "10"
    );
    until(|| active.load(Ordering::SeqCst) == 0).await;
    service.configure_monitor(request(true, "10")).unwrap();
    until(|| calls.load(Ordering::SeqCst) == 2).await;
    drop(viewer2);
    until(|| active.load(Ordering::SeqCst) == 0).await;
    assert_eq!(
        service.snapshot().monitor.phase,
        StockMonitorPhase::WaitingForViewers
    );
    let viewer3 = hub.subscribe(realtime::channels::STOCKS);
    until(|| calls.load(Ordering::SeqCst) == 3).await;
    service
        .watch(StockWatchRequest { asset: None }, hub.clone())
        .await
        .unwrap();
    until(|| active.load(Ordering::SeqCst) == 0).await;
    assert!(!service.snapshot().monitor.enabled);
    drop(viewer3);
    runner.abort();
    let _ = runner.await;
}

#[tokio::test]
async fn backpack_stock_monitor_backs_off_without_overlapping_and_new_parameters_resume_immediately(
) {
    let service = service();
    let hub = realtime::WsHub::new(16);
    let _viewer = hub.subscribe(realtime::channels::STOCKS);
    service.configure_monitor(request(true, "10")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let started = calls.clone();
    let runner = tokio::spawn(run_with(
        Arc::downgrade(&service),
        hub.clone(),
        move |service, _, kind, _| {
            assert_eq!(kind, JobKind::Quote);
            let call = started.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if call == 0 {
                    return Err("Jupiter HTTP 429 fixture".into());
                }
                let mut snapshot = service.snapshot();
                let mut comparison = comparison::tests::comparison();
                comparison.buy.requested_at_ms = common::time::now_ms();
                if call == 2 {
                    comparison.sell_problem = Some("reverse quote HTTP 429 fixture".into());
                }
                snapshot.comparison = Some(comparison);
                Ok(Some(snapshot))
            })
        },
    ));
    until(|| service.snapshot().monitor.phase == StockMonitorPhase::Backoff).await;
    assert_eq!(service.snapshot().monitor.consecutive_failures, 1);
    assert!(service.snapshot().monitor.next_attempt_at_ms.unwrap() > common::time::now_ms() + 4000);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    service.configure_monitor(request(true, "20")).unwrap();
    until(|| service.snapshot().monitor.completed_quotes == 1).await;
    assert_eq!(service.snapshot().monitor.consecutive_failures, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(service.snapshot().monitor.next_attempt_at_ms.unwrap() > common::time::now_ms() + 4000);
    service.configure_monitor(request(true, "30")).unwrap();
    until(|| service.snapshot().monitor.phase == StockMonitorPhase::Backoff).await;
    assert_eq!(service.snapshot().monitor.completed_quotes, 0);
    assert_eq!(service.snapshot().monitor.consecutive_failures, 1);
    assert!(service
        .snapshot()
        .monitor
        .problem
        .unwrap()
        .contains("reverse quote"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(backoff_ms(100), 60_000);
    runner.abort();
    let _ = runner.await;
}

#[tokio::test]
async fn backpack_stock_quantity_limit_waits_without_errors_and_resumes_on_parameters_or_session() {
    let service = service();
    let hub = realtime::WsHub::new(16);
    let _viewer = hub.subscribe(realtime::channels::STOCKS);
    service.configure_monitor(request(true, "10")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let started = calls.clone();
    let runner = tokio::spawn(run_with(
        Arc::downgrade(&service),
        hub,
        move |service, _, kind, _| {
            assert_eq!(kind, JobKind::Quote);
            let call = started.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let mut comparison = comparison::tests::comparison();
                comparison.budget_usdc = service.snapshot().monitor.request.unwrap().budget_usdc;
                comparison.buy.requested_at_ms = common::time::now_ms();
                if call < 2 {
                    comparison.quantity_limit = Some(StockQuoteQuantityLimit {
                        quoted_shares: "0.010658".into(),
                        min_quantity: "1".into(),
                        max_quantity: Some("1000".into()),
                        step_size: "1".into(),
                    });
                    comparison.sell_problem =
                        Some("本次最低到账 0.010658 股，不足当前时段最小股数 1".into());
                } else {
                    comparison.sell = Some(comparison.buy.clone());
                }
                service.snapshot.write().comparison = Some(comparison);
                Ok(Some(service.snapshot()))
            })
        },
    ));
    until(|| service.snapshot().monitor.phase == StockMonitorPhase::QuantityLimited).await;
    let waiting = service.snapshot();
    assert!(waiting.monitor.enabled);
    assert_eq!(waiting.monitor.consecutive_failures, 0);
    assert_eq!(waiting.monitor.request.as_ref().unwrap().budget_usdc, "10");
    assert!(waiting.monitor.next_attempt_at_ms.unwrap() >= common::time::now_ms() + 29_000);
    assert!(waiting.monitor.problem.unwrap().contains("最小股数 1"));
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    service.configure_monitor(request(true, "20")).unwrap();
    until(|| {
        calls.load(Ordering::SeqCst) == 2
            && service.snapshot().monitor.phase == StockMonitorPhase::QuantityLimited
    })
    .await;
    assert_eq!(
        service.snapshot().monitor.request.unwrap().budget_usdc,
        "20"
    );
    {
        let mut snapshot = service.snapshot.write();
        let route = snapshot.trading_route.as_mut().unwrap();
        route.kind = StockRouteKind::Rfq;
        route.session = Some(StockSession {
            name: "US_EQUITIES_REGULAR".into(),
            min_quantity: "0.002".into(),
            max_quantity: Some("10000".into()),
            step_size: "0.00001".into(),
        });
    }
    until(|| service.snapshot().monitor.completed_quotes == 1).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        service.snapshot().monitor.phase,
        StockMonitorPhase::Watching
    );
    assert_eq!(service.snapshot().monitor.consecutive_failures, 0);
    assert!(service.snapshot().monitor.problem.is_none());
    runner.abort();
    let _ = runner.await;
}

#[tokio::test]
async fn stock_alert_background_quotes_stop_when_global_delivery_or_local_monitor_is_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let webhook = Arc::new(
        webhook::WebhookDispatcher::initialize(Some(tmp.path().join("outbox.sqlite")))
            .await
            .unwrap(),
    );
    webhook
        .update_config(shared_types::WebhookConfigPatch {
            enabled: Some(true),
            url: Some("https://example.com/fixture-no-send".into()),
            secret: Some("fixture".into()),
            event_kinds: Some(vec![shared_types::WebhookEventKind::StockSpread]),
            ..Default::default()
        })
        .unwrap();
    let mut service = service();
    Arc::get_mut(&mut service).unwrap().webhook = Some(webhook.clone());
    let mut parameters = request(true, "10");
    parameters.alerts.enabled = true;
    service.configure_monitor(parameters).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let running = active.clone();
    let runner = tokio::spawn(run_with(
        Arc::downgrade(&service),
        realtime::WsHub::new(8),
        move |_, _, kind, _| {
            assert_eq!(kind, JobKind::Quote);
            let running = running.clone();
            Box::pin(async move {
                running.fetch_add(1, Ordering::SeqCst);
                let _guard = Active(running);
                futures::future::pending::<JobResult>().await
            })
        },
    ));
    until(|| active.load(Ordering::SeqCst) == 1).await;
    webhook
        .update_config(shared_types::WebhookConfigPatch {
            enabled: Some(false),
            ..Default::default()
        })
        .unwrap();
    until(|| active.load(Ordering::SeqCst) == 0).await;
    webhook
        .update_config(shared_types::WebhookConfigPatch {
            enabled: Some(true),
            ..Default::default()
        })
        .unwrap();
    until(|| active.load(Ordering::SeqCst) == 1).await;
    let mut stop = request(false, "invalid");
    stop.alerts.min_spread_pct = "invalid".into();
    service.configure_monitor(stop).unwrap();
    until(|| active.load(Ordering::SeqCst) == 0).await;
    runner.abort();
    let _ = runner.await;
}
