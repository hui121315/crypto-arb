use super::*;

pub(super) fn reference_price(book: &OrderBookInfo, side: OrderSide) -> Option<f64> {
    match side {
        OrderSide::Buy => book.best_ask(),
        OrderSide::Sell => book.best_bid(),
    }
}

pub(super) fn reverse_side(side: OrderSide) -> OrderSide {
    match side {
        OrderSide::Buy => OrderSide::Sell,
        OrderSide::Sell => OrderSide::Buy,
    }
}

pub(super) fn vwap_price_for_notional(
    book: &OrderBookInfo,
    side: OrderSide,
    target_notional: f64,
) -> Option<f64> {
    if target_notional <= f64::EPSILON {
        return None;
    }
    let levels = match side {
        OrderSide::Buy => &book.asks,
        OrderSide::Sell => &book.bids,
    };
    let mut remaining = target_notional;
    let mut quantity = 0.0;
    for level in levels {
        if level[0] <= f64::EPSILON || level[1] <= f64::EPSILON {
            continue;
        }
        let level_notional = notional(level);
        let take_notional = level_notional.min(remaining);
        quantity += take_notional / level[0];
        remaining -= take_notional;
        if remaining <= f64::EPSILON {
            return Some(target_notional / quantity);
        }
    }
    None
}

pub(super) fn vwap_price_for_base_quantity(
    book: &OrderBookInfo,
    side: OrderSide,
    target_base_quantity: f64,
) -> Option<f64> {
    if target_base_quantity <= f64::EPSILON {
        return None;
    }
    let levels = match side {
        OrderSide::Buy => &book.asks,
        OrderSide::Sell => &book.bids,
    };
    let mut remaining = target_base_quantity;
    let mut notional = 0.0;
    for level in levels {
        if level[0] <= f64::EPSILON || level[1] <= f64::EPSILON {
            continue;
        }
        let take_quantity = level[1].min(remaining);
        notional += take_quantity * level[0];
        remaining -= take_quantity;
        if remaining <= f64::EPSILON {
            return Some(notional / target_base_quantity);
        }
    }
    None
}

pub(super) fn slippage_bps(
    reference: Option<f64>,
    vwap: Option<f64>,
    side: OrderSide,
) -> Option<f64> {
    let reference = reference.filter(|price| price.is_finite() && *price > f64::EPSILON)?;
    let vwap = vwap.filter(|price| price.is_finite() && *price > f64::EPSILON)?;
    let raw = match side {
        OrderSide::Buy => vwap / reference - 1.0,
        OrderSide::Sell => 1.0 - vwap / reference,
    };
    Some((raw.max(0.0) * 10_000.0).max(0.0))
}

pub(super) fn depth_usd_within_bps(
    book: &OrderBookInfo,
    side: OrderSide,
    reference: Option<f64>,
    depth_bps: f64,
) -> Option<f64> {
    let reference = reference.filter(|price| price.is_finite() && *price > 0.0)?;
    let threshold = match side {
        OrderSide::Buy => reference * (1.0 + depth_bps / 10_000.0),
        OrderSide::Sell => reference * (1.0 - depth_bps / 10_000.0),
    };
    let levels = match side {
        OrderSide::Buy => &book.asks,
        OrderSide::Sell => &book.bids,
    };
    Some(
        levels
            .iter()
            .filter(|level| in_band(**level, side, threshold))
            .map(notional)
            .sum(),
    )
}

pub(super) fn in_band(level: [f64; 2], side: OrderSide, threshold: f64) -> bool {
    match side {
        OrderSide::Buy => level[0] <= threshold,
        OrderSide::Sell => level[0] >= threshold,
    }
}

pub(super) fn notional(level: &[f64; 2]) -> f64 {
    (level[0] * level[1]).max(0.0)
}

pub(super) fn mid_price(book: &OrderBookInfo) -> Option<f64> {
    Some((book.best_bid()? + book.best_ask()?) / 2.0)
}

pub(super) fn stale_market(book: Option<&OrderBookInfo>, now_ms: i64) -> bool {
    book.map(|book| book.timestamp <= 0 || now_ms.saturating_sub(book.timestamp) > STALE_MARKET_MS)
        .unwrap_or(false)
}

pub(super) fn dedup(values: Vec<String>) -> Vec<String> {
    // Blocker/guard lists are tiny (single ticket scope), so Vec keeps this deterministic.
    let mut out = Vec::new();
    for value in values {
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    }
    out
}
