use super::*;
use crate::panels::modules::opportunity_format::{missing_quote_label, missing_quote_text};

#[test]
fn formats_reference_price_before_fallback() {
    assert_eq!(leg_price_text(Some(123.456), "99"), "123.46");
}

#[test]
fn falls_back_to_opportunity_price() {
    assert_eq!(leg_price_text(None, "0.123456789"), "0.12345679");
}

#[test]
fn marks_missing_price_as_waiting_quote() {
    assert_eq!(leg_price_text(None, "-"), missing_quote_text(None));
    assert_eq!(
        leg_price_text(Some(0.0), missing_quote_label()),
        missing_quote_text(None)
    );
}

#[test]
fn formats_leg_price_evidence_without_inventing_source() {
    use shared_types::{MarketDataHealth, MarketDataQuality, MarketDataSourceKind};

    let evidence = OpportunityLegMarketEvidence {
        venue: "hyperliquid:xyz".into(),
        symbol: "MU".into(),
        price: Some(100.0),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(9),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: None,
            problem: None,
        },
    };

    assert_eq!(
        leg_market_evidence_text(Some(&evidence)),
        "证据 新鲜 · WS · 9ms"
    );
    assert_eq!(leg_market_evidence_text(None), "缺腿级行情证据");
}

#[test]
fn empty_ticket_does_not_render_faux_metrics() {
    let selection = ExecutionSelection::empty();

    assert_eq!(ticket_heading(&selection), "等待选择套利机会");
    assert_eq!(
        selection_metric(&selection, |current| format!(
            "{:+.3}%",
            current.one_cycle_net_bps / 100.0
        )),
        "-"
    );
}
