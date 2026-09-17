use super::*;
use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[tokio::test]
async fn stock_native_valuation_requires_actual_minimum_output_and_bounds_requotes() {
    for succeeds in [true, false] {
        let inputs = Arc::new(Mutex::new(Vec::<u64>::new()));
        let app=Router::new().route("/order",get(
            move |State(inputs):State<Arc<Mutex<Vec<u64>>>>,Query(q):Query<HashMap<String,String>>| async move {
                assert_eq!(q.len(),3);
                assert_eq!(q["inputMint"],comparison::SOLANA_USDC);
                assert_eq!(q["outputMint"],STOCK_WRAPPED_SOL);
                let raw=q["amount"].parse::<u64>().unwrap();
                let index={let mut inputs=inputs.lock().unwrap();inputs.push(raw);inputs.len()};
                let minimum=match (index,succeeds) {(1,_)=>10_000_000,(2,true)=>900_000,(_,true)=>1_000_010,_=>500_000};
                Json(json!({"inputMint":comparison::SOLANA_USDC,"outputMint":STOCK_WRAPPED_SOL,
                    "inAmount":raw.to_string(),"outAmount":(minimum+1_000_000).to_string(),"otherAmountThreshold":minimum.to_string(),
                    "swapMode":"ExactIn","router":"metis","transaction":null,"taker":null}))
            }
        )).with_state(inputs.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/order", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = read_at(&client, &endpoint, Some("local-fixture"), "1000000").await;
        server.abort();
        let _ = server.await;
        if succeeds {
            let result = result.unwrap();
            assert_eq!(*inputs.lock().unwrap(), vec![1_000_000, 100_000, 111_112]);
            assert_eq!(
                result
                    .usdc_budget("1000000", common::time::now_ms())
                    .as_deref(),
                Some("0.111112")
            );
            assert!(next_amount(u64::MAX, u64::MAX, &result.quote).is_err());
        } else {
            assert!(result.is_err());
            assert_eq!(
                *inputs.lock().unwrap(),
                vec![1_000_000, 100_000, 200_000, 400_000]
            );
        }
    }
}

#[tokio::test]
#[ignore = "public quote-only probe, no wallet, key, signing or transaction"]
async fn stock_native_valuation_public_quote_only_probe() {
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(6),
        read_at(
            quote::quote_client(),
            quote::JUPITER_ORDER_ENDPOINT,
            None,
            "2046280",
        ),
    )
    .await
    .expect("bounded public quote")
    .expect("minimum SOL output coverage");
    println!(
        "public SOL replacement quote: {}",
        serde_json::json!({
            "nativeLamports":result.native_lamports,"inputUsdc":result.usdc_budget("2046280",common::time::now_ms()),
            "minimumOutputLamports":result.quote.minimum_output_raw,"router":result.quote.router,"elapsedMs":started.elapsed().as_millis()
        })
    );
}
