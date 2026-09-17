//! Portfolio 快照「数据来源 / 新鲜度 / 重试 / WS 传输态」一体化视图模型。
//!
//! 历史上持仓页只把 `degraded` envelope 落 `Stale`，用户看不到快照究竟来自
//! **实时 WS** 还是 **REST 轮询兜底**、上一帧距今多久、WS 通道是否已订阅、
//! 断线/限流的 `retry_after` 还剩多少。本模块从 [`WsChannelState`] + 轮询兜底
//! 开关派生单一事实源 [`SnapshotTransport`]，fail-closed：断线/错误始终保留
//! `last_error` 的 `code/request_id/retry_after`，绝不把传输层故障显示成"实时"。

use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::modules::opportunity_counts::duration_label;
use crate::panels::modules::positions::data::SnapshotSourceKind;
use leptos::prelude::*;
use shared_types::ApiProblem;

impl SnapshotSourceKind {
    const fn label(self) -> &'static str {
        match self {
            Self::LiveWs => "实时 · WS",
            Self::PollingRest => "轮询 · REST",
            Self::Offline => "离线",
        }
    }

    const fn dot_class(self) -> &'static str {
        match self {
            Self::LiveWs => "snapshot-source-dot is-live",
            Self::PollingRest => "snapshot-source-dot is-polling",
            Self::Offline => "snapshot-source-dot is-offline",
        }
    }
}

/// 快照传输态视图模型：来源 + WS 连接态 + 是否已订阅 + 新鲜度 + 重试 + 末次错误。
#[derive(Debug, Clone, PartialEq)]
pub(in crate::panels::modules::positions) struct SnapshotTransport {
    pub(in crate::panels::modules::positions) source: SnapshotSourceKind,
    pub(in crate::panels::modules::positions) transport: WsStatus,
    pub(in crate::panels::modules::positions) subscribed: bool,
    pub(in crate::panels::modules::positions) freshness_ms: Option<u64>,
    pub(in crate::panels::modules::positions) retry_after_ms: Option<u64>,
    pub(in crate::panels::modules::positions) last_error: Option<ApiProblem>,
    pub(in crate::panels::modules::positions) message_count: u64,
    pub(in crate::panels::modules::positions) problem_count: u64,
    pub(in crate::panels::modules::positions) last_problem_at_ms: Option<u64>,
    pub(in crate::panels::modules::positions) fallback_active: bool,
}

/// 从已接受快照来源、WS 通道态与轮询兜底开关派生快照传输态。
///
/// - 来源只在一份快照真正被接受后更新，轮询启动本身不冒充成功来源。
/// - WS 新鲜度来自真实消息时间；REST 新鲜度来自快照的服务端观测时间。
/// - `retry_after_ms` 优先取通道层，其次取末次错误，保证断线/限流可解释。
pub(in crate::panels::modules::positions) fn describe_snapshot_transport(
    channel: &WsChannelState,
    accepted_source: Option<SnapshotSourceKind>,
    poll_active: bool,
    snapshot_observed_at_ms: Option<u64>,
    now_ms: u64,
) -> SnapshotTransport {
    let source = accepted_source.unwrap_or(SnapshotSourceKind::Offline);
    let observed_at_ms = match source {
        SnapshotSourceKind::LiveWs => channel.last_message_at_ms,
        SnapshotSourceKind::PollingRest | SnapshotSourceKind::Offline => snapshot_observed_at_ms,
    };
    let freshness_ms = observed_at_ms.map(|stamp| now_ms.saturating_sub(stamp));
    let retry_after_ms = channel
        .retry_after_ms
        .or_else(|| channel.last_error.as_ref().and_then(|p| p.retry_after_ms));
    SnapshotTransport {
        source,
        transport: channel.status,
        subscribed: channel.subscribed,
        freshness_ms,
        retry_after_ms,
        last_error: channel.last_error.clone(),
        message_count: channel.message_count,
        problem_count: channel.problem_count,
        last_problem_at_ms: channel.last_problem_at_ms,
        fallback_active: poll_active,
    }
}

impl SnapshotTransport {
    fn transport_label(&self) -> &'static str {
        match self.transport {
            WsStatus::Connected if self.subscribed => "WS 已订阅",
            WsStatus::Connected => "WS 已连接",
            WsStatus::Connecting => "WS 连接中",
            WsStatus::Disconnected => "WS 已断开",
        }
    }

    fn freshness_label(&self) -> String {
        match self.freshness_ms {
            Some(ms) => format!("{} 前", duration_label(ms as i64)),
            None => "尚无数据帧".to_owned(),
        }
    }

    /// 悬浮明细：来源 / 传输态 / 新鲜度 / retry / 末次错误（含 `request_id`）。
    fn title(&self) -> String {
        let mut parts = vec![
            format!("传输来源 {}", self.source.label()),
            self.transport_label().to_owned(),
            format!("新鲜度 {}", self.freshness_label()),
            format!(
                "传输事件 {} / 错误 {}",
                self.message_count, self.problem_count
            ),
        ];
        if let Some(observed_at_ms) = self.last_problem_at_ms {
            parts.push(format!("末次 WS 错误时间 {observed_at_ms}"));
        }
        if let Some(retry) = self.retry_after_ms {
            parts.push(format!("retry {}", duration_label(retry as i64)));
        }
        if self.fallback_active {
            parts.push("REST 兜底读取中".to_owned());
        }
        if let Some(problem) = &self.last_error {
            parts.push(error_detail(problem));
        }
        parts.join(" · ")
    }
}

fn error_detail(problem: &ApiProblem) -> String {
    match (problem.status, problem.request_id.as_deref()) {
        (Some(status), Some(request_id)) => {
            format!(
                "末次错误 HTTP {status} · {} · {request_id}",
                problem.message
            )
        }
        (Some(status), None) => format!("末次错误 HTTP {status} · {}", problem.message),
        (None, Some(request_id)) => format!("末次错误 {} · {request_id}", problem.message),
        (None, None) => format!("末次错误 {}", problem.message),
    }
}

/// 快照传输态徽标：渲染来源点 + 来源/传输文案 + 新鲜度，悬浮显示完整明细。
pub(in crate::panels::modules::positions) fn snapshot_transport_chip(
    transport: Memo<SnapshotTransport>,
) -> impl IntoView {
    view! {
        {move || {
            let view_model = transport.get();
            let title = view_model.title();
            let degraded = matches!(view_model.source, SnapshotSourceKind::Offline)
                || view_model.fallback_active
                || view_model.last_error.is_some();
            let class = if degraded {
                "snapshot-transport is-degraded"
            } else {
                "snapshot-transport"
            };
            view! {
                <div class=class title=title>
                    <span class=view_model.source.dot_class()></span>
                    <strong>"传输 · " {view_model.source.label()}</strong>
                    <span>{view_model.transport_label()}</span>
                    <em>{view_model.freshness_label()}</em>
                </div>
            }
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(
        status: WsStatus,
        subscribed: bool,
        last_message_at_ms: Option<u64>,
    ) -> WsChannelState {
        WsChannelState {
            channel: "portfolio".into(),
            status,
            subscribed,
            last_message_at_ms,
            last_error: None,
            retry_after_ms: None,
            message_count: 0,
            problem_count: 0,
            last_problem_at_ms: None,
        }
    }

    #[test]
    fn live_ws_source_when_connected_without_poll_fallback() {
        let mut channel = channel(WsStatus::Connected, true, Some(9_500));
        channel.message_count = 4;
        let transport = describe_snapshot_transport(
            &channel,
            Some(SnapshotSourceKind::LiveWs),
            false,
            None,
            10_000,
        );

        assert_eq!(transport.source, SnapshotSourceKind::LiveWs);
        assert_eq!(transport.freshness_ms, Some(500));
        assert_eq!(transport.transport_label(), "WS 已订阅");
        assert!(transport.last_error.is_none());
        assert!(transport.title().contains("传输来源 实时 · WS"));
        assert!(transport.title().contains("传输事件 4 / 错误 0"));
    }

    #[test]
    fn poll_fallback_marks_rest_source_even_if_ws_connected() {
        let transport = describe_snapshot_transport(
            &channel(WsStatus::Connected, true, Some(1_000)),
            Some(SnapshotSourceKind::PollingRest),
            true,
            Some(4_500),
            5_000,
        );

        assert_eq!(transport.source, SnapshotSourceKind::PollingRest);
        assert_eq!(transport.freshness_ms, Some(500));
        assert!(transport.fallback_active);
        assert!(transport.title().contains("REST 兜底读取中"));
    }

    #[test]
    fn disconnected_without_poll_is_offline() {
        let transport = describe_snapshot_transport(
            &channel(WsStatus::Disconnected, false, None),
            None,
            false,
            None,
            5_000,
        );

        assert_eq!(transport.source, SnapshotSourceKind::Offline);
        assert_eq!(transport.freshness_ms, None);
        assert_eq!(transport.freshness_label(), "尚无数据帧");
    }

    #[test]
    fn retry_after_falls_back_to_last_error() {
        let mut state = channel(WsStatus::Disconnected, false, Some(1_000));
        state.last_error = Some(
            ApiProblem::new("RATE_LIMITED", "rate limited")
                .with_status(429)
                .with_request_id(Some("req-9".into()))
                .with_retry_after_ms(Some(3_000)),
        );
        state.problem_count = 1;
        state.last_problem_at_ms = Some(3_500);

        let transport = describe_snapshot_transport(
            &state,
            Some(SnapshotSourceKind::PollingRest),
            true,
            Some(3_750),
            4_000,
        );

        assert_eq!(transport.retry_after_ms, Some(3_000));
        let title = transport.title();
        assert!(title.contains("retry 3.0s"));
        assert!(title.contains("req-9"));
        assert!(title.contains("HTTP 429"));
        assert!(title.contains("传输事件 0 / 错误 1"));
        assert!(title.contains("末次 WS 错误时间 3500"));
    }
}
