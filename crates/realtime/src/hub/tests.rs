use super::*;
use pretty_assertions::assert_eq;
use std::io;
use tokio::time::{timeout, Duration};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn subscribe_then_publish_delivers() -> TestResult {
    let hub = WsHub::new(16);
    let mut rx = hub.subscribe("arbitrage");
    let n = hub.publish("arbitrage", WsMessage::Text("hello".into()));
    assert_eq!(n, 1, "one subscriber");

    assert_eq!(recv_text(&mut rx).await?, "hello");
    Ok(())
}

#[tokio::test]
async fn publish_without_subscriber_returns_zero() {
    let hub = WsHub::new(16);
    let n = hub.publish("market:BTC", WsMessage::Text("orphan".into()));
    assert_eq!(n, 0);
    assert!(hub.channels().is_empty());
}

#[tokio::test]
async fn activity_subscription_wakes_without_becoming_an_appws_receiver() -> TestResult {
    let hub = WsHub::new(16);
    let mut activity = hub.subscribe_activity("execution");

    assert_eq!(hub.subscriber_count("execution"), 0);
    assert_eq!(
        hub.publish("execution", WsMessage::Text("changed".into())),
        0
    );
    timeout(Duration::from_millis(50), activity.changed()).await??;
    assert_eq!(*activity.borrow(), 1);
    assert!(hub.channels().is_empty());
    Ok(())
}

#[tokio::test]
async fn explicit_activity_pulse_is_payload_free() -> TestResult {
    let hub = WsHub::new(16);
    let mut activity = hub.subscribe_activity("internal:close-runs");

    hub.notify_activity("internal:close-runs");

    timeout(Duration::from_millis(50), activity.changed()).await??;
    assert_eq!(*activity.borrow(), 1);
    assert!(hub.channels().is_empty());
    Ok(())
}

#[tokio::test]
async fn channels_isolated() -> TestResult {
    let hub = WsHub::new(16);
    let mut rx_arb = hub.subscribe("arbitrage");
    let mut rx_wl = hub.subscribe("watchlist");
    hub.publish("arbitrage", WsMessage::Text("arb".into()));
    hub.publish("watchlist", WsMessage::Text("wl".into()));

    assert_eq!(recv_text(&mut rx_arb).await?, "arb");
    assert_eq!(recv_text(&mut rx_wl).await?, "wl");
    Ok(())
}

#[tokio::test]
async fn multiple_subscribers_all_receive() -> TestResult {
    let hub = WsHub::new(16);
    let mut a = hub.subscribe("arbitrage");
    let mut b = hub.subscribe("arbitrage");
    let mut c = hub.subscribe("arbitrage");
    let n = hub.publish("arbitrage", WsMessage::Text("broadcast".into()));
    assert_eq!(n, 3);

    for rx in [&mut a, &mut b, &mut c] {
        assert_eq!(recv_text(rx).await?, "broadcast");
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn throttled_publish_coalesces_latest_message() -> TestResult {
    let hub = WsHub::new(16);
    let mut rx = hub.subscribe("system");

    assert_eq!(
        hub.publish_throttled("system", WsMessage::Text("first".into())),
        1
    );
    assert_eq!(recv_text(&mut rx).await?, "first");

    assert_eq!(
        hub.publish_throttled("system", WsMessage::Text("old".into())),
        1
    );
    assert_eq!(
        hub.publish_throttled("system", WsMessage::Text("new".into())),
        1
    );
    tokio::task::yield_now().await;

    assert!(rx.try_recv().is_err());
    tokio::time::advance(Duration::from_millis(100)).await;
    tokio::task::yield_now().await;

    assert_eq!(recv_text(&mut rx).await?, "new");
    Ok(())
}

#[tokio::test]
async fn channels_listed_sorted() {
    let hub = WsHub::new(16);
    hub.subscribe("c");
    hub.subscribe("a");
    hub.subscribe("b");
    assert_eq!(hub.channels(), vec!["a", "b", "c"]);
}

#[tokio::test]
async fn subscriber_count_tracks() {
    let hub = WsHub::new(16);
    assert_eq!(hub.subscriber_count("arbitrage"), 0);
    let _r1 = hub.subscribe("arbitrage");
    let _r2 = hub.subscribe("arbitrage");
    assert_eq!(hub.subscriber_count("arbitrage"), 2);
    drop(_r1);
    assert_eq!(hub.subscriber_count("arbitrage"), 1);
}

#[test]
fn runtime_snapshot_accumulates_lag_and_keeps_active_zero_lag_channels() {
    let hub = WsHub::new(16);
    let _orders = hub.subscribe("orders");
    let _system = hub.subscribe("system");
    hub.record_lag("orders", 3, 1_000);
    hub.record_lag("orders", 4, 2_000);

    let rows = hub.runtime_snapshots();

    assert_eq!(rows.len(), 2);
    let orders = rows
        .iter()
        .find(|row| row.channel == "orders")
        .expect("orders runtime row");
    assert_eq!(orders.subscribers, 1);
    assert_eq!(orders.lag_events, 2);
    assert_eq!(orders.skipped_messages, 7);
    assert_eq!(orders.last_lag_at_ms, Some(2_000));
    let system = rows
        .iter()
        .find(|row| row.channel == "system")
        .expect("system runtime row");
    assert_eq!(system.lag_events, 0);
    assert_eq!(system.skipped_messages, 0);
    assert_eq!(system.last_lag_at_ms, None);
}

#[tokio::test]
async fn prune_empty_removes_unsubscribed_channels() {
    let hub = WsHub::new(16);
    {
        let _r = hub.subscribe("with_sub");
    }
    let _persistent = hub.subscribe("persistent");
    assert_eq!(hub.channels().len(), 2);
    let removed = hub.prune_empty();
    assert_eq!(removed, 1);
    assert_eq!(hub.channels(), vec!["persistent"]);
}

#[tokio::test]
async fn throttled_publish_without_subscriber_does_not_create_channel() {
    let hub = WsHub::new(16);
    assert_eq!(
        hub.publish_throttled("system", WsMessage::Text("orphan".into())),
        0
    );
    assert!(hub.channels().is_empty());
}

#[tokio::test]
async fn ws_message_json_helper() -> TestResult {
    let message = WsMessage::json(&serde_json::json!({"a": 1, "b": 2}))?;
    assert!(matches!(message, WsMessage::JsonText(_)));
    let value = message
        .payload_json()
        .ok_or_else(|| io::Error::other("unexpected ws message variant"))?;
    assert_eq!(value["a"], 1);
    assert_eq!(value["b"], 2);
    Ok(())
}

async fn recv_text(
    receiver: &mut broadcast::Receiver<WsMessage>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let message = timeout(Duration::from_millis(50), receiver.recv()).await??;
    let WsMessage::Text(text) = message else {
        return Err(io::Error::other("unexpected ws message variant").into());
    };
    Ok(text)
}
