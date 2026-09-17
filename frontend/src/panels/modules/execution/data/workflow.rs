//! Compact `HedgeTicket` view persistence and runtime-source arbitration.

use crate::state::module_runtime::{clear_choice, store_choice, stored_choice};
use leptos::prelude::*;
use shared_types::{HedgeTicketLegView, HedgeTicketView, ResourceStatus};

const WORKFLOW_VIEW_KEY: &str = "crossline.execution.hedgeTicketView";
pub(super) const LOCAL_SNAPSHOT_SOURCE: WorkflowViewSource = WorkflowViewSource::LocalSnapshot;
pub(super) const PREVIEW_SOURCE: WorkflowViewSource = WorkflowViewSource::BackendPreview;
pub(super) const REST_RUN_SOURCE: WorkflowViewSource = WorkflowViewSource::RestRunSnapshot;
pub(super) const WS_RUN_SOURCE: WorkflowViewSource = WorkflowViewSource::WsRunDelta;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::execution) enum WorkflowViewSource {
    LocalSnapshot,
    BackendPreview,
    RestRunSnapshot,
    WsRunDelta,
}

impl WorkflowViewSource {
    pub(in crate::panels::modules::execution) const fn label(self) -> &'static str {
        match self {
            Self::LocalSnapshot => "本地票据快照",
            Self::BackendPreview => "后端预览",
            Self::RestRunSnapshot => "REST 运行单快照",
            Self::WsRunDelta => "WS 运行单增量",
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct WorkflowViewFeed {
    pub(in crate::panels::modules::execution) view: RwSignal<Option<HedgeTicketView>>,
    pub(in crate::panels::modules::execution) provenance: RwSignal<WorkflowViewSource>,
}

impl WorkflowViewFeed {
    pub(super) fn restored() -> Self {
        Self {
            view: RwSignal::new(restored_view()),
            provenance: RwSignal::new(LOCAL_SNAPSHOT_SOURCE),
        }
    }

    pub(super) fn clear(self) {
        clear_choice(WORKFLOW_VIEW_KEY);
        self.view.set(None);
        self.provenance.set(LOCAL_SNAPSHOT_SOURCE);
    }

    pub(super) fn matches_opportunity(self, opportunity_id: &str) -> bool {
        self.view
            .get_untracked()
            .as_ref()
            .and_then(HedgeTicketView::opportunity)
            .is_some_and(|stored| stored == opportunity_id)
    }
}

pub(super) fn apply_preview(feed: WorkflowViewFeed, view: HedgeTicketView) {
    if view.ticket().is_none() || view.opportunity().is_none() {
        return;
    }
    store_view(&view);
    feed.view.set(Some(view));
    feed.provenance.set(PREVIEW_SOURCE);
}

pub(super) fn apply_run_candidate(
    feed: WorkflowViewFeed,
    candidate: HedgeTicketView,
    provenance: WorkflowViewSource,
) {
    let next = merge_candidate(feed.view.get_untracked().as_ref(), candidate);
    store_view(&next);
    feed.view.set(Some(next));
    feed.provenance.set(provenance);
}

fn merge_candidate(
    current: Option<&HedgeTicketView>,
    candidate: HedgeTicketView,
) -> HedgeTicketView {
    let same_context = current.is_some_and(|current| {
        current.ticket() == candidate.ticket() && current.opportunity() == candidate.opportunity()
    });
    if same_context && !has_runtime_health(&candidate) {
        let mut merged = current.cloned().unwrap_or_default();
        merged.execution_run = candidate.execution_run;
        return merged;
    }
    candidate
}

fn has_runtime_health(view: &HedgeTicketView) -> bool {
    [view.long_leg.as_ref(), view.short_leg.as_ref()]
        .into_iter()
        .flatten()
        .any(leg_has_runtime_health)
}

fn leg_has_runtime_health(leg: &HedgeTicketLegView) -> bool {
    [&leg.market, &leg.fee, &leg.balance, &leg.capability]
        .into_iter()
        .any(|health| {
            health.source.is_some()
                || health.evidence_id.is_some()
                || health.problem.is_some()
                || health.status != ResourceStatus::Warming
        })
}

fn restored_view() -> Option<HedgeTicketView> {
    stored_choice(WORKFLOW_VIEW_KEY, |raw| serde_json::from_str(raw).ok())
}

fn store_view(view: &HedgeTicketView) {
    if let Ok(encoded) = serde_json::to_string(view) {
        store_choice(WORKFLOW_VIEW_KEY, &encoded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionRunKey, ExecutionRunPhase, ExecutionRunView, HedgeLegRole};

    #[test]
    fn legacy_run_candidate_keeps_rich_ticket_health() {
        let current = rich_view();
        let candidate = HedgeTicketView {
            ticket_id: Some("ticket-1".into()),
            opportunity_id: Some("opp-1".into()),
            execution_run: Some(ExecutionRunView {
                key: ExecutionRunKey {
                    ticket_id: Some("ticket-1".into()),
                    run_id: Some("run-1".into()),
                    order_id: None,
                },
                phase: ExecutionRunPhase::Working,
            }),
            ..HedgeTicketView::default()
        };

        let merged = merge_candidate(Some(&current), candidate);

        assert_eq!(
            merged.long_leg.as_ref().map(|leg| leg.market.status),
            Some(ResourceStatus::Ready)
        );
        assert_eq!(merged.stable_key().as_deref(), Some("run-1"));
    }

    fn rich_view() -> HedgeTicketView {
        HedgeTicketView {
            ticket_id: Some("ticket-1".into()),
            opportunity_id: Some("opp-1".into()),
            long_leg: Some(HedgeTicketLegView {
                role: HedgeLegRole::Long,
                market: shared_types::WorkflowEvidenceHealth {
                    status: ResourceStatus::Ready,
                    source: Some("ws_push".into()),
                    ..shared_types::WorkflowEvidenceHealth::default()
                },
                ..HedgeTicketLegView::default()
            }),
            ..HedgeTicketView::default()
        }
    }
}
