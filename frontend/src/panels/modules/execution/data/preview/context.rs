use crate::state::{load_state::LoadState, trading_status::TradingStatusState};
use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionEnvironment, TradingRiskStatus};

use super::ExecutionPreview;

#[derive(Clone, PartialEq)]
struct ContextStatus {
    adapter: String,
    environment: ExecutionEnvironment,
    risk: TradingRiskStatus,
}

#[derive(Clone, PartialEq)]
pub(super) struct PreviewContext {
    revision: u64,
    status: Option<ContextStatus>,
}

pub(super) fn use_preview_context() -> Memo<PreviewContext> {
    let status = expect_context::<TradingStatusState>().state;
    Memo::new(move |previous: Option<&PreviewContext>| {
        let status = match status.get() {
            LoadState::Ready(status) if !status.adapter.trim().is_empty() => Some(ContextStatus {
                adapter: status.adapter,
                environment: status.environment,
                risk: status.risk,
            }),
            _ => None,
        };
        // Returning to the same mode must not revive an earlier request or approval.
        let revision = previous.map_or(0, |previous| {
            previous.revision.wrapping_add(u64::from(previous.status != status))
        });
        PreviewContext { revision, status }
    })
}

impl PreviewContext {
    pub(super) fn problem(&self) -> Option<ApiProblem> {
        let message = match self.status.as_ref() {
            None => "交易模式与风控状态未确认或已过期，等待重新读取；旧交易计划不能提交",
            Some(status) if status.risk.kill_switch_active => "紧急停止已开启，不能构建或提交新订单",
            Some(status) if status.environment == ExecutionEnvironment::Live
                && !status.risk.live_trading_enabled => "实盘写入未启用，不能构建或提交新订单",
            Some(_) => return None,
        };
        Some(ApiProblem::new("EXECUTION_PREVIEW_CONTEXT_UNAVAILABLE", message)
            .with_source("execution.preview.context"))
    }

    pub(super) fn validate(&self, preview: &ExecutionPreview) -> Result<(), ApiProblem> {
        if let Some(problem) = self.problem() {
            return Err(problem);
        }
        if self.status.as_ref().is_some_and(|status| {
            crate::panels::shared::execution_environment_label(status.environment)
                == preview.execution_mode_label
        }) {
            Ok(())
        } else {
            Err(ApiProblem::new("EXECUTION_PREVIEW_CONTEXT_MISMATCH",
                "交易检查返回的模式与当前设置不一致，请重新检查；不会沿用旧交易计划")
                .with_source("execution.preview.context"))
        }
    }
}
