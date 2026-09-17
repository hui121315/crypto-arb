use super::*;
use axum::{extract::Query, http::HeaderMap, routing::get, Json, Router};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

struct Server(JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn intent(p: &StockFundingPlan) -> StockFundingWithdrawal {
    StockFundingWithdrawal {
        evidence_conflict: None,
        client_id: format!("bp-{}", p.plan_id),
        submitted_at_ms: p.terms.created_at_ms + 1,
        query_count: 0,
        last_query_at_ms: None,
        remote: None,
        receipt: None,
        problem: None,
    }
}

fn pending(s: &BackpackStocks, now: i64) -> StockFundingPlan {
    let p = funding_plan::tests::fixture(now);
    s.funding_store.insert(p.clone(), now).unwrap();
    s.funding_store.update(&p, intent(&p), now + 1).unwrap()
}

fn attempt(p: &StockFundingPlan, n: u8) -> StockFundingFollowup {
    let at = p.funding_followup_at().unwrap();
    StockFundingFollowup {
        attempts: n,
        last_at_ms: at,
        next_at_ms: (n < STOCK_FUNDING_FOLLOWUP_LIMIT).then(|| at + delay_ms(n)),
        paused: n == STOCK_FUNDING_FOLLOWUP_LIMIT,
        problem: None,
    }
}

#[test]
fn stock_funding_followup_journal_budget_pause_and_legacy_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let s = BackpackStocks::new()
        .unwrap()
        .with_funding_store(path.clone());
    let mut p = pending(&s, 10_000);
    assert!(serde_json::to_value(&p).unwrap().get("followup").is_none());
    assert_eq!(p.funding_followup_at(), Some(15_001));
    for n in 1..=STOCK_FUNDING_FOLLOWUP_LIMIT {
        let f = attempt(&p, n);
        let at = f.last_at_ms;
        assert!(s
            .funding_store
            .update_followup(&p, f.clone(), at - 1)
            .is_err());
        p = s.funding_store.update_followup(&p, f, at).unwrap();
    }
    assert!(p.funding_followup_at().is_none());
    assert_eq!(p.followup.as_ref().unwrap().attempts, 6);
    for case in 0..4 {
        let mut f = p.followup.clone().unwrap();
        match case {
            0 => f.attempts = 1,
            1 => {
                f.paused = false;
                f.next_at_ms = Some(f.last_at_ms + delay_ms(f.attempts));
            }
            2 => f.last_at_ms += 1,
            _ => f.attempts = 7,
        }
        assert!(s
            .funding_store
            .update_followup(&p, f, p.updated_at_ms + 1)
            .is_err());
    }
    assert!(s
        .funding_store
        .cancel(
            &StockPlanRevisionRequest {
                plan_id: p.plan_id.clone(),
                revision: p.revision
            },
            1_000_000
        )
        .is_err());
    drop(s);
    let restored = BackpackStocks::new().unwrap().with_funding_store(path);
    assert!(restored.funding_store.problem().is_none());
    assert_eq!(restored.funding_store.get(&p.plan_id).unwrap(), p);
    assert!(restored.funding_store.next_followup().is_none());
    assert!(restored
        .wallet_claims
        .check("solana", &p.request.wallet_address, 1_000_000)
        .is_err());
}

#[test]
fn stock_funding_followup_conflict_stops_pending_queries_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflict.jsonl");
    let s = BackpackStocks::new().unwrap().with_funding_store(path.clone());
    let p = pending(&s, 10_000);
    assert!(p.funding_followup_at().is_some());
    let mut w = p.withdrawal.clone().unwrap();
    w.evidence_conflict = Some("原提现身份冲突".into());
    let saved = s.funding_store.update(&p, w, 10_002).unwrap();
    assert_eq!(saved.phase, StockFundingPlanPhase::Withdrawing);
    assert!(s.funding_store.next_followup().is_none());
    assert!(s.funding_store.update_followup(&saved, attempt(&p, 1), 15_001).is_err());
    drop(s);
    let restored = BackpackStocks::new().unwrap().with_funding_store(path);
    assert!(restored.funding_store.problem().is_none());
    assert_eq!(restored.funding_store.get(&saved.plan_id).unwrap(), saved);
    assert!(restored.funding_store.next_followup().is_none());
    assert!(restored.wallet_claims.check("solana", &saved.request.wallet_address, 1_000_000).is_err());
}

async fn server(
    p: StockFundingPlan,
    path: std::path::PathBuf,
) -> (String, Arc<AtomicUsize>, Server) {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router=Router::new().route("/wapi/v1/capital/withdrawals",get(move|headers:HeaderMap,Query(q):Query<BTreeMap<String,String>>|{
        let p=p.clone();let count=count.clone();let path=path.clone();async move {
            assert_eq!(q.len(),2);assert_eq!(q["clientId"],format!("bp-{}",p.plan_id));assert_eq!(q["limit"],"2");
            rfq_tests::signed(&headers,"withdrawalQueryAll",q);
            let last=std::fs::read_to_string(path).unwrap().lines().last().unwrap().to_owned();
            let entry:Value=serde_json::from_str(&last).unwrap();
            assert!(entry["plan"]["followup"]["attempts"].as_u64().unwrap()>0);
            assert!(entry["plan"]["withdrawal"]["queryCount"].as_u64().unwrap()>0);
            count.fetch_add(1,Ordering::SeqCst);
            Json(json!([{"id":42,"clientId":format!("bp-{}",p.plan_id),"blockchain":"Solana","symbol":p.request.funding_asset,
                "quantity":p.terms.quantity,"toAddress":p.terms.destination,"status":"pending","isInternal":false,
                "createdAt":chrono::DateTime::from_timestamp_millis(p.terms.created_at_ms+1).unwrap().to_rfc3339()}]))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    (root, calls, server)
}

async fn wait_queries(s: &BackpackStocks, id: &str, count: u32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let p = s.funding_store.get(id).unwrap();
            if p.withdrawal
                .as_ref()
                .is_some_and(|w| w.query_count >= count && w.remote.is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stock_funding_followup_worker_single_query_restart_budget_and_ws() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let now = common::time::now_ms() - 300_000;
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_funding_store(path.clone());
    s.credential_loader = rfq_tests::keys;
    let mut p = pending(&s, now);
    // Recover five durable attempts, not five live waits or requests.
    for n in 1..=5 {
        let f = attempt(&p, n);
        let at = f.last_at_ms;
        p = s.funding_store.update_followup(&p, f, at).unwrap();
    }
    let (root, calls, _server) = server(p.clone(), path.clone()).await;
    s.root = root.clone();
    let s = Arc::new(s);
    let hub = realtime::WsHub::new(16);
    let mut frames = hub.subscribe(realtime::channels::STOCKS);
    let guard = s.submission_lock.lock().await;
    s.resume_funding(hub.clone());
    s.resume_funding(hub.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        s.funding_store
            .get(&p.plan_id)
            .unwrap()
            .followup
            .as_ref()
            .unwrap()
            .attempts,
        5
    );
    drop(guard);
    wait_queries(&s, &p.plan_id, 1).await;
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.payload_json().unwrap()["fundingPlans"][0]["followup"]["attempts"],
        6
    );
    let saved = s.funding_store.get(&p.plan_id).unwrap();
    assert!(saved.followup.as_ref().unwrap().paused);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        saved
            .withdrawal
            .as_ref()
            .unwrap()
            .remote
            .as_ref()
            .unwrap()
            .fee,
        None
    );
    if let Ok(path) = std::env::var("STOCK_FUNDING_FOLLOWUP_CAPTURE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&s.snapshot()).unwrap()).unwrap();
    }
    drop(s);
    let mut restored = BackpackStocks::new().unwrap().with_funding_store(path);
    restored.root = root;
    restored.credential_loader = rfq_tests::keys;
    let restored = Arc::new(restored);
    assert_eq!(restored.funding_store.get(&p.plan_id).unwrap(), saved);
    restored.resume_funding(hub);
    assert!(restored.funding_worker.lock().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(restored
        .wallet_claims
        .check("solana", &p.request.wallet_address, common::time::now_ms())
        .is_err());
}

#[tokio::test]
async fn stock_funding_followup_no_pending_no_credentials_and_wrong_account_pauses() {
    fn unexpected() -> Result<credentials::Credentials, String> {
        panic!("idle service must not load credentials")
    }
    fn wrong() -> Result<credentials::Credentials, String> {
        Err("do not expose loader details".into())
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_funding_store(path.clone());
    s.credential_loader = unexpected;
    let s = Arc::new(s);
    let hub = realtime::WsHub::new(8);
    s.resume_funding(hub.clone());
    assert!(s.funding_worker.lock().is_none());
    drop(s);
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_funding_store(path.clone());
    s.credential_loader = wrong;
    let p = pending(&s, common::time::now_ms() - 60_000);
    let s = Arc::new(s);
    s.resume_funding(hub.clone());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if s.funding_store
                .get(&p.plan_id)
                .unwrap()
                .followup
                .is_some_and(|f| f.problem.is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let saved = s.funding_store.get(&p.plan_id).unwrap();
    let f = saved.followup.as_ref().unwrap();
    assert!(f.paused);
    assert_eq!(f.attempts, 1);
    assert!(!f.problem.as_ref().unwrap().contains("loader"));
    assert_eq!(saved.withdrawal.as_ref().unwrap().query_count, 0);
    drop(s);
    let mut restored = BackpackStocks::new().unwrap().with_funding_store(path);
    restored.credential_loader = unexpected;
    let restored = Arc::new(restored);
    restored.resume_funding(hub);
    assert!(restored.funding_worker.lock().is_none());
}

#[tokio::test]
async fn stock_funding_followup_wrong_fingerprint_and_journal_failure_never_query() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    fn different() -> Result<credentials::Credentials, String> {
        credentials::Credentials::parse(
            &STANDARD.encode(common::signing::ed25519_public_key(&[8; 32]).unwrap()),
            &STANDARD.encode([8; 32]),
        )
    }
    fn unexpected() -> Result<credentials::Credentials, String> {
        panic!("journal failure must precede credentials")
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("funding.jsonl");
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_funding_store(path.clone());
    s.credential_loader = different;
    let p = pending(&s, common::time::now_ms() - 60_000);
    let hub = realtime::WsHub::new(8);
    assert!(s.funding_followup_step(&p.plan_id, &hub).await.unwrap());
    let saved = s.funding_store.get(&p.plan_id).unwrap();
    assert!(saved.followup.as_ref().unwrap().paused);
    assert_eq!(saved.withdrawal.as_ref().unwrap().query_count, 0);
    assert!(!s.funding_followup_step(&p.plan_id, &hub).await.unwrap());
    assert_eq!(s.funding_store.get(&p.plan_id).unwrap(), saved);
    drop(s);
    let path = dir.path().join("blocked.jsonl");
    let mut s = BackpackStocks::new()
        .unwrap()
        .with_funding_store(path.clone());
    s.credential_loader = unexpected;
    let p = pending(&s, common::time::now_ms() - 60_000);
    std::fs::rename(&path, path.with_extension("original")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(s.funding_followup_step(&p.plan_id, &hub).await.is_err());
    assert!(s.funding_store.problem().is_some());
    assert!(s.funding_store.next_followup().is_none());
    assert!(s.funding_store.records()[0].followup.is_none());
}
