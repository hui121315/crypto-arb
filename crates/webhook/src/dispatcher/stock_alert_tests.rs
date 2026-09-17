use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn stock_alert_local_receiver_retries_same_bark_id_and_requires_application_ack() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/fixture", listener.local_addr().unwrap());
    let receiver = tokio::spawn(async move {
        let mut messages = Vec::new();
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut chunk = [0; 2048];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                assert!(bytes.len() < 32_768);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break bytes[end + 4..end + 4 + length].to_vec();
                    }
                }
            };
            messages.push(serde_json::from_slice::<serde_json::Value>(&body).unwrap());
            if attempt == 0 {
                // Lost response: the same stable notification id must be retried.
                continue;
            }
            let response = r#"{"code":200,"message":"success"}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        messages
    });
    let event = WebhookEvent {
        id: "stock-spread-fixture-100".into(),
        version: shared_types::WEBHOOK_EVENT_VERSION.into(),
        kind: WebhookEventKind::StockSpread,
        occurred_at_ms: 1000,
        payload: serde_json::json!({"message":"MU · 链买 / Backpack 卖\n报价差额 +0.5%\n库存未核验；仅观察，不是已锁定利润。", "completeNetUsdc":null,"executable":false}),
    };
    let generic: serde_json::Value =
        serde_json::from_slice(&delivery_body(WebhookProvider::Generic, &event).unwrap()).unwrap();
    assert_eq!(generic["kind"], "stock_spread");
    assert_eq!(generic["payload"]["executable"], false);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let body = delivery_body(WebhookProvider::Bark, &event).unwrap();
    // The injected local transport does not relax production HTTPS/SSRF checks.
    let outcome = attempt_delivery(WebhookProvider::Bark, 3, 0, || {
        let client = client.clone();
        let endpoint = endpoint.clone();
        let body = body.clone();
        async move {
            let response = client
                .post(endpoint)
                .header("Connection", "close")
                .body(body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let bytes = response.bytes().await.map_err(|e| e.to_string())?;
            let (application_ack, message) = application_ack(WebhookProvider::Bark, &bytes);
            Ok(DeliveryResponse {
                status,
                provider: WebhookProvider::Bark,
                application_ack,
                message,
            })
        }
    })
    .await;
    let messages = tokio::time::timeout(std::time::Duration::from_secs(3), receiver)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], messages[1]);
    assert_eq!(messages[0]["id"], bark_collapse_id(&event.id));
    assert_eq!(messages[0]["title"], "CROSSLINE · 股票价差观察");
    assert!(messages[0]["body"].as_str().unwrap().contains("仅观察"));
    assert!(outcome.delivered);
    assert_eq!(outcome.attempts, 2);
    assert_eq!(outcome.application_ack, WebhookApplicationAck::Accepted);
    assert_eq!(
        application_ack(WebhookProvider::Bark, br#"{"code":400}"#).0,
        WebhookApplicationAck::Rejected
    );
}
