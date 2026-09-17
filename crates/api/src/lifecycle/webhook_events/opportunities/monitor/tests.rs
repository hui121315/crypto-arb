use super::*;

#[test]
fn blocked_transfer_payload_is_explicit_and_deduplicated() {
    let blocked = CandidateTransferStatus::Blocked {
        detail: "共同网络当前暂停充提".to_owned(),
    };
    let payload = candidate_transfer_payload(&blocked);
    assert_eq!(payload["state"], "blocked");
    assert_eq!(payload["available"], false);
    assert_eq!(
        monitor_event_key("opportunity-1", &blocked),
        monitor_event_key("opportunity-1", &blocked)
    );
    assert_ne!(
        monitor_event_key("opportunity-1", &blocked),
        monitor_event_key(
            "opportunity-1",
            &CandidateTransferStatus::Available {
                detail: "充提可用".to_owned(),
                base_network: "bitcoin".to_owned(),
                quote_network: "tron".to_owned(),
                requires_tag: false,
            }
        )
    );
}

#[test]
fn monitor_key_ignores_dynamic_detail_but_tracks_real_route_changes() {
    let first = CandidateTransferStatus::Available {
        detail: "目标规模成本约 0.120%".to_owned(),
        base_network: "Ethereum".to_owned(),
        quote_network: "TRON".to_owned(),
        requires_tag: false,
    };
    let price_moved = CandidateTransferStatus::Available {
        detail: "目标规模成本约 0.121%".to_owned(),
        base_network: "ethereum".to_owned(),
        quote_network: "tron".to_owned(),
        requires_tag: false,
    };
    let route_changed = CandidateTransferStatus::Available {
        detail: "目标规模成本约 0.121%".to_owned(),
        base_network: "arbitrum".to_owned(),
        quote_network: "tron".to_owned(),
        requires_tag: false,
    };

    assert_eq!(
        monitor_event_key("opportunity-1", &first),
        monitor_event_key("opportunity-1", &price_moved)
    );
    assert_ne!(
        monitor_event_key("opportunity-1", &first),
        monitor_event_key("opportunity-1", &route_changed)
    );

    let blocked_a = CandidateTransferStatus::Blocked {
        detail: "提币暂停".to_owned(),
    };
    let blocked_b = CandidateTransferStatus::Blocked {
        detail: "充值暂停".to_owned(),
    };
    assert_eq!(
        monitor_event_key("opportunity-1", &blocked_a),
        monitor_event_key("opportunity-1", &blocked_b)
    );
}

#[test]
fn startup_monitor_arms_only_after_candidate_set_stops_changing() {
    let mut cursor = Cursor::default();
    let first = std::collections::HashMap::from([("opportunity-1".to_owned(), 1_000)]);
    let second = std::collections::HashMap::from([
        ("opportunity-1".to_owned(), 20_000),
        ("opportunity-2".to_owned(), 20_000),
    ]);

    settle_monitor_baseline(&mut cursor, first.clone(), 1_000);
    assert!(!cursor.opportunity_monitor_initialized);
    assert_eq!(cursor.opportunity_monitor_arm_after_ms, 31_000);

    settle_monitor_baseline(&mut cursor, second, 20_000);
    assert!(!cursor.opportunity_monitor_initialized);
    assert_eq!(cursor.opportunity_monitor_arm_after_ms, 50_000);

    settle_monitor_baseline(&mut cursor, first.clone(), 40_000);
    assert!(!cursor.opportunity_monitor_initialized);
    assert_eq!(cursor.opportunity_monitor_arm_after_ms, 70_000);

    settle_monitor_baseline(&mut cursor, first, 70_000);
    assert!(cursor.opportunity_monitor_initialized);
}

#[test]
fn startup_monitor_has_a_bounded_baseline_window() {
    let mut cursor = Cursor {
        opportunity_monitor_force_arm_after_ms: 180_000,
        ..Cursor::default()
    };
    let first = std::collections::HashMap::from([("opportunity-1".to_owned(), 1_000)]);
    let changed = std::collections::HashMap::from([("opportunity-2".to_owned(), 180_000)]);

    settle_monitor_baseline(&mut cursor, first, 1_000);
    assert!(!cursor.opportunity_monitor_initialized);
    settle_monitor_baseline(&mut cursor, changed, 180_000);
    assert!(cursor.opportunity_monitor_initialized);
    assert!(cursor
        .opportunity_monitor_keys
        .contains_key("opportunity-2"));
}

#[test]
fn transfer_monitor_never_labels_projected_basis_as_deterministic() {
    let projected = monitor_profit_class(Some(StrategyKind::SpotPerp));
    let cross_projected = monitor_profit_class(Some(StrategyKind::CrossSpotPerp));
    let locked = monitor_profit_class(Some(StrategyKind::SpotCross));

    assert_eq!(projected.key(), "projected_basis");
    assert!(!projected.is_locked());
    assert!(!cross_projected.is_locked());
    assert_eq!(locked.key(), "locked_spread");
    assert!(locked.is_locked());
    assert_eq!(
        monitor_conclusion(
            projected,
            &CandidateTransferStatus::NotRequired {
                detail: "同所策略".to_owned(),
            },
            true,
        ),
        "同所无需跨所充提，但基差退出收益尚未锁定"
    );
}
