use crate::state::load_state::LoadState;
use shared_types::{ReviewEnvelope, VenueQualityEnvelope};

use super::derive::{review_state_presentation, venue_quality_state_presentation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReviewTaskSummary {
    pub(super) value: String,
    pub(super) badge: String,
    pub(super) tone: &'static str,
}

pub(super) fn review_task_summary<T>(
    state: &LoadState<ReviewEnvelope<T>>,
    unit: &'static str,
) -> ReviewTaskSummary {
    let presentation = review_state_presentation(state);
    ReviewTaskSummary {
        value: review_task_value(state, unit),
        badge: compact_badge(presentation.tone, presentation.badge),
        tone: presentation.tone,
    }
}

pub(super) fn venue_quality_task_summary(
    state: &LoadState<VenueQualityEnvelope>,
) -> ReviewTaskSummary {
    let presentation = venue_quality_state_presentation(state);
    let value = state
        .value()
        .map(|envelope| format!("{}/{} 已采样", envelope.sampled_count, envelope.row_count))
        .unwrap_or_else(|| unavailable_value(state));
    ReviewTaskSummary {
        value,
        badge: compact_badge(presentation.tone, presentation.badge),
        tone: presentation.tone,
    }
}

fn review_task_value<T>(state: &LoadState<ReviewEnvelope<T>>, unit: &'static str) -> String {
    state
        .value()
        .map(|envelope| format!("{} {unit}", envelope.page.total_rows))
        .unwrap_or_else(|| unavailable_value(state))
}

fn unavailable_value<T>(state: &LoadState<T>) -> String {
    match state {
        LoadState::Loading => "读取中".to_owned(),
        LoadState::Error(_) => "不可用".to_owned(),
        LoadState::Ready(_) | LoadState::Stale { .. } => "未知".to_owned(),
    }
}

fn compact_badge(tone: &'static str, badge: String) -> String {
    if badge.contains("旧快照") {
        return "旧快照".to_owned();
    }
    let code_like = !badge.is_empty()
        && badge.chars().all(|value| {
            value.is_ascii_uppercase() || value.is_ascii_digit() || "_-".contains(value)
        });
    let too_long = badge.chars().count() > 10;
    match (tone, code_like || too_long) {
        ("is-warning", true) => "需关注".to_owned(),
        ("is-danger", true) => "不可用".to_owned(),
        _ => badge,
    }
}
