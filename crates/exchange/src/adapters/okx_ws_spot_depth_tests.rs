use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parse_snapshot_and_contiguous_update() {
    let snapshot = parse_update(
        r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"bids":[["10","2","0","1"]],"asks":[["11","3","0","1"]],"ts":"1000","seqId":10,"prevSeqId":-1}]}"#,
    )
    .unwrap();
    let mut cached = snapshot_book(&snapshot).unwrap();
    assert_eq!(cached.book.symbol, "BTC/USDT");

    let update = parse_update(
        r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[{"bids":[["10","0","0","0"],["9","4","0","1"]],"asks":[],"ts":"1100","seqId":11,"prevSeqId":10}]}"#,
    )
    .unwrap();
    assert_eq!(
        apply_incremental(&mut cached, &update),
        MergeOutcome::Applied
    );
    assert_eq!(cached.book.bids, vec![[9.0, 4.0]]);
}

#[test]
fn sequence_gap_fails_closed() {
    let mut cached = CachedBook {
        book: OrderBookInfo {
            symbol: "BTC/USDT".into(),
            exchange: EXCHANGE.into(),
            bids: vec![[10.0, 2.0]],
            asks: vec![[11.0, 3.0]],
            timestamp: 1,
        },
        observed_at_ms: now_ms(),
        last_seq_id: 10,
    };
    let update = ParsedUpdate {
        action: BookAction::Update,
        symbol: "BTC-USDT".into(),
        bids: vec![[10.0, 3.0]],
        asks: Vec::new(),
        timestamp: 2,
        seq_id: 12,
        prev_seq_id: 11,
    };
    assert_eq!(apply_incremental(&mut cached, &update), MergeOutcome::Gap);
}

#[test]
fn circuit_open_invalidates_spot_depth_cache() {
    let stream = SpotDepthStream {
        manager: Arc::new(WsManager::new(WsConfig {
            url: WS_URL.into(),
            exchange: EXCHANGE.into(),
            heartbeat_interval: Duration::from_secs(20),
            heartbeat: WsHeartbeat::Text("ping".into()),
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            circuit_breaker_threshold: 10,
        })),
        books: Arc::new(DashMap::new()),
        subscriptions: Arc::new(DashMap::new()),
    };
    stream.on_text(r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"bids":[["1","1","0","1"]],"asks":[["2","1","0","1"]],"ts":"1","seqId":1,"prevSeqId":-1}]}"#);
    assert!(!stream.books.is_empty());

    stream.handle_event(WsEvent::CircuitOpened);

    assert!(stream.books.is_empty());
}
