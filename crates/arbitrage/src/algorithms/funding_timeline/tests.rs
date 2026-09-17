use super::*;
use crate::models::RawOpportunityExtra;
use shared_types::{ArbitrageType, FundingRateData, StrategyKind};

const NOW: i64 = 1_700_000_000_000;

#[test]
fn aligned_one_hour_and_eight_hour_use_each_native_rate_once() {
    let row = raw(
        rate("long", 0.0001, 1, NOW + 10 * 60_000),
        rate("short", 0.0008, 8, NOW + 10 * 60_000 + 10),
    );

    let projection = first_funding_projection(&row).expect("aligned funding window");

    assert_eq!(projection.event_count, 1);
    assert!((projection.funding_yield - 0.0007).abs() < 1e-12);
    assert_eq!(projection.mismatch_reserve_yield, 0.0);
    assert_eq!(projection.settlement_at_ms, NOW + 10 * 60_000 + 10);
    assert!(current_settlements_aligned(&row));
    assert!(assess_entry(&row, NOW).passed);
}

#[test]
fn every_supported_interval_pair_can_pass_when_next_settlement_is_aligned() {
    for (long_interval, short_interval) in [(1, 8), (2, 6), (4, 8), (6, 8), (1, 1)] {
        let row = raw(
            rate("long", 0.0001, long_interval, NOW + HOUR_MS),
            rate("short", 0.0008, short_interval, NOW + HOUR_MS),
        );

        assert!(
            assess_entry(&row, NOW).passed,
            "{long_interval}h/{short_interval}h aligned pair should pass"
        );
    }
}

#[test]
fn current_joint_event_rejects_stale_or_staggered_schedules() {
    let aligned = FundingTimelineInput {
        long_rate: 0.0001,
        long_next_ms: NOW + HOUR_MS,
        long_interval_hours: 1,
        short_rate: 0.0008,
        short_next_ms: NOW + HOUR_MS,
        short_interval_hours: 8,
    };
    assert!(current_joint_event_input(aligned, NOW).is_some());

    let stale = FundingTimelineInput {
        long_next_ms: NOW,
        ..aligned
    };
    assert!(current_joint_event_input(stale, NOW).is_none());

    let staggered = FundingTimelineInput {
        short_next_ms: NOW + 2 * HOUR_MS,
        ..aligned
    };
    assert!(current_joint_event_input(staggered, NOW).is_none());
}

#[test]
fn staggered_events_never_accumulate_into_an_executable_projection() {
    let row = raw(
        rate("long", 0.0001, 1, NOW + HOUR_MS),
        rate("short", 0.0008, 8, NOW + 8 * HOUR_MS),
    );

    assert!(first_funding_projection(&row).is_none());
    let assessment = assess_entry(&row, NOW);
    assert!(!assessment.passed);
    assert!(assessment.detail.contains("下一共同结算"));
}

#[test]
fn aligned_but_implausibly_distant_short_cycle_is_rejected_as_stale() {
    let row = raw(
        rate("long", 0.0001, 1, NOW + 8 * HOUR_MS),
        rate("short", 0.0008, 8, NOW + 8 * HOUR_MS),
    );

    let assessment = assess_entry(&row, NOW);

    assert!(!assessment.passed);
    assert!(assessment.detail.contains("超过对应原生周期"));
}

#[test]
fn one_joint_event_must_cover_the_full_cost() {
    let input = FundingTimelineInput {
        long_rate: 0.0001,
        long_next_ms: NOW + 10 * 60_000,
        long_interval_hours: 1,
        short_rate: 0.0008,
        short_next_ms: NOW + 10 * 60_000,
        short_interval_hours: 8,
    };

    assert!(first_profitable_input(input, 0.0006).is_some());
    assert!(first_profitable_input(input, 0.0007).is_none());
}

fn raw(long_rate: FundingRateData, short_rate: FundingRateData) -> RawOpportunity {
    RawOpportunity {
        symbol: "BTC".into(),
        arb_type: ArbitrageType::CrossExchange,
        long_exchange: "long".into(),
        short_exchange: "short".into(),
        long_rate,
        short_rate,
        spread_8h: 0.0,
        single_yield: 0.0,
        extra: RawOpportunityExtra {
            strategy_kind: Some(StrategyKind::PerpCross),
            ..Default::default()
        },
    }
}

fn rate(exchange: &str, rate: f64, interval: u32, next: i64) -> FundingRateData {
    FundingRateData {
        symbol: "BTCUSDT".into(),
        exchange: exchange.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(interval)),
        predicted_rate: None,
        next_funding_time: next,
        funding_interval: interval,
        volume_24h: 1_000_000.0,
        timestamp: 1,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}
