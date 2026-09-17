use super::super::*;
use chrono::Utc;
use shared_types::{OrderStatus, OrderType};

pub(super) fn kucoin_balance_delta(
    subject: &str,
    total: f64,
    available: f64,
) -> kucoin_ws_user::KucoinBalanceDelta {
    kucoin_ws_user::KucoinBalanceDelta {
        subject: subject.to_owned(),
        currency: "USDT".to_owned(),
        total,
        available,
        hold_balance: total - available,
        unrealized_pnl: 1.0,
    }
}
pub(super) fn kucoin_position_delta(
    _subject: &str,
    _margin_mode: &str,
    _is_open: bool,
    size: f64,
) -> kucoin_ws_user::KucoinPositionDelta {
    kucoin_ws_user::KucoinPositionDelta::Change {
        native_symbol: "XBTUSDTM".to_owned(),
        current_contracts: size,
        updated_time_ms: 1_700_000_000_000,
    }
}
pub(super) fn okx_balance_delta(
    currency: &str,
    total: f64,
    available: f64,
) -> okx_ws_user::OkxBalanceDelta {
    okx_ws_user::OkxBalanceDelta {
        currency: currency.to_owned(),
        total,
        available,
        frozen: total - available,
        unrealized_pnl: 0.0,
        updated_time_ms: 1_700_000_000_000,
    }
}
pub(super) fn okx_account_summary_delta() -> okx_ws_user::OkxAccountSummaryDelta {
    okx_ws_user::OkxAccountSummaryDelta {
        total_equity_usd: 1_000.0,
        total_available_balance_usd: 950.0,
        total_initial_margin_usd: 100.0,
        total_maintenance_margin_usd: 50.0,
        updated_time_ms: 1_700_000_000_000,
    }
}
pub(super) fn bybit_wallet_account(
    account_type: &str,
    coins: Vec<bybit_ws_user::BybitWalletCoinDelta>,
) -> bybit_ws_user::BybitWalletAccount {
    bybit_wallet_account_with_available(account_type, 950.0, coins)
}
pub(super) fn bybit_wallet_account_with_available(
    account_type: &str,
    total_available_balance: f64,
    coins: Vec<bybit_ws_user::BybitWalletCoinDelta>,
) -> bybit_ws_user::BybitWalletAccount {
    bybit_ws_user::BybitWalletAccount {
        account_type: account_type.to_owned(),
        total_equity: 1_000.0,
        total_available_balance,
        total_initial_margin: 40.0,
        total_maintenance_margin: 20.0,
        account_im_rate: 0.04,
        account_mm_rate: 0.02,
        coins,
    }
}
pub(super) fn bybit_wallet_coin(
    coin: &str,
    equity: f64,
    available_to_withdraw: Option<f64>,
) -> bybit_ws_user::BybitWalletCoinDelta {
    bybit_ws_user::BybitWalletCoinDelta {
        coin: coin.to_owned(),
        equity,
        usd_value: equity,
        wallet_balance: equity,
        available_to_withdraw,
        locked: 0.0,
        unrealized_pnl: 0.0,
    }
}
pub(super) fn bybit_execution_delta(
    order_id: &str,
    exec_id: &str,
    fee_currency: Option<&str>,
) -> bybit_ws_user::BybitExecutionUpdate {
    bybit_ws_user::BybitExecutionUpdate {
        order_id: order_id.to_owned(),
        client_order_id: "cid-1".to_owned(),
        exec_id: exec_id.to_owned(),
        symbol: "BTC".to_owned(),
        category: "linear".to_owned(),
        side: "Sell".to_owned(),
        price: 95_900.1,
        size: 0.5,
        fee: Some(26.3725275),
        fee_currency: fee_currency.map(ToOwned::to_owned),
        fee_rate: Some(0.00055),
        extra_fees: Vec::new(),
        trade_time_ms: 1_746_270_400_353,
        is_maker: Some(false),
        seq: Some(140_612_148_849_382),
    }
}
pub(super) fn bitget_account_delta(
    coin: &str,
    equity: f64,
    available: f64,
    frozen: f64,
    usdt_equity: f64,
    unrealized_pnl: f64,
) -> bitget_ws_user::BitgetAccountDelta {
    bitget_ws_user::BitgetAccountDelta {
        coin: coin.to_owned(),
        frozen,
        available,
        equity,
        usdt_equity,
        unrealized_pnl,
    }
}
pub(super) fn bitget_position_delta(
    symbol: &str,
    margin_coin: &str,
    size: f64,
    side: &str,
) -> bitget_ws_user::BitgetPositionDelta {
    bitget_ws_user::BitgetPositionDelta {
        symbol: symbol.to_owned(),
        margin_coin: margin_coin.to_owned(),
        margin_size: 50.0,
        margin_mode: "crossed".to_owned(),
        hold_mode: "hedge_mode".to_owned(),
        position_status: "normal".to_owned(),
        side: side.to_owned(),
        size,
        available: size,
        frozen: 0.0,
        entry_price: 30_000.0,
        leverage: 5.0,
        unrealized_pnl: 12.5,
        liquidation_price: Some(20_000.0),
        maintenance_margin_rate: 0.01,
        mark_price: 30_100.0,
        updated_time_ms: 1_700_000_000_000,
    }
}
pub(super) fn bitget_fill_delta(order_id: &str, exec_id: &str) -> bitget_ws_user::BitgetFillUpdate {
    bitget_ws_user::BitgetFillUpdate {
        order_id: order_id.to_owned(),
        client_order_id: "cid-1".to_owned(),
        exec_id: exec_id.to_owned(),
        category: "USDT-FUTURES".to_owned(),
        symbol: "BTC".to_owned(),
        side: "buy".to_owned(),
        hold_side: "long".to_owned(),
        trade_side: "open".to_owned(),
        price: 94_993.0,
        size: 0.01,
        value: 949.93,
        realized_pnl: 0.0,
        fee: 0.569958,
        fee_currency: Some("USDT".to_owned()),
        trade_time_ms: 1_736_378_720_623,
        updated_time_ms: 1_736_378_720_623,
        is_rpi: Some(false),
    }
}
pub(super) fn bybit_position_delta(
    symbol: &str,
    category: &str,
    size: f64,
    side: &str,
) -> bybit_ws_user::BybitPositionDelta {
    bybit_ws_user::BybitPositionDelta {
        symbol: symbol.to_owned(),
        category: category.to_owned(),
        side: side.to_owned(),
        size,
        entry_price: 50_100.0,
        mark_price: 50_000.0,
        unrealized_pnl: 2.5,
        leverage: 3.0,
        liquidation_price: Some(45_000.0),
        updated_time_ms: 1_697_682_317_038,
    }
}
pub(super) fn okx_position_delta(
    inst_id: &str,
    quantity: f64,
    side: &str,
) -> okx_ws_user::OkxPositionDelta {
    okx_ws_user::OkxPositionDelta {
        symbol: exchange::strip_common_suffixes(inst_id),
        inst_id: inst_id.to_owned(),
        inst_type: "SWAP".to_owned(),
        side: side.to_owned(),
        quantity,
        entry_price: 100.0,
        mark_price: 101.0,
        unrealized_pnl: 1.5,
        leverage: 10.0,
        liquidation_price: Some(50.0),
        margin: 10.0,
        initial_margin: 10.0,
        maintenance_margin_ratio: 0.005,
        updated_time_ms: 1_700_000_000_000,
    }
}
pub(super) fn order_info(exchange: &str, status: OrderStatus) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: "o1".to_owned(),
        symbol: "BTC".to_owned(),
        exchange: exchange.to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status,
        quantity: 1.0,
        price: 100.0,
        filled_quantity: 1.0,
        filled_price: 100.0,
        fees: 0.0,
        created_at: Utc::now(),
    }
}
