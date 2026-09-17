use super::*;

impl PrivateWsHealthStore {
    pub(crate) fn record_task_started(&self, venue: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::new(
                OP_PRIVATE_WS_SESSION,
                VenueOperationStatus::Unknown,
                "私有 WS 任务已启动，等待连接样本",
            ),
        );
    }

    pub(crate) fn record_task_aborted(&self, venue: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(OP_PRIVATE_WS_SESSION, "私有 WS 任务已重启或停止"),
        );
    }

    pub(crate) fn record_connected(&self, venue: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_SESSION, "私有 WS 已连接"),
        );
    }

    pub(crate) fn record_disconnected(&self, venue: &str, reason: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::warn(OP_PRIVATE_WS_SESSION, format!("私有 WS 断开：{reason}")),
        );
    }

    pub(crate) fn record_circuit_opened(&self, venue: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(OP_PRIVATE_WS_SESSION, "私有 WS 熔断已打开"),
        );
    }

    pub(crate) fn record_subscribe_attempt(&self, venue: &str, requested: usize) {
        self.subscription_acks.remove(venue);
        self.record(
            venue,
            PrivateWsRuntimeUpdate::new(
                OP_PRIVATE_WS_SUBSCRIBE,
                VenueOperationStatus::Unknown,
                "私有 WS 订阅已发送请求",
            )
            .with_requested(requested),
        );
    }

    pub(crate) fn record_subscribe_sent(&self, venue: &str, requested: usize, sent: usize) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_SUBSCRIBE, "私有 WS 订阅发送完成")
                .with_requested(requested)
                .with_rows(sent),
        );
        if sent > 0 {
            self.record_stream_waiting(venue, OP_PRIVATE_WS_ORDER_STREAM, "订单流");
            self.record_stream_waiting(venue, OP_PRIVATE_WS_ACCOUNT_STREAM, "账户流");
        }
    }

    pub(crate) fn record_subscribe_sent_pending_ack(
        &self,
        venue: &str,
        requested: usize,
        sent: usize,
    ) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::new(
                OP_PRIVATE_WS_SUBSCRIBE,
                VenueOperationStatus::Unknown,
                "私有 WS 订阅已发送，等待服务端确认",
            )
            .with_requested(requested)
            .with_rows(sent),
        );
    }

    pub(crate) fn record_subscribe_ack(
        &self,
        venue: &str,
        requested: usize,
        channel: &str,
        request_id: Option<&str>,
    ) {
        let acknowledged = {
            let mut channels = self.subscription_acks.entry(venue.to_owned()).or_default();
            channels.insert(channel.to_owned());
            channels.len()
        };
        let status = if acknowledged == requested {
            VenueOperationStatus::Ok
        } else {
            VenueOperationStatus::Unknown
        };
        self.record(
            venue,
            PrivateWsRuntimeUpdate::new(
                OP_PRIVATE_WS_SUBSCRIBE,
                status,
                format!("私有 WS 服务端已确认 {acknowledged}/{requested} 条订阅"),
            )
            .with_requested(requested)
            .with_rows(acknowledged)
            .with_request_id(request_id),
        );
        if acknowledged == requested {
            self.record_stream_waiting(venue, OP_PRIVATE_WS_ORDER_STREAM, "订单流");
            self.record_stream_waiting(venue, OP_PRIVATE_WS_ACCOUNT_STREAM, "账户流");
        }
    }

    pub(crate) fn record_auth_failed(&self, venue: &str, error: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_SESSION,
                format!("私有 WS 鉴权失败：{error}"),
            )
            .with_retry_after_ms(PRIVATE_WS_RETRY_AFTER_MS)
            .with_error(error),
        );
    }

    pub(crate) fn record_subscribe_rejected(
        &self,
        venue: &str,
        requested: usize,
        channel: &str,
        error: &str,
        request_id: Option<&str>,
    ) {
        let acknowledged = self
            .subscription_acks
            .get(venue)
            .map_or(0, |channels| channels.len());
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_SUBSCRIBE,
                format!("私有 WS 订阅被拒绝：channel={channel}; {error}"),
            )
            .with_requested(requested)
            .with_rows(acknowledged)
            .with_retry_after_ms(PRIVATE_WS_RETRY_AFTER_MS)
            .with_error(error)
            .with_request_id(request_id),
        );
    }

    pub(crate) fn record_subscribe_failed(
        &self,
        venue: &str,
        requested: usize,
        sent: usize,
        error: &str,
    ) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_SUBSCRIBE,
                format!("私有 WS 订阅发送失败：{error}"),
            )
            .with_requested(requested)
            .with_rows(sent)
            .with_error(error),
        );
    }

    pub(crate) fn record_subscribe_build_failed(
        &self,
        venue: &str,
        requested: usize,
        built: usize,
        error: &str,
    ) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_SUBSCRIBE,
                format!("私有 WS 订阅 payload 构建失败：{error}"),
            )
            .with_requested(requested)
            .with_rows(built)
            .with_error(error),
        );
    }

    pub(crate) fn record_text_received(&self, venue: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_SESSION, "私有 WS 收到实时消息"),
        );
    }

    pub(crate) fn record_parse_error(&self, venue: &str, error: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_SESSION,
                format!("私有 WS 消息解析失败：{error}"),
            ),
        );
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_ACCOUNT_STREAM,
                "私有 WS 消息解析失败，账户缓存等待新的权威事件",
            )
            .with_error(error),
        );
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_ORDER_STREAM,
                "私有 WS 消息解析失败，订单缓存等待新的权威事件",
            )
            .with_error(error),
        );
    }
}
