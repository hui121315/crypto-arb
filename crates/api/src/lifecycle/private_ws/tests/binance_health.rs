use super::super::binance::binance_order_events_require_durable_ack;
use super::*;

#[test]
fn binance_order_health_waits_for_durable_apply_ack() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = binance_ws_user::parse_user_event(include_str!(
        "../../../../../exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json"
    ))?
    .ok_or("Binance fixture did not produce an event")?;
    let events = crate::trading_service::private_ws_mapper::map_binance_event(parsed);

    assert!(binance_order_events_require_durable_ack(&events));
    assert!(!binance_order_events_require_durable_ack(&[
        crate::trading_service::private_ws_events::PrivateWsEvent::AccountDirty(
            crate::trading_service::private_ws_events::PrivateAccountDirty::new(
                "binance",
                crate::trading_service::private_ws_events::PrivateAccountScope::All,
                "test",
            ),
        ),
    ]));
    Ok(())
}
