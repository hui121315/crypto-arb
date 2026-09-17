#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `HttpClient` 集成测试：覆盖 200 / 401 / 429+Retry-After / 5xx 重试 / 超时 五场景。

use exchange::{http_outcome_metrics_snapshot, ExchangeError, HttpClient};
use reqwest::Method;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn returns_2xx_directly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .mount(&server)
        .await;

    let client = HttpClient::builder("test-2xx")
        .timeout_secs(5)
        .build()
        .unwrap();

    let url = format!("{}/ok", server.uri());
    let resp = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect("should succeed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "hello");
}

#[tokio::test]
async fn returns_4xx_without_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .expect(1) // 4xx 不应重试
        .mount(&server)
        .await;

    let client = HttpClient::builder("test-4xx")
        .max_retries(3)
        .build()
        .unwrap();

    let url = format!("{}/auth", server.uri());
    let resp = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect("4xx is returned, not error");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn retries_on_429_with_retry_after() {
    let server = MockServer::start().await;
    // 前 1 次 429 + Retry-After: 1，第 2 次 200
    Mock::given(method("GET"))
        .and(path("/limit"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_string("rate limited"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/limit"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = HttpClient::builder("test-429")
        .max_retries(3)
        .base_backoff_ms(50)
        .build()
        .unwrap();

    let url = format!("{}/limit", server.uri());
    let resp = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect("should retry then succeed");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn retries_on_5xx_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(503).set_body_string("temp"))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = HttpClient::builder("test-5xx-retry")
        .max_retries(5)
        .base_backoff_ms(20)
        .build()
        .unwrap();

    let url = format!("{}/flaky", server.uri());
    let resp = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect("eventually succeeds");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn server_5xx_exhausts_retries_returns_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dead"))
        .respond_with(ResponseTemplate::new(500).set_body_string("oops"))
        .mount(&server)
        .await;

    let client = HttpClient::builder("test-5xx-exhaust")
        .max_retries(2)
        .base_backoff_ms(10)
        .build()
        .unwrap();

    let url = format!("{}/dead", server.uri());
    let err = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect_err("max retries exhausted");
    match err {
        ExchangeError::Http { status, .. } => assert_eq!(status, 500),
        other => panic!("expected Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn server_5xx_body_read_failure_is_reported() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 512];
        let _ = stream.read(&mut request).await;
        stream
            .write_all(
                b"HTTP/1.1 500 Internal Server Error\r\n\
                  Content-Length: 64\r\n\
                  Connection: close\r\n\
                  \r\n\
                  short",
            )
            .await
            .unwrap();
    });

    let client = HttpClient::builder("test-5xx-body-read")
        .max_retries(1)
        .base_backoff_ms(1)
        .build()
        .unwrap();
    let url = format!("http://{addr}/broken-body");
    let err = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect_err("truncated 5xx body should return an http error");
    server.await.unwrap();

    match err {
        ExchangeError::Http { status, body } => {
            assert_eq!(status, 500);
            assert!(body.contains("response body read failed"));
        }
        other => panic!("expected Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn timeout_returns_timeout_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;

    let client = HttpClient::builder("test-timeout")
        .timeout_secs(1)
        .max_retries(1)
        .base_backoff_ms(10)
        .build()
        .unwrap();

    let url = format!("{}/slow", server.uri());
    let err = client
        .execute_with_retry(|| client.request(Method::GET, &url))
        .await
        .expect_err("should timeout");
    assert!(matches!(err, ExchangeError::Timeout { .. }));
}

#[tokio::test]
async fn transport_rtt_stops_at_response_headers_before_body_wait() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (headers_sent, headers_seen) = tokio::sync::oneshot::channel();
    let (release_body, body_released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 512];
        let _ = stream.read(&mut request).await;
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\n\
                  Content-Length: 4\r\n\
                  Connection: close\r\n\
                  \r\n",
            )
            .await
            .unwrap();
        stream.flush().await.unwrap();
        headers_sent.send(()).unwrap();
        body_released.await.unwrap();
        stream.write_all(b"pong").await.unwrap();
    });

    let client = HttpClient::builder("pr_bk_transport_rtt")
        .timeout_secs(5)
        .max_retries(1)
        .build()
        .unwrap();
    let url = format!("http://{addr}/headers-before-body");
    let request = tokio::spawn(async move {
        client
            .execute_with_retry(|| client.request(Method::GET, &url))
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), headers_seen)
        .await
        .expect("server should send response headers")
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), request).await;
    let metric = http_outcome_metrics_snapshot()
        .into_iter()
        .find(|row| row.exchange == "pr_bk_transport_rtt" && row.path == "/headers-before-body");
    release_body.send(()).unwrap();

    let response = response
        .expect("HTTP send boundary must finish before response body is released")
        .unwrap()
        .expect("response headers should produce a successful attempt");
    let metric = metric.expect("transport RTT metric should be recorded at response headers");
    assert_eq!(metric.request_total, 1);
    assert_eq!(response.text().await.unwrap(), "pong");
    server.await.unwrap();
}
