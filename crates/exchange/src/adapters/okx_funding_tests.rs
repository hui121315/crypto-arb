use super::*;
use pretty_assertions::assert_eq;

#[derive(serde::Deserialize)]
struct PublicFundingFixture {
    funding_rate: crate::adapters::okx_response::OkxResponse<FundingRateItem>,
}

#[test]
fn parse_funding_with_8h_interval() {
    let item = FundingRateItem {
        inst_id: "BTC-USDT-SWAP".into(),
        funding_rate: "0.0001".into(),
        funding_time: "1700000000000".into(),
        next_funding_time: "1700028800000".into(),
        next_funding_rate: String::new(),
    };
    let row = parse_funding(&item, 1_000_000.0).expect("funding parses");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "okx");
    assert!((row.rate - 0.0001).abs() < 1e-12);
    assert_eq!(row.funding_interval, 8);
    assert!((row.rate_8h - 0.0001).abs() < 1e-12);
    assert_eq!(row.predicted_rate, None);
}

#[test]
fn parse_funding_normalizes_4h_to_8h() {
    let item = FundingRateItem {
        inst_id: "ETH-USDT-SWAP".into(),
        funding_rate: "0.0001".into(),
        funding_time: "1700000000000".into(),
        next_funding_time: "1700014400000".into(),
        next_funding_rate: String::new(),
    };
    let row = parse_funding(&item, 0.0).expect("funding parses");
    assert_eq!(row.funding_interval, 4);
    assert!((row.rate_8h - 0.0002).abs() < 1e-12);
}

#[test]
fn parse_funding_preserves_official_6h_interval() {
    let item = FundingRateItem {
        inst_id: "ALT-USDT-SWAP".into(),
        funding_rate: "0.0001".into(),
        funding_time: "1700000000000".into(),
        next_funding_time: "1700021600000".into(),
        next_funding_rate: String::new(),
    };
    let row = parse_funding(&item, 0.0).expect("funding parses");
    assert_eq!(row.funding_interval, 6);
    assert!((row.rate_8h - (0.0001 * 8.0 / 6.0)).abs() < 1e-12);
}

#[test]
fn parse_funding_extracts_predicted_rate() {
    let item = FundingRateItem {
        inst_id: "BTC-USDT-SWAP".into(),
        funding_rate: "0.0001".into(),
        funding_time: "1700000000000".into(),
        next_funding_time: "1700028800000".into(),
        next_funding_rate: "0.00012".into(),
    };
    let row = parse_funding(&item, 0.0).expect("funding parses");
    assert_eq!(row.predicted_rate, Some(0.000_12));
}

#[test]
fn parse_funding_handles_invalid_predicted_rate() {
    let item = FundingRateItem {
        inst_id: "BTC-USDT-SWAP".into(),
        funding_rate: "0.0001".into(),
        funding_time: "1700000000000".into(),
        next_funding_time: "1700028800000".into(),
        next_funding_rate: "not-a-number".into(),
    };
    let row = parse_funding(&item, 0.0).expect("funding parses");
    assert_eq!(row.predicted_rate, None);
}

#[test]
fn okx_funding_rate_parses_official_fixture_interval() {
    let fixture = include_str!("../../fixtures/okx/public_funding_mark_index_oi_btc_eth_usdt.json");
    let response: PublicFundingFixture =
        serde_json::from_str(fixture).expect("okx public funding fixture");
    let mut rows = response
        .funding_rate
        .into_data("funding-rate")
        .expect("funding rows");
    let item = rows.pop().expect("funding row");
    let parsed = parse_funding(&item, 201_680.534_4).expect("funding parses");

    assert_eq!(parsed.exchange, "okx");
    assert_eq!(parsed.symbol, "BTC");
    assert!((parsed.rate - 0.000_094_705_405_610_4).abs() < 1e-16);
    assert!((parsed.rate_8h - 0.000_094_705_405_610_4).abs() < 1e-16);
    assert_eq!(parsed.predicted_rate, None);
    assert_eq!(parsed.funding_interval, 8);
    assert_eq!(parsed.next_funding_time, 1_780_473_600_000);
    assert_eq!(parsed.timestamp, 1_780_444_800_000);
    assert_eq!(parsed.volume_24h, 201_680.534_4);
}
