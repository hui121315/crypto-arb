mod account_dirty_matrix;
mod cases_3;
mod cases_4;
mod cases_5;
mod cases_6;
mod cases_binance_okx;
mod cases_gate_account;
mod cases_hyperliquid_clearinghouse;
mod cases_hyperliquid_events;
mod cases_kucoin;
mod cases_okx_account;
mod fixtures_a;
mod fixtures_b;

use super::PrivateWsEvent;

fn assert_single_account_dirty(events: &[PrivateWsEvent]) {
    assert!(matches!(events, [PrivateWsEvent::AccountDirty(_)]));
}
