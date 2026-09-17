use super::*;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;
use axum::{extract::Request, routing::get, Json, Router};
use common::config::AppConfig;
use exchange::adapters::{
    Binance, BinanceConfig, BinanceCredentials, Bitget, BitgetConfig, BitgetCredentials, Bybit,
    BybitConfig, BybitCredentials, Kraken, KrakenConfig, KrakenCredentials, KrakenSpotCredentials,
};
use exchange::live::LiveTradingAdapter;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub(super) struct HistoryServer {
    pub(super) url: String,
    task: tokio::task::JoinHandle<()>,
    pub(super) queries: Arc<Mutex<Vec<String>>>,
}

impl Drop for HistoryServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(venue: &str, body: Value) -> HistoryServer {
    history_server(venue, body, None).await
}

pub(super) async fn history_server(
    venue: &str,
    body: Value,
    path: Option<&'static str>,
) -> HistoryServer {
    let queries = Arc::new(Mutex::new(Vec::new()));
    let recorded = queries.clone();
    let (history_path, time_path, time_body) = match venue {
        "binance" => (
            "/sapi/v1/capital/deposit/hisrec",
            "/fapi/v1/time",
            json!({"serverTime": common::time::now_ms()}),
        ),
        "bitget" => (
            "/api/v3/account/deposit-records",
            "/api/v2/public/time",
            json!({"code":"00000","data":{"serverTime":common::time::now_ms().to_string()}}),
        ),
        "bybit" => (
            "/v5/asset/deposit/query-record",
            "/v5/market/time",
            json!({"retCode":0,"result":{"timeSecond":(common::time::now_ms()/1000).to_string()}}),
        ),
        "kraken" => ("/funding/v1/withdrawals", "/fixture-unused-time", json!({})),
        _ => unreachable!(),
    };
    let app = Router::new()
        .route(
            time_path,
            get(move || {
                let body = time_body.clone();
                async move { Json(body) }
            }),
        )
        .route(
            path.unwrap_or(history_path),
            get(move |request: Request| {
                let body = body.clone();
                let recorded = recorded.clone();
                async move {
                    recorded
                        .lock()
                        .unwrap()
                        .push(request.uri().query().unwrap_or_default().to_owned());
                    Json(body)
                }
            }),
        );
    let app = if venue == "kraken" {
        app.route("/funding/v1/addresses", get(|| async {
            Json(json!({"addresses":[{"address_id":"ABR6SXP-SF6CY-VJMONY","scope":{"method_id":"3e7f8072-cc6d-4394-982a-5f4ca6ab27dd"},
                "verified":true,"address_details":{"crypto":{"address":"SolanaDepositAddress"}}}]}))
        }))
    } else { app };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    HistoryServer { url, task, queries }
}

pub(super) fn adapter(venue: &str, url: &str) -> Box<dyn LiveTradingAdapter> {
    match venue {
        "binance" => Box::new(
            Binance::new(BinanceConfig {
                base_url_override: Some(url.to_owned()),
                credentials: Some(BinanceCredentials {
                    api_key: "fixture-key".to_owned(),
                    api_secret: "fixture-secret".to_owned(),
                }),
                ..Default::default()
            })
            .unwrap(),
        ),
        "bitget" => Box::new(
            Bitget::new(BitgetConfig {
                base_url_override: Some(url.to_owned()),
                credentials: Some(BitgetCredentials {
                    api_key: "fixture-key".to_owned(),
                    api_secret: "fixture-secret".to_owned(),
                    passphrase: "fixture-passphrase".to_owned(),
                }),
                ..Default::default()
            })
            .unwrap(),
        ),
        "bybit" => Box::new(
            Bybit::new(BybitConfig {
                base_url_override: Some(url.to_owned()),
                credentials: Some(BybitCredentials {
                    api_key: "fixture-key".to_owned(),
                    api_secret: "fixture-secret".to_owned(),
                }),
                ..Default::default()
            })
            .unwrap(),
        ),
        "kraken" => {
            let ws = url.replacen("http:", "ws:", 1);
            Box::new(Kraken::new(KrakenConfig {
                credentials: Some(KrakenCredentials { spot: Some(KrakenSpotCredentials {api_key:"fixture-key".into(),api_secret:"c2VjcmV0".into()}), futures: None }),
                spot_rest_url_override: Some(url.into()), futures_rest_url_override: Some(url.into()),
                spot_public_ws_url_override: Some(format!("{ws}/spot")), spot_private_ws_url_override: Some(format!("{ws}/private")),
                futures_ws_url_override: Some(format!("{ws}/futures")), ..Default::default()
            }).unwrap())
        }
        _ => unreachable!(),
    }
}

fn row(venue: &str, amount: &str, fee: &str, now: i64) -> Value {
    match venue {
        "binance" => {
            json!({"amount":amount,"coin":"USDC","network":"SOL","status":1,"address":"SolanaDepositAddress","addressTag":"","txId":"SolanaTxSignature","insertTime":now,"confirmTimes":"12/12"})
        }
        "bitget" => {
            json!({"orderId":"9","recordId":"SolanaTxSignature","coin":"USDC","type":"deposit","dest":"on_chain","size":amount,"status":"success","toAddress":"SolanaDepositAddress","chain":"SOL","createdTime":now.to_string()})
        }
        "bybit" => {
            json!({"coin":"USDC","chain":"SOL","amount":amount,"depositFee":fee,"txID":"SolanaTxSignature","status":3,"toAddress":"SolanaDepositAddress","tag":"","confirmations":"12","depositType":"0","successAt":now.to_string()})
        }
        _ => unreachable!(),
    }
}

fn envelope(venue: &str, rows: Vec<Value>) -> Value {
    match venue {
        "binance" => json!(rows),
        "bitget" => json!({"code":"00000","msg":"success","data":rows}),
        "bybit" => json!({"retCode":0,"retMsg":"OK","result":{"rows":rows,"nextPageCursor":""}}),
        _ => unreachable!(),
    }
}

fn ledger(venue: &str, now: i64) -> (tempfile::TempDir, AppConfig, OnchainReplenishmentRun) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    let path = dir.path().join("replenishment.jsonl");
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.plan.legs[0].venue = venue.to_owned();
    run.authorization.authorized_at_ms = now;
    run.authorization.valid_until_ms = now + 60_000;
    run.plan.built_at_ms = now;
    run.plan.valid_until_ms = now + 5_000;
    // An unresolved first leg must never release the second leg on a shortfall.
    run.plan.legs.push(run.plan.legs[0].clone());
    run.transfers[0].submission_attempted_at_ms = now;
    run.transfers[0].status = shared_types::OnchainReplenishmentTransferStatus::SourceCompleted;
    run.transfers[0].credited_amount_exact = None;
    run.transfers[0].withdrawal_unlocked = None;
    std::fs::write(path, format!("{}\n", json!({"schemaVersion":1,"run":run}))).unwrap();
    (dir, config, run)
}

#[tokio::test]
async fn kraken_deposit_http_credit_survives_restart_without_repeating_a_transfer() {
    use exchange::adapters::{Kraken, KrakenConfig, KrakenCredentials, KrakenSpotCredentials};
    use axum::routing::post;
    const METHOD_ID: &str = "3e7f8072-cc6d-4394-982a-5f4ca6ab27dd";
    const NETWORK_ID: &str = "b336ce74-8d60-42b8-8714-b1095e06b711";
    let now = common::time::now_ms();
    for amount in ["12.5", "12.4"] {
        let (_dir, config, mut run) = ledger("kraken", now);
        run.plan.legs[0].network_evidence.network = Some(METHOD_ID.into());
        std::fs::write(config.storage.onchain_replenishment_ledger_path.as_ref().unwrap(),
            format!("{}\n", json!({"schemaVersion":1,"run":run}))).unwrap();
        let method_body = json!({"methods":[{"asset":{"class":"currency","name":"USDC"},"method_id":METHOD_ID,
            "fees":{"base":{"asset":{"class":"currency","name":"USDC"},"amount":"0"},"included":true},
            "network":{"network_id":NETWORK_ID,"network_name":"Solana"}}]});
        let funding_body = json!({"deposits":[{"deposit_id":"FTcQ4qW-fWGQbQwUfqdnZo4dsMn1ao","method_id":METHOD_ID,"network_id":NETWORK_ID,
            "status":"success","amount":{"asset":{"class":"currency","name":"USDC"},"amount":amount},
            "fee":{"asset":{"class":"currency","name":"USDC"},"amount":"0"},
            "create_time":chrono::DateTime::from_timestamp_millis(now).unwrap().to_rfc3339()}]});
        let legacy_body = json!({"error":[],"result":{"deposit":[{"aclass":"currency","asset":"USDC","refid":"FTcQ4qW-fWGQbQwUfqdnZo4dsMn1ao",
            "txid":"SolanaTxSignature","info":"SolanaDepositAddress","amount":amount,"fee":"0","time":now/1000,"status":"Success"}],"next_cursor":""}});
        let app = Router::new()
            .route("/funding/v1/methods/deposit",get(move || { let body=method_body.clone(); async move {Json(body)} }))
            .route("/funding/v1/deposits",get(move || { let body=funding_body.clone(); async move {Json(body)} }))
            .route("/0/private/DepositStatus",post(move || { let body=legacy_body.clone(); async move {Json(body)} }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}",listener.local_addr().unwrap());
        let server=HistoryServer { url,task:tokio::spawn(async move {axum::serve(listener,app).await.unwrap();}),queries:Arc::new(Mutex::new(Vec::new())) };
        let ws=server.url.replacen("http:","ws:",1);
        let adapter=Kraken::new(KrakenConfig {
            credentials:Some(KrakenCredentials {spot:Some(KrakenSpotCredentials{api_key:"fixture-key".into(),api_secret:"c2VjcmV0".into()}),futures:None}),
            spot_rest_url_override:Some(server.url.clone()),futures_rest_url_override:Some(server.url.clone()),
            spot_public_ws_url_override:Some(format!("{ws}/spot")),spot_private_ws_url_override:Some(format!("{ws}/private")),
            futures_ws_url_override:Some(format!("{ws}/futures")),..Default::default()
        }).unwrap();
        let evidence=adapter.deposit_status(&deposit_status_request(&run).unwrap()).await.unwrap().unwrap();
        let store=OnchainReplenishmentPlanStore::load(&config);
        let updated=record_cex_credit(&store,&run,&evidence,now+1).unwrap();
        assert_eq!(updated.status,if amount=="12.5" {OnchainReplenishmentRunStatus::ReadyForNextTransfer} else {OnchainReplenishmentRunStatus::Paused});
        assert_eq!(updated.transfers[0].credited_amount_exact.as_deref(),Some(amount));
        assert_eq!(updated.transfers[0].deposit_fee_exact.as_deref(),Some("0"));
        assert_eq!(updated.transfers.len(),1);
        let restored_store=OnchainReplenishmentPlanStore::load(&config);
        let restored=restored_store.run(&run.run_id,now+2).unwrap();
        assert_eq!(restored.transfers,updated.transfers);
        assert!(record_cex_credit(&restored_store,&restored,&evidence,now+3).is_err());
        let replay=restored_store.run(&run.run_id,now+3).unwrap();
        assert_eq!(replay.transfers,restored.transfers,"repeated proof must not credit twice or advance another leg");
    }
}

#[tokio::test]
async fn replenishment_deposit_http_to_ledger_preserves_actual_credit_and_pauses_shortfalls() {
    let now = common::time::now_ms();
    for venue in ["binance", "bitget", "bybit"] {
        for amount in ["12.4", "0", "12.5"] {
            let (_dir, config, run) = ledger(venue, now);
            let mut wrong_identity = row(venue, amount, "0", now);
            wrong_identity["coin"] = "OTHER".into();
            let server = server(
                venue,
                envelope(venue, vec![wrong_identity, row(venue, amount, "0", now)]),
            )
            .await;
            let store = OnchainReplenishmentPlanStore::load(&config);
            let request = deposit_status_request(&run).unwrap();
            let evidence = adapter(venue, &server.url)
                .deposit_status(&request)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(evidence.amount, amount.parse::<Decimal>().unwrap());
            let updated = record_cex_credit(&store, &run, &evidence, now + 1).unwrap();
            let expected = if amount == "12.5" {
                OnchainReplenishmentRunStatus::ReadyForNextTransfer
            } else {
                OnchainReplenishmentRunStatus::Paused
            };
            assert_eq!(updated.status, expected, "{venue}, amount={amount}");
            assert_eq!(
                updated.transfers[0].credited_amount_exact.as_deref(),
                Some(amount)
            );
            assert_eq!(updated.transfers.len(), 1);
            assert_eq!(
                updated.transfers[0]
                    .reported_deposit_amount_exact
                    .as_deref(),
                Some(amount)
            );
            assert_eq!(
                updated.transfers[0].deposit_fee_exact,
                evidence.deposit_fee.map(|fee| fee.normalize().to_string()),
                "an explicit fee, including zero, must survive the durable credit record"
            );
            if amount != "12.5" {
                assert!(updated.problem.as_deref().unwrap().contains("低于计划"));
                let claim = store
                    .claim_submission(
                        &run.run_id,
                        &run.authorization.actor,
                        run.plan.clone(),
                        1,
                        &store.submission_snapshot(),
                        now + 2,
                    )
                    .unwrap();
                assert!(claim.replayed);
                assert_eq!(claim.run.status, OnchainReplenishmentRunStatus::Paused);
                assert_eq!(claim.run.transfers.len(), 1);
            }
            let restored = OnchainReplenishmentPlanStore::load(&config)
                .run(&run.run_id, now + 2)
                .unwrap();
            assert_eq!(restored.status, updated.status);
            assert_eq!(restored.transfers, updated.transfers);
            assert_eq!(server.queries.lock().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn replenishment_deposit_nonzero_fee_requires_net_credit_evidence_without_double_deduction() {
    let now = common::time::now_ms();
    let (_dir, config, run) = ledger("bybit", now);
    let server = server(
        "bybit",
        envelope("bybit", vec![row("bybit", "12.5", "0.1", now)]),
    )
    .await;
    let evidence = adapter("bybit", &server.url)
        .deposit_status(&deposit_status_request(&run).unwrap())
        .await
        .unwrap()
        .unwrap();
    let store = OnchainReplenishmentPlanStore::load(&config);
    let updated = record_cex_credit(&store, &run, &evidence, now + 1).unwrap();
    assert_eq!(updated.status, OnchainReplenishmentRunStatus::Paused);
    assert_eq!(updated.transfers[0].credited_amount_exact, None);
    assert_eq!(
        updated.transfers[0]
            .reported_deposit_amount_exact
            .as_deref(),
        Some("12.5")
    );
    assert_eq!(
        updated.transfers[0].deposit_fee_exact.as_deref(),
        Some("0.1")
    );
    let claim = store
        .claim_submission(
            &run.run_id,
            &run.authorization.actor,
            run.plan.clone(),
            1,
            &store.submission_snapshot(),
            now + 2,
        )
        .unwrap();
    assert!(claim.replayed);
    assert_eq!(claim.run.status, OnchainReplenishmentRunStatus::Paused);
    let restored = OnchainReplenishmentPlanStore::load(&config)
        .run(&run.run_id, now + 2)
        .unwrap();
    assert_eq!(restored.transfers, updated.transfers);
}

#[tokio::test]
async fn replenishment_deposit_ambiguity_and_invalid_amounts_never_reach_the_ledger() {
    let now = common::time::now_ms();
    for venue in ["binance", "bitget", "bybit"] {
        for rows in [
            vec![row(venue, "12.4", "0", now), row(venue, "12.5", "0", now)],
            vec![row(venue, "-1", "0", now)],
            vec![row(venue, "bad", "0", now)],
        ] {
            let (_dir, config, run) = ledger(venue, now);
            let server = server(venue, envelope(venue, rows)).await;
            let result = adapter(venue, &server.url)
                .deposit_status(&deposit_status_request(&run).unwrap())
                .await;
            assert!(result.is_err(), "{venue}");
            let restored = OnchainReplenishmentPlanStore::load(&config)
                .run(&run.run_id, now + 2)
                .unwrap();
            assert_eq!(restored.transfers, run.transfers);
            assert_eq!(
                restored.status,
                OnchainReplenishmentRunStatus::AwaitingDestinationCredit
            );
        }
    }
}
