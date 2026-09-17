use super::*;
use pretty_assertions::assert_eq;

#[test]
fn bullet_response_builds_token_url() {
    let raw = r#"{
        "code":"200000",
        "data":{
            "token":"token-1",
            "instanceServers":[{
                "endpoint":"wss://ws-api-futures.kucoin.com/",
                "protocol":"websocket",
                "pingInterval":10000
            }]
        }
    }"#;
    let wrap: BulletResponse = serde_json::from_str(raw).unwrap();
    let session = wrap.into_session().unwrap();
    assert!(session
        .ws_url
        .starts_with("wss://ws-api-futures.kucoin.com/?token=token-1&connectId=crossline-"));
    assert_eq!(session.ping_interval_ms, 9_000);
}

#[test]
fn bullet_response_propagates_api_error() {
    let raw = r#"{"code":"40000","data":{"token":"","instanceServers":[]}}"#;
    let wrap: BulletResponse = serde_json::from_str(raw).unwrap();
    let err = wrap.into_session().unwrap_err();
    assert!(matches!(err, ExchangeError::Api { .. }));
}

#[test]
fn bullet_response_requires_websocket_protocol() {
    let raw = r#"{
        "code":"200000",
        "data":{
            "token":"t",
            "instanceServers":[{"endpoint":"https://example","protocol":"http"}]
        }
    }"#;
    let wrap: BulletResponse = serde_json::from_str(raw).unwrap();
    let err = wrap.into_session().unwrap_err();
    assert!(matches!(err, ExchangeError::Parse(_)));
}
