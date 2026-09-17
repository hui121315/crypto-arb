pub(super) const ARBITRAGE_STREAM_SERIALIZE_FAILED: &str = "ARBITRAGE_STREAM_SERIALIZE_FAILED";

pub(super) fn publish_snapshot(
    hub: &realtime::WsHub,
    metrics: &crate::metrics::Metrics,
    payload: &shared_types::OpportunityStreamEvent,
) -> Result<(), String> {
    // 浏览器关闭、无任何 WS 连接时不构建/序列化 payload——REST 快照走
    // cache_arbitrage_report，纯推送物料在零订阅者时是持续的无谓 CPU/分配。
    if hub.subscriber_count(realtime::channels::ARBITRAGE) == 0 {
        return Ok(());
    }
    let (message, payload_bytes) = snapshot_payload_value(payload)?;
    record_stream_payload_metrics(metrics, payload, payload_bytes);
    hub.publish_throttled(realtime::channels::ARBITRAGE, message);
    Ok(())
}

/// 序列化一次拿到共享文本载体：长度即 payload 字节指标，文本经
/// `WsMessage::JsonText` 在 broadcast 与出站批装配中零重序列化复用。
pub(super) fn snapshot_payload_value(
    payload: &shared_types::OpportunityStreamEvent,
) -> Result<(realtime::WsMessage, usize), String> {
    let text = serde_json::to_string(payload).map_err(|error| {
        format!("{ARBITRAGE_STREAM_SERIALIZE_FAILED}: encode payload failed: {error}")
    })?;
    let payload_bytes = text.len();
    Ok((
        realtime::WsMessage::JsonText(std::sync::Arc::from(text)),
        payload_bytes,
    ))
}

pub(super) fn record_stream_payload_metrics(
    metrics: &crate::metrics::Metrics,
    payload: &shared_types::OpportunityStreamEvent,
    payload_bytes: usize,
) {
    metrics.record_ws_arbitrage_payload(
        payload_bytes,
        payload.top_ids.len(),
        payload.changed_ids.len(),
        payload.changed_rows.len(),
        payload.removed_ids.len(),
    );
}
