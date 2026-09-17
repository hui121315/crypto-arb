use super::*;

#[test]
fn publish_replaces_the_whole_generation_atomically() {
    let index = OpportunityIndex::default();
    index.publish("snap-1".into(), 10, &[]);
    index.publish("snap-2".into(), 20, &[]);

    let health = index.health(25);
    assert_eq!(health.version, 2);
    assert_eq!(health.snapshot_id.as_deref(), Some("snap-2"));
    assert_eq!(health.published_at_ms, Some(20));
    assert_eq!(health.freshness_ms, Some(5));
    assert_eq!(health.rows, 0);
}

#[test]
fn report_entry_and_id_lookup_share_one_published_generation() -> serde_json::Result<()> {
    let index = OpportunityIndex::default();
    let updates = index.subscribe_updates();
    let first = opportunity("first")?;
    let second = opportunity("second")?;

    index.publish("snap-shared".into(), 20, &[first, second]);

    assert!(updates.has_changed().unwrap_or(false));
    let first_entry = required(index.entry(), "published report entry")?;
    let second_entry = required(index.entry(), "same published report entry")?;
    assert!(Arc::ptr_eq(&first_entry, &second_entry));
    assert_eq!(
        index.current("second").map(|row| row.id),
        Some("second".to_owned())
    );
    assert_eq!(
        first_entry
            .value
            .opportunities
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    Ok(())
}

#[test]
fn stale_list_snapshot_fails_closed() {
    let index = OpportunityIndex::default();
    index.publish("snap-2".into(), 20, &[]);

    let mismatch = index.get_bound("a", Some("snap-1"));
    assert_eq!(
        mismatch.as_ref().err(),
        Some(&OpportunitySnapshotMismatch {
            expected: "snap-1".into(),
            actual: "snap-2".into(),
        })
    );
}

#[test]
fn read_preserves_ranked_publish_order_and_version() -> serde_json::Result<()> {
    let index = OpportunityIndex::default();
    let first = opportunity("first")?;
    let second = opportunity("second")?;

    let version = index.publish("snap-ranked".into(), 20, &[first, second]);
    let view = required(index.read(), "published snapshot")?;

    assert_eq!(view.version(), version);
    assert_eq!(index.version(), version);
    assert_eq!(
        view.rows()
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    Ok(())
}

#[test]
fn read_reuses_the_published_rows_without_cloning() -> serde_json::Result<()> {
    let index = OpportunityIndex::default();
    let first = opportunity("first")?;
    let second = opportunity("second")?;
    let third = opportunity("third")?;
    let version = index.publish("snap-selected".into(), 20, &[first, second, third]);

    let view = required(index.read(), "published snapshot")?;
    let entry = required(index.entry(), "published report entry")?;

    assert_eq!(view.version(), version);
    assert_eq!(view.snapshot_id(), "snap-selected");
    assert_eq!(
        view.rows().as_ptr(),
        entry.value.opportunities.as_ptr(),
        "read path must borrow the published generation"
    );
    assert_eq!(
        view.rows()
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second", "third"]
    );
    Ok(())
}

fn opportunity(id: &str) -> serde_json::Result<ArbitrageOpportunityDto> {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "symbol": "BTC",
        "type": "cross_exchange",
        "typeLabel": "test",
        "longExchange": "binance",
        "shortExchange": "okx",
        "spread8h": 0.0,
        "longRate8h": 0.0,
        "shortRate8h": 0.0,
        "longRate": 0.0,
        "shortRate": 0.0,
        "singleYield": 0.0,
        "netSingleYield": 0.0,
        "rawSingleYield": 0.0,
        "settlementInterval": 8,
        "riskAdjustedYield": 0.0,
        "tradingCostRate": 0.0,
        "minHoldingPeriods": 1,
        "riskLevel": "low",
        "volatility": 0.0,
        "sharpeRatio": 0.0,
        "score": 0.0,
        "recommendation": "hold",
        "optimalPosition": 0.0,
        "maxPosition": 0.0,
        "liquidityScore": 0.0,
        "volume24h": 0.0,
        "dataSource": "test",
        "confidence": 0.0,
        "updatedAt": "2026-07-31T00:00:00Z",
        "longFundingInterval": 8,
        "shortFundingInterval": 8
    }))
}

fn required<T>(value: Option<T>, message: &'static str) -> serde_json::Result<T> {
    value.ok_or_else(|| serde_json::Error::io(std::io::Error::other(message)))
}
