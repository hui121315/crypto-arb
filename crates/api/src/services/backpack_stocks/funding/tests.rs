use super::*;
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

#[derive(Clone, Default)]
struct Mock {
    calls: Arc<AtomicUsize>,
    body: Arc<Mutex<Value>>,
    gate: Option<Arc<tokio::sync::Notify>>,
    entered: Arc<tokio::sync::Notify>,
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn server(mock: Mock) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route(
            "/wapi/v1/capital/deposit/address",
            get(
                |State(m): State<Mock>,
                 headers: HeaderMap,
                 Query(query): Query<BTreeMap<String, String>>| async move {
                    assert_eq!(
                        query,
                        BTreeMap::from([("blockchain".into(), "Solana".into())])
                    );
                    rfq_tests::signed(&headers, "depositAddressQuery", query);
                    m.calls.fetch_add(1, Ordering::SeqCst);
                    m.entered.notify_one();
                    if let Some(gate) = &m.gate {
                        gate.notified().await;
                    }
                    Json(m.body.lock().clone())
                },
            ),
        )
        .with_state(mock);
    (
        root,
        Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap()
        })),
    )
}

#[tokio::test]
async fn stock_funding_address_is_signed_read_only_cached_and_invalidated_without_fund_actions() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("plans.jsonl");
    let address = bs58::encode([9; 32]).into_string();
    let mock = Mock::default();
    *mock.body.lock() = json!({"address":address});
    let (root, _server) = server(mock.clone()).await;
    let (mut s, _) = BackpackStocks::stock_plan_fixture(path.clone(), common::time::now_ms());
    s.root = root;
    let hub = realtime::WsHub::new(8);
    let request = || StockDepositAddressRequest {
        asset: "MU.US".into(),
    };
    let account = s.account.read().evidence.clone();
    let first = s
        .read_deposit_address(request(), &hub)
        .await
        .unwrap()
        .deposit_address
        .unwrap();
    assert_eq!(first.address, address);
    assert_eq!(first.blockchain, "Solana");
    assert_eq!(
        first.account_fingerprint,
        rfq_tests::keys().unwrap().fingerprint()
    );
    assert_eq!(
        s.read_deposit_address(request(), &hub)
            .await
            .unwrap()
            .deposit_address,
        Some(first)
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    s.snapshot
        .write()
        .deposit_address
        .as_mut()
        .unwrap()
        .account_fingerprint = "previous-account".into();
    s.read_deposit_address(request(), &hub).await.unwrap();
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    s.snapshot
        .write()
        .deposit_address
        .as_mut()
        .unwrap()
        .checked_at_ms -= 30_001;
    *mock.body.lock() = json!({"address":"invalid-solana-address"});
    assert!(s.read_deposit_address(request(), &hub).await.is_err());
    assert!(s.snapshot().deposit_address.is_none());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 3);
    for response in [json!({}), json!({"address":17})] {
        *mock.body.lock() = response;
        assert!(s.read_deposit_address(request(), &hub).await.is_err());
        assert!(s.snapshot().deposit_address.is_none());
    }
    assert!(s
        .read_deposit_address(
            StockDepositAddressRequest {
                asset: "SNDK.US".into()
            },
            &hub
        )
        .await
        .is_err());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 5);
    assert_eq!(s.account.read().evidence, account);
    assert!(s.snapshot().plans.is_empty());
    assert!(!path.exists());
    *mock.body.lock() = json!({"address":address});
    s.read_deposit_address(request(), &hub).await.unwrap();
    s.credential_loader = || Err("fixture missing credentials".into());
    assert!(s.read_deposit_address(request(), &hub).await.is_err());
    assert!(s.snapshot().deposit_address.is_none());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn stock_funding_address_discards_response_after_selection_generation_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("plans.jsonl");
    let gate = Arc::new(tokio::sync::Notify::new());
    let mock = Mock {
        gate: Some(gate.clone()),
        ..Default::default()
    };
    *mock.body.lock() = json!({"address":bs58::encode([9;32]).into_string()});
    let (root, _server) = server(mock.clone()).await;
    let (mut s, _) = BackpackStocks::stock_plan_fixture(path.clone(), common::time::now_ms());
    s.root = root;
    let s = Arc::new(s);
    let reader = s.clone();
    let result = tokio::spawn(async move {
        reader
            .read_deposit_address(
                StockDepositAddressRequest {
                    asset: "MU.US".into(),
                },
                &realtime::WsHub::new(8),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), mock.entered.notified())
        .await
        .unwrap();
    {
        let mut snapshot = s.snapshot.write();
        s.generation.fetch_add(1, Ordering::SeqCst);
        snapshot.security.as_mut().unwrap().asset = "SNDK.US".into();
    }
    gate.notify_one();
    assert!(result.await.unwrap().is_err());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    assert!(s.snapshot().deposit_address.is_none());
    assert!(!path.exists());
}

#[test]
fn stock_funding_official_public_assets_keep_native_units_and_reject_duplicate_mapping() {
    let bytes = include_bytes!("fixtures/assets.json");
    let (stock, funding) = protocol::asset_context(bytes, "MU.US").unwrap();
    assert_eq!(stock[0].minimum_deposit.as_deref(), Some("0.0006"));
    assert_eq!(stock[0].minimum_withdrawal.as_deref(), Some("0.0012"));
    assert_eq!(stock[0].withdrawal_fee.as_deref(), Some("0.0006"));
    let usdc = funding.iter().find(|a| a.asset == "USDC").unwrap();
    assert_eq!(usdc.tokens[0].minimum_deposit.as_deref(), Some("0.5"));
    assert_eq!(usdc.tokens[0].withdrawal_fee.as_deref(), Some("0.5"));
    let sol = funding.iter().find(|a| a.asset == "SOL").unwrap();
    assert_eq!(sol.tokens[0].contract_address.as_deref(), Some("So1"));
    assert_eq!(sol.tokens[0].native_decimals, Some(9));
    assert_eq!(
        protocol::tokens(bytes, "SNDK.US").unwrap()[0]
            .withdrawal_fee
            .as_deref(),
        Some("0.0004")
    );
    let mut bad: Vec<Value> = serde_json::from_slice(bytes).unwrap();
    bad.push(bad[1].clone());
    assert!(protocol::asset_context(&serde_json::to_vec(&bad).unwrap(), "MU.US").is_err());
    assert!(protocol::asset_context(b"[]", "MU.US")
        .unwrap()
        .1
        .is_empty());
}
