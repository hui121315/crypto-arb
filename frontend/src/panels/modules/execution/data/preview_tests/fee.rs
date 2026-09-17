use super::*;

#[test]
fn fee_evidence_lines_keep_source_and_verification_problem() {
    let manual = TradeFeeSnapshot {
        venue: "binance".into(),
        symbol: "MUUSDT".into(),
        product: FeeProduct::Perp,
        account_id: None,
        maker_fee_bps: 2.0,
        taker_fee_bps: 5.0,
        open_fee_bps: 5.0,
        close_fee_bps: 5.0,
        source: TradeFeeSource::Manual,
        fetched_at_ms: 1,
        valid_until_ms: 2,
        freshness_ms: Some(0),
        evidence: None,
        verification_problem: None,
        note: None,
    };
    let official_problem = TradeFeeSnapshot {
        source: TradeFeeSource::OfficialSchedule,
        verification_problem: Some("fee tier mismatch".into()),
        ..manual.clone()
    };
    let official = TradeFeeSnapshot {
        source: TradeFeeSource::OfficialSchedule,
        freshness_ms: Some(500),
        evidence: Some(TradeFeeEvidence {
            evidence_id: "fee:binance:perp:vip0".into(),
            source_name: "Binance Futures Commission Rate".into(),
            source_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/account/rest-api/User-Commission-Rate".into(),
            checked_at_ms: 1_780_185_600_000,
            effective_at_ms: None,
            schedule_version: Some("2026-06".into()),
            tier: Some("VIP0".into()),
            scope: Some("USDT-M futures".into()),
            problem: None,
        }),
        ..manual.clone()
    };

    let lines = fee_evidence_lines(&[manual, official_problem, official]);

    assert_eq!(lines[0].source, "手工录入");
    assert!(lines[0].health.contains("仅观察"));
    assert_eq!(lines[1].source, "官方费率表");
    assert!(lines[1].health.contains("fee tier mismatch"));
    assert_eq!(lines[2].source, "官方费率表");
    assert!(lines[2].health.contains("fee:binance:perp:vip0"));
    assert!(lines[2].health.contains("Binance Futures Commission Rate"));
    assert!(lines[2].health.contains("https://developers.binance.com"));
    assert!(lines[2].health.contains("checked_at_ms 1780185600000"));
    assert!(lines[2].health.contains("freshness_ms 500"));
    assert!(lines[2].health.contains("tier VIP0"));
    assert!(lines[2].health.contains("scope USDT-M futures"));
    assert!(lines[2].health.contains("version 2026-06"));
}
