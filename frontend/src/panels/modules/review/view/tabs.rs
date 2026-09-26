use super::task_summary::ReviewTaskSummary;
use crate::state::module_runtime::normalize_choice;
use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReviewTab {
    Executed,
    Settlements,
    Missed,
    Strategy,
    VenueQuality,
}

impl ReviewTab {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Executed => "执行记录",
            Self::Settlements => "链上 / 股票",
            Self::Missed => "错失机会",
            Self::Strategy => "策略绩效",
            Self::VenueQuality => "场所质量",
        }
    }

    pub(super) const fn unit(self) -> &'static str {
        match self {
            Self::Executed | Self::Settlements | Self::Missed => "条记录",
            Self::Strategy => "组样本",
            Self::VenueQuality => "个样本",
        }
    }

    pub(super) const fn slug(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::Settlements => "settlements",
            Self::Missed => "missed",
            Self::Strategy => "strategy",
            Self::VenueQuality => "venue-quality",
        }
    }

    pub(super) fn from_slug(value: &str) -> Option<Self> {
        Some(match normalize_choice(value) {
            "executed" => Self::Executed,
            "settlements" => Self::Settlements,
            "missed" => Self::Missed,
            "strategy" => Self::Strategy,
            "venue-quality" => Self::VenueQuality,
            _ => return None,
        })
    }

    pub(super) const fn tab_id(self) -> &'static str {
        match self {
            Self::Executed => "review-tab-executed",
            Self::Settlements => "review-tab-settlements",
            Self::Missed => "review-tab-missed",
            Self::Strategy => "review-tab-strategy",
            Self::VenueQuality => "review-tab-venue-quality",
        }
    }

    pub(super) const fn panel_id(self) -> &'static str {
        match self {
            Self::Executed => "review-panel-executed",
            Self::Settlements => "review-panel-settlements",
            Self::Missed => "review-panel-missed",
            Self::Strategy => "review-panel-strategy",
            Self::VenueQuality => "review-panel-venue-quality",
        }
    }

    pub(super) const fn next(self) -> Self {
        match self {
            Self::Executed => Self::Settlements,
            Self::Settlements => Self::Missed,
            Self::Missed => Self::Strategy,
            Self::Strategy => Self::VenueQuality,
            Self::VenueQuality => Self::Executed,
        }
    }

    pub(super) const fn previous(self) -> Self {
        match self {
            Self::Executed => Self::VenueQuality,
            Self::Settlements => Self::Executed,
            Self::Missed => Self::Settlements,
            Self::Strategy => Self::Missed,
            Self::VenueQuality => Self::Strategy,
        }
    }
}

pub(super) fn focus_review_tab(
    tab: ReviewTab,
    executed: NodeRef<leptos::html::Button>,
    settlements: NodeRef<leptos::html::Button>,
    missed: NodeRef<leptos::html::Button>,
    strategy: NodeRef<leptos::html::Button>,
    venue_quality: NodeRef<leptos::html::Button>,
) {
    let target = match tab {
        ReviewTab::Executed => executed,
        ReviewTab::Settlements => settlements,
        ReviewTab::Missed => missed,
        ReviewTab::Strategy => strategy,
        ReviewTab::VenueQuality => venue_quality,
    };
    if let Some(button) = target.get() {
        let _ = button.focus();
    }
}

pub(super) fn review_tab_from_key(current: ReviewTab, key: &str) -> Option<ReviewTab> {
    match key {
        "ArrowRight" => Some(current.next()),
        "ArrowLeft" => Some(current.previous()),
        "Home" => Some(ReviewTab::Executed),
        "End" => Some(ReviewTab::VenueQuality),
        _ => None,
    }
}

pub(super) fn tab_button(
    label: &'static str,
    summary: Memo<ReviewTaskSummary>,
    tab: ReviewTab,
    active: RwSignal<ReviewTab>,
    node_ref: NodeRef<leptos::html::Button>,
) -> impl IntoView {
    view! {
        <button
            node_ref=node_ref
            type="button"
            id=tab.tab_id()
            role="tab"
            aria-controls=tab.panel_id()
            aria-selected=move || (active.get() == tab).to_string()
            tabindex=move || if active.get() == tab { 0 } else { -1 }
            class=move || {
                let tone = summary.get().tone;
                if active.get() == tab {
                    format!("active {tone}")
                } else {
                    tone.to_owned()
                }
            }
            on:click=move |_| active.set(tab)
        >
            <span class="review-task-heading">
                <span>{label}</span>
                <small>{move || summary.get().badge}</small>
            </span>
            <strong class="review-task-value">{move || summary.get().value}</strong>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_tab_slug_round_trips() {
        for tab in [
            ReviewTab::Executed,
            ReviewTab::Missed,
            ReviewTab::Strategy,
            ReviewTab::VenueQuality,
        ] {
            assert_eq!(ReviewTab::from_slug(tab.slug()), Some(tab));
        }
        assert_eq!(ReviewTab::Executed.label(), "执行记录");
    }

    #[test]
    fn parser_accepts_stored_hash_and_rejects_unknown() {
        assert_eq!(ReviewTab::from_slug(" #missed "), Some(ReviewTab::Missed));
        assert_eq!(ReviewTab::from_slug("/strategy"), Some(ReviewTab::Strategy));
        assert_eq!(
            ReviewTab::from_slug("venue-quality"),
            Some(ReviewTab::VenueQuality)
        );
        assert_eq!(ReviewTab::from_slug("unknown"), None);
    }
}
