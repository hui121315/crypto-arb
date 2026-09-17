use super::*;

#[test]
fn live_candidate_venue_requires_supported_normalized_adapter() {
    assert_eq!(live_request_venue(" Binance "), Some("binance".to_owned()));
    assert_eq!(live_request_venue("unknown"), None);
    assert_eq!(live_request_venue(" "), None);
}
