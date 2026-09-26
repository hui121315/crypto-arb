//! Compact `HedgeTicket` view persistence and runtime-source arbitration.

use super::connection::ExecutionConnection;
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
            Self::LocalSnapshot => "本机保存的交易计划",
            Self::BackendPreview => "后台检查结果",
            Self::RestRunSnapshot => "从后台读取的交易记录",
            Self::WsRunDelta => "后台实时推送的交易进度",
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct WorkflowViewFeed {
    connection: ExecutionConnection,
    pub(in crate::panels::modules::execution) view: RwSignal<Option<HedgeTicketView>>,
    pub(in crate::panels::modules::execution) provenance: RwSignal<WorkflowViewSource>,
}

impl WorkflowViewFeed {
    pub(super) fn restored() -> Self {
        let connection = expect_context::<ExecutionConnection>();
        Self {
            connection,
            view: RwSignal::new(connection.read(WORKFLOW_VIEW_KEY, |raw| serde_json::from_str(raw).ok())),
            provenance: RwSignal::new(LOCAL_SNAPSHOT_SOURCE),
        }
    }

    pub(super) fn clear(self) {
        self.connection.clear(WORKFLOW_VIEW_KEY);
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
    store_view(feed.connection, &view);
    feed.view.set(Some(view));
    feed.provenance.set(PREVIEW_SOURCE);
}

pub(super) fn apply_run_candidate(
    feed: WorkflowViewFeed,
    candidate: HedgeTicketView,
    provenance: WorkflowViewSource,
) {
    let next = merge_candidate(feed.view.get_untracked().as_ref(), candidate);
    store_view(feed.connection, &next);
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

fn store_view(connection: ExecutionConnection, view: &HedgeTicketView) {
    if let Ok(encoded) = serde_json::to_string(view) {
        connection.store(WORKFLOW_VIEW_KEY, &encoded);
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
