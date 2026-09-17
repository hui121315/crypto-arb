//! 主产品执行 workflow 的运行视图标识（PR-BO）。
//!
//! `ExecutionRunView` 把一次套利执行抽象成可在 UI 上稳定跟踪的对象，以
//! `ticket_id/run_id/order_id` 为主键来源。核心 fail-closed 契约：处于实盘
//! 进行中阶段（submitting/working/closing）的 run 只有在拥有可恢复主键
//! （稳定 `run_id` + 关联 `ticket_id`）时才算可安全跟踪——否则刷新、返回列表
//! 或 WS 重连后无法可靠地把页面状态对回同一条 run，必须按不可跟踪处理而不是
//! 假装恢复成功。仅有交易所 `order_id` 不足以恢复整条 workflow。

use crate::{
    hedge::{ExecutionRun, ExecutionRunState, HedgeLegRole},
    problem::ApiProblem,
    resource::ResourceStatus,
    strategy::StrategyKind,
};
use serde::{Deserialize, Serialize};

#[path = "workflow/projection.rs"]
mod projection;

/// 一次执行 run 在 UI 上的稳定主键来源。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunKey {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl ExecutionRunKey {
    /// 归一化后的 ticket id（去空白、空串视为无）。
    pub fn ticket(&self) -> Option<&str> {
        non_empty(&self.ticket_id)
    }

    /// 归一化后的 run id。
    pub fn run(&self) -> Option<&str> {
        non_empty(&self.run_id)
    }

    /// 归一化后的交易所 order id。
    pub fn order(&self) -> Option<&str> {
        non_empty(&self.order_id)
    }

    /// 是否可被引用（任一非空标识）。
    pub fn is_addressable(&self) -> bool {
        self.ticket().is_some() || self.run().is_some() || self.order().is_some()
    }

    /// fail-closed：能否在刷新/返回列表/WS 重连后稳定恢复同一个 run。
    /// 需要稳定 `run_id` 且关联 `ticket_id`；仅有交易所 `order_id` 不足。
    pub fn is_resumable(&self) -> bool {
        self.run().is_some() && self.ticket().is_some()
    }

    /// 用于 UI diff/选中态的稳定键；不可恢复时返回 `None`（fail-closed）。
    pub fn stable_key(&self) -> Option<String> {
        if self.is_resumable() {
            self.run().map(str::to_owned)
        } else {
            None
        }
    }
}

/// 执行 run 的 workflow 阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRunPhase {
    Preview,
    Confirming,
    Submitting,
    Working,
    Closing,
    Settled,
    Failed,
}

impl ExecutionRunPhase {
    /// 是否终态。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Failed)
    }

    /// 是否存在进行中的实盘订单（需要可恢复主键才能安全跟踪/操作）。
    pub fn is_live_inflight(self) -> bool {
        matches!(self, Self::Submitting | Self::Working | Self::Closing)
    }
}

/// 一次执行 run 的 UI 运行视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRunView {
    pub key: ExecutionRunKey,
    pub phase: ExecutionRunPhase,
}

impl ExecutionRunView {
    /// fail-closed：实盘进行中的 run 必须拥有可恢复主键才算可安全跟踪；
    /// 非进行中阶段只要可被引用即可。
    pub fn is_trackable(&self) -> bool {
        if self.phase.is_live_inflight() {
            self.key.is_resumable()
        } else {
            self.key.is_addressable()
        }
    }

    /// 重连后是否需要主动 reconcile（进行中且主键可恢复）。
    pub fn needs_reconcile_on_reconnect(&self) -> bool {
        self.phase.is_live_inflight() && self.key.is_resumable()
    }
}

/// 机会工作流所属的主产品模块。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityWorkflowSurface {
    Opportunities,
    Futures,
}

/// 机会列表可恢复状态。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityWorkflowListState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<StrategyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_apr_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_depth_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_index: Option<usize>,
}

impl OpportunityWorkflowListState {
    /// 用户是否处在搜索/筛选后的列表上下文里。
    pub fn has_filter_context(&self) -> bool {
        non_empty(&self.query).is_some()
            || self.strategy.is_some()
            || self.min_apr_pct.is_some_and(|apr| apr > 0.0)
            || self.min_depth_usd.is_some_and(|depth| depth > 0.0)
    }

    /// 是否有可用于回到列表的快照/分页锚点。
    pub fn has_cache_anchor(&self) -> bool {
        non_empty(&self.snapshot_id).is_some() || non_empty(&self.cursor).is_some()
    }
}

/// 详情面板的恢复状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityWorkflowDetailStatus {
    #[default]
    Empty,
    Loading,
    Ready,
    Stale,
    Error,
}

impl OpportunityWorkflowDetailStatus {
    pub fn is_usable_after_return(self) -> bool {
        matches!(self, Self::Ready | Self::Stale)
    }
}

/// 详情面板可恢复状态。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityWorkflowDetailState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opportunity_id: Option<String>,
    #[serde(default)]
    pub status: OpportunityWorkflowDetailStatus,
}

impl OpportunityWorkflowDetailState {
    pub fn opportunity(&self) -> Option<&str> {
        non_empty(&self.opportunity_id)
    }

    pub fn is_usable_for(&self, selected_opportunity_id: Option<&str>) -> bool {
        let Some(selected) = selected_opportunity_id else {
            return false;
        };
        self.status.is_usable_after_return()
            && self
                .opportunity()
                .is_some_and(|opportunity| opportunity == selected)
    }
}

/// 机会选择、列表、详情和执行 run 的主工作流状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityWorkflowState {
    pub surface: OpportunityWorkflowSurface,
    #[serde(default)]
    pub list: OpportunityWorkflowListState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_opportunity_id: Option<String>,
    #[serde(default)]
    pub detail: OpportunityWorkflowDetailState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_run: Option<ExecutionRunView>,
}

impl OpportunityWorkflowState {
    pub fn selected_opportunity(&self) -> Option<&str> {
        non_empty(&self.selected_opportunity_id)
    }

    /// 返回列表后能否恢复同一个机会上下文。
    pub fn can_restore_selection_after_return(&self) -> bool {
        self.selected_opportunity().is_some() && self.list.has_cache_anchor()
    }

    /// 返回后是否需要重新拉详情。
    pub fn needs_detail_fetch_after_return(&self) -> bool {
        let selected = self.selected_opportunity();
        selected.is_some() && !self.detail.is_usable_for(selected)
    }

    /// WS 重连后是否需要按执行 run 主键主动 reconcile。
    pub fn needs_run_reconcile_on_reconnect(&self) -> bool {
        self.execution_run
            .as_ref()
            .is_some_and(ExecutionRunView::needs_reconcile_on_reconnect)
    }
}

/// `HedgeTicket` 在 UI workflow 里的可恢复视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEvidenceHealth {
    pub status: ResourceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for WorkflowEvidenceHealth {
    fn default() -> Self {
        Self {
            status: ResourceStatus::Warming,
            source: None,
            evidence_id: None,
            observed_at_ms: None,
            retry_after_ms: None,
            request_id: None,
            problem: None,
        }
    }
}

/// 一条票据腿的四类执行前证据健康，不复制大型原始证据载荷。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeTicketLegView {
    pub role: HedgeLegRole,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub venue: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub symbol: String,
    #[serde(default)]
    pub market: WorkflowEvidenceHealth,
    #[serde(default)]
    pub fee: WorkflowEvidenceHealth,
    #[serde(default)]
    pub balance: WorkflowEvidenceHealth,
    #[serde(default)]
    pub capability: WorkflowEvidenceHealth,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HedgeTicketView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opportunity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<StrategyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_leg: Option<HedgeTicketLegView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_leg: Option<HedgeTicketLegView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_run: Option<ExecutionRunView>,
}

impl HedgeTicketView {
    pub fn ticket(&self) -> Option<&str> {
        non_empty(&self.ticket_id)
    }

    pub fn opportunity(&self) -> Option<&str> {
        non_empty(&self.opportunity_id)
    }

    /// 预览态需要 ticket + opportunity；若已有实盘 run，则 run 本身也必须可跟踪。
    pub fn can_restore_execution_context(&self) -> bool {
        if self.ticket().is_none() || self.opportunity().is_none() {
            return false;
        }
        match &self.execution_run {
            Some(run) => run.is_trackable(),
            None => true,
        }
    }

    pub fn needs_reconcile_on_reconnect(&self) -> bool {
        self.execution_run
            .as_ref()
            .is_some_and(ExecutionRunView::needs_reconcile_on_reconnect)
    }

    pub fn stable_key(&self) -> Option<String> {
        self.execution_run
            .as_ref()
            .and_then(|run| run.key.stable_key())
            .or_else(|| self.ticket().map(str::to_owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ticket: Option<&str>, run: Option<&str>, order: Option<&str>) -> ExecutionRunKey {
        ExecutionRunKey {
            ticket_id: ticket.map(str::to_owned),
            run_id: run.map(str::to_owned),
            order_id: order.map(str::to_owned),
        }
    }

    #[test]
    fn run_and_ticket_is_resumable_with_stable_key() {
        let k = key(Some("tk-1"), Some("run-1"), Some("ord-1"));
        assert!(k.is_addressable());
        assert!(k.is_resumable());
        assert_eq!(k.stable_key().as_deref(), Some("run-1"));
    }

    #[test]
    fn order_only_is_addressable_but_not_resumable() {
        let k = key(None, None, Some("ord-1"));
        assert!(k.is_addressable());
        assert!(!k.is_resumable());
        assert_eq!(k.stable_key(), None);
    }

    #[test]
    fn run_without_ticket_is_not_resumable() {
        let k = key(None, Some("run-1"), None);
        assert!(!k.is_resumable());
        assert_eq!(k.stable_key(), None);
    }

    #[test]
    fn empty_key_is_not_addressable() {
        let k = key(None, None, None);
        assert!(!k.is_addressable());
        assert!(!k.is_resumable());
    }

    #[test]
    fn whitespace_ids_treated_as_empty() {
        let k = key(Some("  "), Some("\t"), Some(""));
        assert!(!k.is_addressable());
        assert!(!k.is_resumable());
    }

    #[test]
    fn inflight_run_requires_resumable_key_to_be_trackable() {
        let resumable = ExecutionRunView {
            key: key(Some("tk-1"), Some("run-1"), Some("ord-1")),
            phase: ExecutionRunPhase::Working,
        };
        assert!(resumable.is_trackable());
        assert!(resumable.needs_reconcile_on_reconnect());

        let order_only = ExecutionRunView {
            key: key(None, None, Some("ord-1")),
            phase: ExecutionRunPhase::Working,
        };
        assert!(!order_only.is_trackable());
        assert!(!order_only.needs_reconcile_on_reconnect());
    }

    #[test]
    fn preview_is_trackable_when_addressable_without_run() {
        let v = ExecutionRunView {
            key: key(Some("tk-1"), None, None),
            phase: ExecutionRunPhase::Preview,
        };
        assert!(v.is_trackable());
        assert!(!v.needs_reconcile_on_reconnect());
    }

    #[test]
    fn terminal_phase_does_not_need_reconcile() {
        for phase in [ExecutionRunPhase::Settled, ExecutionRunPhase::Failed] {
            assert!(phase.is_terminal());
            let v = ExecutionRunView {
                key: key(Some("tk-1"), Some("run-1"), None),
                phase,
            };
            assert!(!v.needs_reconcile_on_reconnect());
            assert!(v.is_trackable());
        }
    }

    #[test]
    fn opportunity_workflow_restores_selection_only_with_list_anchor() {
        let mut workflow = OpportunityWorkflowState {
            surface: OpportunityWorkflowSurface::Opportunities,
            list: OpportunityWorkflowListState::default(),
            selected_opportunity_id: Some("opp-1".into()),
            detail: OpportunityWorkflowDetailState::default(),
            execution_run: None,
        };

        assert!(!workflow.can_restore_selection_after_return());

        workflow.list.snapshot_id = Some("snap-1".into());
        assert!(workflow.can_restore_selection_after_return());
    }

    #[test]
    fn opportunity_workflow_keeps_filter_context_explicit() {
        let empty = OpportunityWorkflowListState::default();
        assert!(!empty.has_filter_context());

        let query = OpportunityWorkflowListState {
            query: Some(" MU ".into()),
            ..OpportunityWorkflowListState::default()
        };
        assert!(query.has_filter_context());

        let apr = OpportunityWorkflowListState {
            min_apr_pct: Some(0.5),
            ..OpportunityWorkflowListState::default()
        };
        assert!(apr.has_filter_context());
    }

    #[test]
    fn opportunity_workflow_fetches_detail_when_selected_detail_is_missing_or_wrong() {
        let mut workflow = OpportunityWorkflowState {
            surface: OpportunityWorkflowSurface::Futures,
            list: OpportunityWorkflowListState {
                snapshot_id: Some("snap-1".into()),
                ..OpportunityWorkflowListState::default()
            },
            selected_opportunity_id: Some("opp-1".into()),
            detail: OpportunityWorkflowDetailState::default(),
            execution_run: None,
        };

        assert!(workflow.needs_detail_fetch_after_return());

        workflow.detail = OpportunityWorkflowDetailState {
            opportunity_id: Some("opp-2".into()),
            status: OpportunityWorkflowDetailStatus::Ready,
        };
        assert!(workflow.needs_detail_fetch_after_return());

        workflow.detail.opportunity_id = Some("opp-1".into());
        assert!(!workflow.needs_detail_fetch_after_return());

        workflow.detail.status = OpportunityWorkflowDetailStatus::Stale;
        assert!(!workflow.needs_detail_fetch_after_return());

        workflow.detail.status = OpportunityWorkflowDetailStatus::Error;
        assert!(workflow.needs_detail_fetch_after_return());
    }

    #[test]
    fn opportunity_workflow_surfaces_run_reconcile_need() {
        let workflow = OpportunityWorkflowState {
            surface: OpportunityWorkflowSurface::Opportunities,
            list: OpportunityWorkflowListState::default(),
            selected_opportunity_id: Some("opp-1".into()),
            detail: OpportunityWorkflowDetailState::default(),
            execution_run: Some(ExecutionRunView {
                key: key(Some("ticket-1"), Some("run-1"), Some("order-1")),
                phase: ExecutionRunPhase::Working,
            }),
        };

        assert!(workflow.needs_run_reconcile_on_reconnect());
    }

    #[test]
    fn hedge_ticket_view_requires_ticket_and_opportunity() {
        let mut ticket = HedgeTicketView {
            ticket_id: Some("ticket-1".into()),
            opportunity_id: None,
            ..HedgeTicketView::default()
        };

        assert!(!ticket.can_restore_execution_context());

        ticket.opportunity_id = Some("opp-1".into());
        assert!(ticket.can_restore_execution_context());
        assert_eq!(ticket.stable_key().as_deref(), Some("ticket-1"));
    }

    #[test]
    fn hedge_ticket_view_rejects_untrackable_live_run() {
        let ticket = HedgeTicketView {
            ticket_id: Some("ticket-1".into()),
            opportunity_id: Some("opp-1".into()),
            execution_run: Some(ExecutionRunView {
                key: key(None, None, Some("order-1")),
                phase: ExecutionRunPhase::Submitting,
            }),
            ..HedgeTicketView::default()
        };

        assert!(!ticket.can_restore_execution_context());
        assert!(!ticket.needs_reconcile_on_reconnect());
        assert_eq!(ticket.stable_key().as_deref(), Some("ticket-1"));
    }

    #[test]
    fn hedge_ticket_view_uses_resumable_run_key_when_available() {
        let ticket = HedgeTicketView {
            ticket_id: Some("ticket-1".into()),
            opportunity_id: Some("opp-1".into()),
            execution_run: Some(ExecutionRunView {
                key: key(Some("ticket-1"), Some("run-1"), Some("order-1")),
                phase: ExecutionRunPhase::Working,
            }),
            ..HedgeTicketView::default()
        };

        assert!(ticket.can_restore_execution_context());
        assert!(ticket.needs_reconcile_on_reconnect());
        assert_eq!(ticket.stable_key().as_deref(), Some("run-1"));
    }
}
