//! Binance server time calibration.

use super::binance_public_rest as public_rest;
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use common::time::now_ms;
use std::sync::atomic::{AtomicI64, Ordering};

pub(super) async fn sync_server_time(
    http: &HttpClient,
    base_url: &str,
    offset_ms: &AtomicI64,
    synced_at_ms: &AtomicI64,
    ttl_ms: i64,
) -> ExchangeResult<()> {
    let last = synced_at_ms.load(Ordering::Relaxed);
    let now = now_ms();
    if last != 0 && now.saturating_sub(last) < ttl_ms {
        return Ok(());
    }

    let local_before = now_ms();
    let server = public_rest::server_time(http, base_url).await?;
    let local_after = now_ms();
    // The shared weighted limiter may queue this request before transport. Using
    // the midpoint would turn that local queue time into a false positive clock
    // offset. Binance reports server time at request handling, so the receive
    // timestamp is the conservative reference and keeps signed requests behind
    // server time rather than accidentally more than one second ahead.
    let offset = conservative_offset_ms(server.server_time, local_after);
    let round_trip_ms = local_after.saturating_sub(local_before);

    offset_ms.store(offset, Ordering::Relaxed);
    synced_at_ms.store(now_ms(), Ordering::Relaxed);
    tracing::debug!(
        offset_ms = offset,
        round_trip_ms,
        "binance server time synced"
    );
    Ok(())
}

fn conservative_offset_ms(server_time_ms: i64, local_after_ms: i64) -> i64 {
    server_time_ms.saturating_sub(local_after_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    struct NoQueryParams;

    impl Match for NoQueryParams {
        fn matches(&self, request: &Request) -> bool {
            request.url.query().is_none()
        }
    }

    #[test]
    fn calibration_does_not_treat_pre_send_queue_time_as_clock_skew() {
        let local_before = 1_000;
        let server_at_response = 11_000;
        let local_after = 11_100;

        assert_eq!(
            conservative_offset_ms(server_at_response, local_after),
            -100
        );
        assert_ne!(
            server_at_response - ((local_before + local_after) / 2),
            -100
        );
    }

    #[tokio::test]
    async fn server_time_parses_official_fixture_and_uses_no_query() {
        let server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/binance/usdm_server_time.json");
        let body: serde_json::Value = serde_json::from_str(fixture).expect("server time fixture");

        Mock::given(method("GET"))
            .and(path("/fapi/v1/time"))
            .and(NoQueryParams)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let http = HttpClient::builder("binance")
            .timeout_secs(5)
            .build()
            .expect("http client");
        let offset_ms = AtomicI64::new(0);
        let synced_at_ms = AtomicI64::new(0);

        sync_server_time(&http, &server.uri(), &offset_ms, &synced_at_ms, 0)
            .await
            .expect("server time sync");

        assert_ne!(offset_ms.load(Ordering::Relaxed), 0);
        assert!(synced_at_ms.load(Ordering::Relaxed) > 0);
    }
}
