use super::super::plain_venues::hyperliquid_private_subscription_payloads;
use super::super::HYPERLIQUID_PRIVATE_WS_SUBSCRIPTIONS;

#[test]
fn hyperliquid_private_subscribe_messages_include_all_dexs_clearinghouse(
) -> Result<(), Box<dyn std::error::Error>> {
    let user = "0x0000000000000000000000000000000000000000";
    let payloads = hyperliquid_private_subscription_payloads(user)
        .into_iter()
        .map(
            |(_, payload)| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                Ok(serde_json::from_str(&payload?)?)
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    let subscriptions = payloads
        .iter()
        .map(|payload| payload["subscription"]["type"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();

    assert_eq!(payloads.len(), HYPERLIQUID_PRIVATE_WS_SUBSCRIPTIONS);
    assert_eq!(payloads[8]["subscription"]["aggregateByTime"], false);
    assert_eq!(
        subscriptions,
        vec![
            "orderUpdates",
            "openOrders",
            "openOrders",
            "openOrders",
            "openOrders",
            "openOrders",
            "openOrders",
            "userEvents",
            "userFills",
            "userFundings",
            "clearinghouseState",
            "allDexsClearinghouseState",
            "spotState",
        ]
    );
    let open_order_dexes = payloads[1..=6]
        .iter()
        .map(|payload| payload["subscription"]["dex"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        open_order_dexes,
        vec!["", "xyz", "cash", "flx", "km", "vntl"]
    );
    assert_eq!(payloads[11]["subscription"]["user"], user);
    Ok(())
}
