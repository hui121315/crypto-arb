use super::super::*;

pub(super) fn hyperliquid_clearinghouse_state(
    dex: Option<&str>,
    account_value: f64,
    total_margin_used: f64,
    withdrawable: f64,
    positions: Vec<hyperliquid_ws_user::HyperliquidPositionDelta>,
) -> hyperliquid_ws_user::HyperliquidClearinghouseState {
    hyperliquid_ws_user::HyperliquidClearinghouseState {
        dex: dex.map(str::to_owned),
        user: "0x0000000000000000000000000000000000000000".to_owned(),
        account_value,
        total_margin_used,
        withdrawable,
        positions,
    }
}
pub(super) fn hyperliquid_dex_state(
    dex: &str,
    account_value: f64,
    positions: Vec<hyperliquid_ws_user::HyperliquidPositionDelta>,
) -> hyperliquid_ws_user::HyperliquidDexClearinghouseState {
    hyperliquid_ws_user::HyperliquidDexClearinghouseState {
        dex: dex.to_owned(),
        state: hyperliquid_clearinghouse_state(
            Some(dex),
            account_value,
            10.0,
            account_value - 10.0,
            positions,
        ),
    }
}
pub(super) fn hyperliquid_position_delta(
    coin: &str,
    side: &str,
    size: f64,
    unrealized_pnl: f64,
) -> hyperliquid_ws_user::HyperliquidPositionDelta {
    hyperliquid_ws_user::HyperliquidPositionDelta {
        coin: coin.to_owned(),
        side: side.to_owned(),
        size,
        entry_price: 100.0,
        liquidation_price: Some(50.0),
        margin_used: 10.0,
        unrealized_pnl,
        leverage: 5.0,
    }
}
pub(super) fn hyperliquid_fill(
    order_id: &str,
    tx_hash: &str,
) -> hyperliquid_ws_user::HyperliquidFill {
    hyperliquid_ws_user::HyperliquidFill {
        venue: "hyperliquid".to_owned(),
        coin: "BTC".to_owned(),
        trade_id: Some(456),
        order_id: order_id.to_owned(),
        side: "B".to_owned(),
        price: 100.0,
        size: 0.25,
        fee: 0.01,
        fee_token: "USDC".to_owned(),
        closed_pnl: 0.0,
        liquidation: None,
        crossed: true,
        time_ms: 42,
        tx_hash: tx_hash.to_owned(),
    }
}
pub(super) fn hyperliquid_funding(
    coin: &str,
    usdc: f64,
    funding_rate: f64,
    time_ms: i64,
) -> hyperliquid_ws_user::HyperliquidFunding {
    hyperliquid_ws_user::HyperliquidFunding {
        venue: "hyperliquid".to_owned(),
        coin: coin.to_owned(),
        usdc,
        size: 1.0,
        funding_rate,
        time_ms,
    }
}
pub(super) fn hyperliquid_liquidation(
    id: i64,
    notional_position: f64,
    account_value: f64,
) -> hyperliquid_ws_user::HyperliquidLiquidation {
    hyperliquid_ws_user::HyperliquidLiquidation {
        id,
        liquidator: "0x1111111111111111111111111111111111111111".to_owned(),
        liquidated_user: "0x2222222222222222222222222222222222222222".to_owned(),
        notional_position,
        account_value,
    }
}
pub(super) fn hyperliquid_non_user_cancel(
    coin: &str,
    order_id: &str,
) -> hyperliquid_ws_user::HyperliquidNonUserCancel {
    hyperliquid_ws_user::HyperliquidNonUserCancel {
        venue: "hyperliquid".to_owned(),
        coin: coin.to_owned(),
        order_id: order_id.to_owned(),
    }
}
pub(super) fn hyperliquid_spot_balance_delta(
    coin: &str,
    total: f64,
    hold: f64,
) -> hyperliquid_ws_user::HyperliquidSpotBalance {
    hyperliquid_ws_user::HyperliquidSpotBalance {
        coin: coin.to_owned(),
        token: 0,
        total,
        hold,
        entry_notional: 0.0,
    }
}
pub(super) fn gate_position_delta(
    symbol: &str,
    side: &str,
    size: f64,
) -> gate_ws_user::GatePositionDelta {
    gate_ws_user::GatePositionDelta {
        symbol: symbol.to_owned(),
        side: side.to_owned(),
        size,
        entry_price: 100.0,
        mark_price: 101.0,
        unrealized_pnl: 1.0,
        leverage: 5.0,
        liquidation_price: Some(50.0),
        margin: 10.0,
        maintenance_margin_ratio: 0.005,
        updated_time_ms: 1_700_000_000_000,
    }
}
pub(super) fn gate_usertrade_delta(
    trade_id: &str,
    order_id: &str,
) -> gate_ws_user::GateUserTradeDelta {
    gate_ws_user::GateUserTradeDelta {
        trade_id: trade_id.to_owned(),
        exchange_order_id: order_id.to_owned(),
        symbol: "BTC".to_owned(),
        quantity: 1.0,
        price: 40_000.4,
        fee: 0.0009290592,
        point_fee: 0.0,
        occurred_at_ms: 1_628_736_848_321,
    }
}
