use super::*;

#[test]
fn funding_interval_map_keeps_only_positive_non_default_intervals() {
    let items: Vec<FundingInfoItem> = serde_json::from_str(
        r#"[
            {"symbol":"BTCUSDT","fundingIntervalHours":8},
            {"symbol":"ALTUSDT","fundingIntervalHours":4},
            {"symbol":"FASTUSDT","fundingIntervalHours":-1},
            {"symbol":"ZEROUSDT","fundingIntervalHours":0}
        ]"#,
    )
    .expect("test fundingInfo json is valid");

    let map = funding_interval_map(items);

    assert_eq!(map.len(), 1);
    assert_eq!(map.get("ALTUSDT"), Some(&4));
    assert!(!map.contains_key("BTCUSDT"));
    assert!(!map.contains_key("FASTUSDT"));
    assert!(!map.contains_key("ZEROUSDT"));
}

#[test]
fn cache_defaults_missing_symbols_to_8h() {
    let cache = FundingIntervalCache::default();
    let mut intervals = HashMap::new();
    intervals.insert("ALTUSDT".to_owned(), 4);

    cache.replace(intervals, 1_000);

    assert!(cache.is_fresh(1_001));
    assert_eq!(cache.interval_for("ALTUSDT"), 4);
    assert_eq!(cache.interval_for("BTCUSDT"), 8);
}
