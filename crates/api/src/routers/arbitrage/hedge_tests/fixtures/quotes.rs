use super::*;

pub(super) fn leg_quote(role: HedgeLegRole, side: OrderSide) -> HedgeLegQuote {
    HedgeLegQuote {
        role,
        exchange: "okx".into(),
        symbol: "BTCUSDT".into(),
        side,
        reference_price: Some(100.0),
        bid: Some(99.9),
        ask: Some(100.1),
        mid: Some(100.0),
        open_vwap_price: Some(100.1),
        open_slippage_bps: Some(0.0),
        close_vwap_price: Some(99.9),
        close_slippage_bps: Some(0.0),
        depth_usd_5bps: Some(1_000.0),
        depth_usd_10bps: Some(1_500.0),
        depth_usd_20bps: Some(2_000.0),
        max_notional_usd: Some(1_000.0),
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: Some(0.0),
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: Some(1),
        blockers: Vec::new(),
    }
}
