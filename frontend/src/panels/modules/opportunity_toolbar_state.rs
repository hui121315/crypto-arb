use leptos::prelude::*;
use shared_types::ApiProblem;

use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::modules::opportunity_counts::{duration_label, OpportunityCountMeta};
use crate::state::arbitrage_stream::{stream_channel_status_label, stream_problem_label};
use crate::state::load_state::LoadState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArbitrageFeedStatus {
    pub(crate) label: &'static str,
    pub(crate) tone: &'static str,
}

pub(crate) fn arbitrage_feed_status(
    list_state: &LoadState<()>,
    channel_state: &WsChannelState,
    stream_stale: bool,
    has_problem: bool,
) -> ArbitrageFeedStatus {
    if let LoadState::Stale { problem, .. } = list_state {
        return ArbitrageFeedStatus {
            label: match problem.code.as_str() {
                shared_types::problem::codes::OPPORTUNITY_SNAPSHOT_STALE
                | "OPPORTUNITY_ENVELOPE_STALE" => "候选快照陈旧",
                _ => "候选快照降级",
            },
            tone: "is-degraded",
        };
    }
    if matches!(list_state, LoadState::Error(_)) {
        return ArbitrageFeedStatus {
            label: "候选读取失败",
            tone: "is-degraded",
        };
    }
    if has_problem {
        return ArbitrageFeedStatus {
            label: "候选通道降级",
            tone: "is-degraded",
        };
    }
    if stream_stale {
        return ArbitrageFeedStatus {
            label: if channel_state.status == WsStatus::Connected && channel_state.subscribed {
                "候选流静默"
            } else if channel_state.status == WsStatus::Disconnected {
                "候选流中断"
            } else {
                "候选连接超时"
            },
            tone: "is-degraded",
        };
    }
    if matches!(list_state, LoadState::Loading) || channel_state.status == WsStatus::Connecting {
        return ArbitrageFeedStatus {
            label: "候选连接中",
            tone: "is-warming",
        };
    }
    if channel_state.status == WsStatus::Connected && channel_state.subscribed {
        return ArbitrageFeedStatus {
            label: "候选实时更新",
            tone: "is-live",
        };
    }
    ArbitrageFeedStatus {
        label: "候选源未就绪",
        tone: "is-warming",
    }
}

pub(crate) fn arbitrage_feed_summary(
    feed_status: Memo<ArbitrageFeedStatus>,
    channel_state: RwSignal<WsChannelState>,
    stream_stale: RwSignal<bool>,
    meta_signal: RwSignal<OpportunityCountMeta>,
) -> impl IntoView {
    view! {
        <summary>
            <span
                class=move || format!("futures-feed-indicator {}", feed_status.get().tone)
                aria-hidden="true"
            ></span>
            <strong>{move || feed_status.get().label}</strong>
            <span class="futures-feed-channel">
                {move || compact_channel_label(
                    &channel_state.get(),
                    stream_stale.get(),
                )}
            </span>
            <span class="futures-feed-age">
                {move || compact_snapshot_age_label(&meta_signal.get())}
            </span>
            <span class="futures-feed-detail-label">
                {move || if feed_status.get().tone == "is-degraded" {
                    "查看原因"
                } else {
                    "详情"
                }}
            </span>
        </summary>
    }
}

pub(crate) fn compact_snapshot_age_label(meta: &OpportunityCountMeta) -> String {
    let prefix = if meta.source.ends_with("refresh-inflight") {
        "扫描中 · 候选"
    } else {
        "候选"
    };
    meta.freshness_ms
        .map(|value| format!("{prefix} {} 前", duration_label(value.max(0))))
        .unwrap_or_else(|| {
            if meta.cached_at.is_some() {
                format!("{prefix}已接收")
            } else {
                "等待候选".to_owned()
            }
        })
}

fn compact_channel_label(state: &WsChannelState, stream_stale: bool) -> &'static str {
    if state.last_error.is_some() {
        return "机会流 WS 异常";
    }
    if state.subscribed {
        return if stream_stale {
            "机会流 WS 静默"
        } else {
            "机会流 WS"
        };
    }
    match (state.status, stream_stale) {
        (WsStatus::Connecting, true) => "机会流连接超时",
        (WsStatus::Connecting, false) => "机会流连接中",
        (WsStatus::Connected, true) => "机会流订阅超时",
        (WsStatus::Connected, false) => "机会流待订阅",
        (WsStatus::Disconnected, _) => "机会流未连接",
    }
}

pub(crate) fn stream_channel_message(state: &WsChannelState) -> impl IntoView {
    let label = stream_channel_status_label(state);
    let class_name = if label.is_error {
        "settings-message is-error"
    } else {
        "settings-message"
    };
    view! { <em class=class_name>{label.text}</em> }
}

pub(crate) fn stream_problem_message(
    state: &WsChannelState,
    problem: Option<ApiProblem>,
    list_problem: Option<&ApiProblem>,
) -> Option<impl IntoView> {
    problem_message(
        "实时机会更新失败",
        visible_toolbar_problem(state, problem, list_problem),
    )
}

pub(crate) fn list_state_message(state: &LoadState<()>) -> Option<impl IntoView> {
    let (class_name, message) = list_state_label(state)?;
    Some(view! { <em class=class_name>{message}</em> })
}

pub(crate) fn problem_message(
    label: &'static str,
    problem: Option<ApiProblem>,
) -> Option<impl IntoView> {
    problem.map(|problem| {
        view! {
            <em class="settings-message is-error">
                {format!("{label} · {}", stream_problem_label(&problem))}
            </em>
        }
    })
}

fn list_state_label(state: &LoadState<()>) -> Option<(&'static str, String)> {
    match state {
        LoadState::Loading => (
            "settings-message",
            "机会快照冷启动 · 等待首个响应".to_owned(),
        )
            .into(),
        LoadState::Stale { problem, .. } => (
            "settings-message is-error",
            format!(
                "{} · {}",
                retained_snapshot_status(problem),
                stream_problem_label(problem)
            ),
        )
            .into(),
        LoadState::Error(problem) => (
            "settings-message is-error",
            format!("机会快照冷启动失败 · {}", stream_problem_label(problem)),
        )
            .into(),
        LoadState::Ready(()) => None,
    }
}

fn retained_snapshot_status(problem: &ApiProblem) -> &'static str {
    match problem.code.as_str() {
        shared_types::problem::codes::OPPORTUNITY_SNAPSHOT_STALE | "OPPORTUNITY_ENVELOPE_STALE" => {
            "机会快照陈旧"
        }
        _ => "机会快照降级",
    }
}

fn visible_toolbar_problem(
    state: &WsChannelState,
    problem: Option<ApiProblem>,
    list_problem: Option<&ApiProblem>,
) -> Option<ApiProblem> {
    let problem = problem?;
    if state.last_error.as_ref() == Some(&problem) || list_problem == Some(&problem) {
        return None;
    }
    Some(problem)
}

pub(crate) fn opportunity_snapshot_usable(
    state: &LoadState<()>,
    meta: &OpportunityCountMeta,
) -> bool {
    use shared_types::OpportunityEnvelopeStatus;
    if !matches!(
        meta.status,
        OpportunityEnvelopeStatus::Fresh | OpportunityEnvelopeStatus::Degraded
    ) {
        return false;
    }
    match state {
        LoadState::Ready(()) => true,
        // A partial venue failure is not a failure of every independently verified row.
        LoadState::Stale { problem, .. } if meta.status == OpportunityEnvelopeStatus::Degraded => {
            meta.error.as_ref() == Some(problem)
                || meta.partial_failures.contains(problem)
                || (problem.code == shared_types::problem::codes::OPPORTUNITY_MARKET_DATA_DEGRADED
                    && problem.source.as_deref() == Some("opportunity-envelope"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_feed_summary_separates_channel_from_snapshot_age() {
        let meta = OpportunityCountMeta {
            freshness_ms: Some(16_500),
            ..Default::default()
        };
        let mut channel = WsChannelState::new("arbitrage");
        channel.status = WsStatus::Connected;
        channel.subscribed = true;

        assert_eq!(compact_snapshot_age_label(&meta), "候选 16.5s 前");
        assert_eq!(compact_channel_label(&channel, true), "机会流 WS 静默");
    }

    #[test]
    fn compact_feed_summary_names_an_inflight_scan_without_marking_it_stale() {
        let meta = OpportunityCountMeta {
            source: "snapshot-refresh-inflight".into(),
            freshness_ms: Some(25_000),
            ..Default::default()
        };

        assert_eq!(compact_snapshot_age_label(&meta), "扫描中 · 候选 25.0s 前");
    }

    #[test]
    fn toolbar_problem_hides_problem_already_rendered_by_channel_or_list_state() {
        let problem = ApiProblem::new("WS_PAYLOAD_DECODE", "ws failed")
            .with_source("frontend-ws")
            .with_request_id(Some("req-toolbar".to_owned()));
        let mut state = WsChannelState::new("arbitrage");
        state.last_error = Some(problem.clone());

        assert!(visible_toolbar_problem(&state, Some(problem.clone()), None).is_none());
        assert!(visible_toolbar_problem(
            &WsChannelState::new("arbitrage"),
            Some(problem.clone()),
            Some(&problem),
        )
        .is_none());
    }

    #[test]
    fn list_state_labels_cold_and_stale_with_problem_context() {
        assert!(list_state_message(&LoadState::Loading).is_some());
        let problem = ApiProblem::new("LIST_RATE_LIMITED", "slow")
            .with_source("opportunity-list")
            .with_request_id(Some("req-list-1".to_owned()))
            .with_retry_after_ms(Some(2_000));
        let state = LoadState::Stale { value: (), problem };

        let label = list_state_label(&state)
            .map(|(_, label)| label)
            .unwrap_or_default();
        assert!(label.contains("机会快照降级"));
        assert!(label.contains("source opportunity-list"));
        assert!(label.contains("request_id req-list-1"));
        assert!(label.contains("retry 2000ms"));

        let stale = LoadState::Stale {
            value: (),
            problem: ApiProblem::new(
                shared_types::problem::codes::OPPORTUNITY_SNAPSHOT_STALE,
                "old snapshot",
            ),
        };
        let stale_label = list_state_label(&stale)
            .map(|(_, label)| label)
            .unwrap_or_default();
        assert!(stale_label.contains("机会快照陈旧"));
    }
}
