use super::private_ws_events::{
    BinanceOrderTradeDelta, PrivateAccountDirty, PrivateAccountScope,
    PrivateAssetValuationSnapshot, PrivateBalancesPatch, PrivateBalancesSnapshot, PrivateFillDelta,
    PrivateFillWithEvidenceDelta, PrivateFundingDelta, PrivateLiquidationDelta,
    PrivateNonUserCancelDelta, PrivateOpenOrdersSnapshot, PrivateOrderDelta, PrivatePositionsPatch,
    PrivatePositionsSnapshot, PrivateWsApplyOutcome, PrivateWsEvent,
};
use super::TradingService;
use exchange::adapters::{
    binance_ws_user, bitget_uta_ws_user as bitget_ws_user, bybit_ws_user, gate_spot_ws_user,
    gate_ws_user, hyperliquid_ws_user, kucoin_ws_user, okx_ws_user,
};
use shared_types::{
    OrderInfo, OrderSide, OrderTransportMetadata, PositionInfo, VenueAccountSummary,
    VenueAssetValuation, VenueBalanceInfo, VenueFillTransportEvidence, VenueLiquidationMethod,
    VenueLiquidationTransportEvidence,
};

mod account_dirty;
mod apply;
mod binance_okx;
mod bybit_bitget;
mod gate_kucoin;
mod hyperliquid;
mod spot_orders;
#[cfg(test)]
mod tests;
mod values;

use account_dirty::dirty_account;
pub(crate) use apply::apply_events;
pub(crate) use binance_okx::{map_binance_event, map_okx_event};
pub(crate) use bybit_bitget::{map_bitget_event, map_bybit_event};
pub(crate) use gate_kucoin::{map_gate_event, map_kucoin_event};
pub(crate) use hyperliquid::map_hyperliquid_event;
use hyperliquid::*;
pub(crate) use spot_orders::{
    map_gate_spot_orders, map_kraken_spot_execution, map_kucoin_spot_event,
};
pub(crate) use values::funding_payment_delta;
use values::*;
