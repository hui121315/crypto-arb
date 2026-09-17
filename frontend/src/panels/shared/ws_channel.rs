use crate::api::ws::WsChannelState;

pub(in crate::panels) fn ws_channel_activity_label(state: &WsChannelState) -> String {
    let mut parts = vec![
        format!("帧 {}", state.message_count),
        format!("错误 {}", state.problem_count),
    ];
    if let Some(observed_at_ms) = state.last_problem_at_ms {
        parts.push(format!("末次错误时间 {observed_at_ms}"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_label_keeps_monotonic_channel_evidence() {
        let mut state = WsChannelState::new("orders");
        state.message_count = 7;
        state.problem_count = 2;
        state.last_problem_at_ms = Some(42);

        assert_eq!(
            ws_channel_activity_label(&state),
            "帧 7 · 错误 2 · 末次错误时间 42"
        );
    }
}
