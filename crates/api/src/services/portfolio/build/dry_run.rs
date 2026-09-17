use super::super::*;
use std::collections::BTreeMap;

pub(super) fn rows_from_dry_run_orders(
    orders: &[OrderRecord],
    funding: &PositionFundingIndex,
    marks: &HashMap<(String, String), f64>,
) -> Vec<PositionRow> {
    let mut positions = BTreeMap::<(String, String), DryRunPosition>::new();
    let mut filled_orders = orders
        .iter()
        .filter(|order| is_filled_dry_run(order))
        .collect::<Vec<_>>();
    filled_orders.sort_by(|left, right| {
        left.updated_at_ms
            .cmp(&right.updated_at_ms)
            .then_with(|| left.intent.created_at_ms.cmp(&right.intent.created_at_ms))
    });
    for order in filled_orders {
        let Some(fill) = DryRunFill::from_order(order) else {
            continue;
        };
        let symbol = exchange::strip_common_suffixes(&order.intent.symbol);
        positions
            .entry((order.intent.exchange.clone(), symbol))
            .or_default()
            .add(fill);
    }
    positions
        .into_iter()
        .filter_map(|((venue, symbol), position)| {
            row_from_dry_run_position(&venue, &symbol, &position, funding, marks)
        })
        .collect()
}

fn is_filled_dry_run(order: &OrderRecord) -> bool {
    order.intent.mode == ExecutionMode::DryRun && order.state == LiveOrderState::Filled
}

#[derive(Debug, Clone, Copy)]
struct DryRunFill {
    sign: f64,
    quantity: f64,
    price: f64,
    leverage: f64,
    reduce_only: bool,
}

impl DryRunFill {
    fn from_order(order: &OrderRecord) -> Option<Self> {
        let quantity = order.filled_quantity.unwrap_or(order.intent.quantity);
        let price = order.filled_price.or(order.intent.price)?;
        valid_fill(quantity, price).then_some(Self {
            sign: side_sign(order.intent.side),
            quantity,
            price,
            leverage: valid_positive(order.intent.leverage).unwrap_or(1.0),
            reduce_only: order.intent.reduce_only,
        })
    }
}

#[derive(Debug, Default)]
struct DryRunPosition {
    signed_quantity: f64,
    entry_price: f64,
    leverage: f64,
}

impl DryRunPosition {
    fn add(&mut self, fill: DryRunFill) {
        if self.signed_quantity.abs() <= f64::EPSILON {
            if !fill.reduce_only {
                self.open(fill.sign, fill.quantity, fill.price, fill.leverage);
            }
            return;
        }
        if self.signed_quantity.signum() == fill.sign {
            if !fill.reduce_only {
                self.add_same_side(fill);
            }
            return;
        }

        let open_quantity = self.signed_quantity.abs();
        if fill.quantity + f64::EPSILON < open_quantity {
            self.signed_quantity += fill.sign * fill.quantity;
        } else if fill.reduce_only || (fill.quantity - open_quantity).abs() <= f64::EPSILON {
            self.clear();
        } else {
            self.open(
                fill.sign,
                fill.quantity - open_quantity,
                fill.price,
                fill.leverage,
            );
        }
    }

    fn add_same_side(&mut self, fill: DryRunFill) {
        let open_quantity = self.signed_quantity.abs();
        let open_notional = open_quantity * self.entry_price;
        let added_notional = fill.quantity * fill.price;
        let total_quantity = open_quantity + fill.quantity;
        let total_notional = open_notional + added_notional;
        self.entry_price = total_notional / total_quantity;
        self.leverage = if total_notional > f64::EPSILON {
            (open_notional * self.leverage + added_notional * fill.leverage) / total_notional
        } else {
            1.0
        };
        self.signed_quantity += fill.sign * fill.quantity;
    }

    fn open(&mut self, sign: f64, quantity: f64, price: f64, leverage: f64) {
        self.signed_quantity = sign * quantity;
        self.entry_price = price;
        self.leverage = leverage.max(1.0);
    }

    fn clear(&mut self) {
        self.signed_quantity = 0.0;
        self.entry_price = 0.0;
        self.leverage = 0.0;
    }
}

fn row_from_dry_run_position(
    venue: &str,
    symbol: &str,
    position: &DryRunPosition,
    funding: &PositionFundingIndex,
    marks: &HashMap<(String, String), f64>,
) -> Option<PositionRow> {
    if position.signed_quantity.abs() <= f64::EPSILON {
        return None;
    }
    let quantity = position.signed_quantity.abs();
    let entry_price = position.entry_price;
    let leverage = position.leverage.max(1.0);
    let symbol = exchange::strip_common_suffixes(symbol);
    let mark_price = marks
        .get(&(normalized_venue_name(venue), symbol.to_ascii_uppercase()))
        .copied()
        .filter(|price| price.is_finite() && *price > 0.0)
        .unwrap_or(entry_price);
    let margin_usd = quantity * mark_price / leverage;
    let unrealized_pnl_usd = position.signed_quantity * (mark_price - entry_price);
    let funding_evidence = funding_evidence_for_key(venue, &symbol, funding);
    let funding_rate_8h = funding_evidence.and_then(|evidence| evidence.rate_8h);
    Some(PositionRow {
        venue: venue.to_owned(),
        symbol,
        origin: PositionOrigin::ExecutionLedger,
        side: dry_run_side(position.signed_quantity),
        quantity,
        entry_price,
        mark_price,
        leverage,
        unrealized_pnl_usd,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: funding_evidence.and_then(|evidence| evidence.next_funding_ms),
        funding_rate_8h: funding_rate_8h.unwrap_or(0.0),
        funding_rate_verified: funding_rate_8h.is_some(),
        maintenance_margin_ratio: 0.0,
        pair_evidence: None,
        paired_with: None,
        margin_usd,
        severity: PositionSeverity::Ok,
        seconds_until_funding: None,
    })
}

fn dry_run_side(signed_quantity: f64) -> PositionSide {
    if signed_quantity > 0.0 {
        PositionSide::Long
    } else {
        PositionSide::Short
    }
}

fn valid_fill(quantity: f64, price: f64) -> bool {
    quantity.is_finite() && price.is_finite() && quantity > f64::EPSILON && price > 0.0
}

fn side_sign(side: OrderSide) -> f64 {
    match side {
        OrderSide::Buy => 1.0,
        OrderSide::Sell => -1.0,
    }
}
