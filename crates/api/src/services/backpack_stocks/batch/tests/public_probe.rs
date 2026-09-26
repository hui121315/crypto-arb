use super::*;

// Opt-in only: no app startup, private credentials, journals, taker or notification dispatcher.
#[tokio::test]
#[ignore = "Explicit public read-only multi-stock worker probe; requires runtime-only secret backend"]
async fn batch_public_two_rounds_pause_and_new_budget() {
    assert_eq!(
        std::env::var("APP_CREDENTIALS__SECRET_BACKEND").as_deref(),
        Ok("runtime")
    );
    let mut service = BackpackStocks::new().unwrap();
    service.credential_loader = || panic!("public batch must not load account credentials");
    let service = Arc::new(service);
    let hub = realtime::WsHub::new(128);
    let viewer = hub.subscribe(realtime::channels::STOCKS);
    let mut request = StockBatchRequest {
        enabled: true,
        assets: vec!["MU.US".into(), "SNDK.US".into()],
        budget_usdc: "100".into(),
        keyed: false,
        interval_secs: 5,
    };
    let mut evidence = vec![];
    let result = tokio::time::timeout(Duration::from_secs(160), async {
        service.set_batch(request.clone(), &service.snapshot().batch.revision, hub.clone()).map_err(|e|e.to_string())?;
        let first = wait_round(&service, 1).await?;
        evidence.push(report("first", &first));
        valid_round(&first, "100000000")?;
        let second = wait_round(&service, 2).await?;
        evidence.push(report("second", &second));
        valid_round(&second, "100000000")?;
        for (a, b) in first.rows.iter().zip(&second.rows) {
            if b.checked_at_ms <= a.checked_at_ms {
                return Err("second round did not replace previous quotes".into());
            }
        }
        request.enabled = false;
        let paused = service.set_batch(request.clone(), &service.snapshot().batch.revision, hub.clone()).map_err(|e|e.to_string())?.batch;
        tokio::time::sleep(Duration::from_secs(6)).await;
        let still = service.snapshot().batch;
        if still.running
            || still.completed_rounds != paused.completed_rounds
            || still.rows.iter().any(|r| r.refreshing)
        {
            return Err("paused worker continued quoting".into());
        }
        evidence.push(report("paused", &still));
        request.enabled = true;
        request.budget_usdc = "25".into();
        service.set_batch(request.clone(), &service.snapshot().batch.revision, hub.clone()).map_err(|e|e.to_string())?;
        let resumed = wait_round(&service, still.completed_rounds + 1).await?;
        evidence.push(report("resumed", &resumed));
        valid_round(&resumed, "25000000")?;
        Ok::<_, String>(())
    })
    .await;
    request.enabled = false;
    service.set_batch(request, &service.snapshot().batch.revision, hub).unwrap();
    drop(viewer);
    drop(service);
    if let Ok(path) = std::env::var("STOCK_PUBLIC_CAPTURE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    eprintln!(
        "public batch stages: {}",
        serde_json::to_string(&evidence).unwrap()
    );
    result
        .expect("public worker probe timed out")
        .expect("public worker validation");
}

async fn wait_round(service: &BackpackStocks, count: u64) -> Result<StockBatchStatus, String> {
    tokio::time::timeout(Duration::from_secs(50), async {
        loop {
            let batch = service.snapshot().batch;
            if batch.completed_rounds >= count && !batch.running {
                return batch;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "public batch round timed out".into())
}

fn valid_round(batch: &StockBatchStatus, raw: &str) -> Result<(), String> {
    if let Some(error) = &batch.problem {
        return Err(error.clone());
    }
    if batch.rows.len() != 2 {
        return Err("batch did not return both stocks".into());
    }
    for row in &batch.rows {
        if row.buy.as_ref().is_none_or(|q| q.input_raw != raw)
            || row.sell.is_none()
            || row.mint.is_none()
        {
            return Err(format!(
                "{} incomplete: {:?}",
                row.security.asset, row.problem
            ));
        }
    }
    Ok(())
}

fn report(stage: &str, batch: &StockBatchStatus) -> serde_json::Value {
    let now = common::time::now_ms();
    serde_json::json!({"stage":stage,"rounds":batch.completed_rounds,"problem":batch.problem,
        "observedAtMs":now,"rows":batch.rows.iter().map(|r|serde_json::json!({
            "asset":r.security.asset,"checkedAtMs":r.checked_at_ms,"problem":r.problem,
            "connected":r.connected,"books":r.books.len(),
            "buyMs":r.buy.as_ref().map(|q|q.received_at_ms-q.requested_at_ms),
            "sellMs":r.sell.as_ref().map(|q|q.received_at_ms-q.requested_at_ms),
            "freshBuy":r.token_price(true,now).is_some(),"freshSell":r.token_price(false,now).is_some(),
        })).collect::<Vec<_>>()})
}
