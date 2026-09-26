use super::*;
use shared_types::AccountStateSnapshot;

#[path = "account_state/rows.rs"]
mod rows;
#[path = "account_state/selection.rs"]
mod selection;
#[cfg(test)]
#[path = "account_state/tests.rs"]
mod tests;

use rows::{
    account_binding_row, account_data_health_row, account_evidence_table,
    account_field_quality_row, account_problem_row, account_state_status_class,
    account_state_status_label, account_summary_row,
};
use selection::selected_account_evidence;

pub(super) fn account_state_evidence_panel(
    state: LoadState<AccountStateSnapshot>,
    venue_id: &str,
) -> AnyView {
    let (snapshot, stale_problem) = match state {
        LoadState::Ready(snapshot) => (snapshot, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取账户字段数据依据失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取账户字段数据依据"</div> }.into_any();
        }
    };
    if venue_id.trim().is_empty() {
        return view! { <div class="empty-cell">"请选择交易所后查看账户字段数据依据"</div> }.into_any();
    }
    let selection = selected_account_evidence(&snapshot, venue_id);
    let source = snapshot.source;
    let observed_at_ms = snapshot.observed_at_ms;
    let status = account_state_status_label(snapshot.status);
    let status_class = account_state_status_class(snapshot.status);
    let summary = format!(
        "{} · 账户事实 {} · 字段 {} · 行健康 {} · 账户绑定 {} · 问题 {} · 观察 {}",
        venue_id,
        selection.summaries.len(),
        selection.field_quality.len(),
        selection.row_health.len(),
        selection.bindings.len(),
        selection.problems.len(),
        observed_at_ms,
    );
    let field_rows = selection
        .field_quality
        .iter()
        .map(account_field_quality_row)
        .collect_view()
        .into_any();
    let summary_rows = selection
        .summaries
        .iter()
        .map(account_summary_row)
        .collect_view()
        .into_any();
    let health_rows = selection
        .row_health
        .iter()
        .map(account_data_health_row)
        .collect_view()
        .into_any();
    let binding_rows = selection
        .bindings
        .iter()
        .map(account_binding_row)
        .collect_view()
        .into_any();
    let problem_rows = selection
        .problems
        .iter()
        .map(account_problem_row)
        .collect_view()
        .into_any();

    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"账户字段数据依据"</strong>
                    <em>{summary}</em>
                    <em>{format!("来源 {source}")}</em>
                </div>
                <span class=status_class>{status}</span>
            </div>
            {stale_problem.map(|problem| view! {
                <div class="empty-cell">{problem_message("账户字段数据依据刷新失败，显示上次快照", &problem)}</div>
            })}
            {account_evidence_table("账户事实", selection.summaries.len(), summary_rows, "当前交易所没有账户级 equity / margin 事实。")}
            {account_evidence_table("字段质量", selection.field_quality.len(), field_rows, "当前交易所没有字段质量告警。")}
            {account_evidence_table("行健康", selection.row_health.len(), health_rows, "当前交易所没有账户行健康记录。")}
            {account_evidence_table("账户绑定", selection.bindings.len(), binding_rows, "当前交易所没有账户范围绑定数据依据。")}
            {account_evidence_table("账户问题", selection.problems.len(), problem_rows, "当前交易所没有账户问题。")}
        </div>
    }
    .into_any()
}
