use crate::panels::shared::{CheckItem, CheckItemState};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubjectKind,
    ApiProblem, ExecutionGuard, HedgeDepthStatus, HedgeLegRole, HedgePreflightOperation,
    HedgePreflightStatus, HedgePreviewPositionsEvidence, ListStatus, MarketDataHealth,
    MarketDataQuality, OrderPayloadPricePolicy, VenueBalanceInfo, VenueOperationHealth,
    VenueOperationStatus, VenueOrderKind,
};

use super::super::data::{
    ExecutionPreview, PreviewFundingWindowEvidence, PreviewOneCycleCost, PreviewReadiness,
};
use super::super::problem::execution_problem_text;
use crate::panels::modules::market_evidence::market_health_label;

pub(in crate::panels::modules::execution) fn risk_preview(
    preview: Memo<ExecutionPreview>,
    preview_state: RwSignal<LoadState<ExecutionPreview>>,
    expired: Memo<bool>,
) -> impl IntoView {
    view! {
        <section class="execution-section execution-risk-section">
            <div class="execution-section-head">
                <div>
                    <span>"风险预览"</span>
                    <strong>{move || if expired.get() { "交易检查已过期".into() } else { decision_text(&preview.get()) }}</strong>
                </div>
                <em>{move || {
                    let current = preview.get();
                    format!("{} · {}", current.source, current.execution_mode_label)
                }}</em>
            </div>
            {move || preview_load_notice_text(&preview_state.get()).map(|text| view! {
                <div class="risk-empty stale-note">{text}</div>
            })}
            <dl class="execution-risk-summary">
                <div><dt>{move || if expired.get() { "上次测算净收益" } else { "预计净收益" }}</dt><dd>{move || net_edge_text(&preview.get())}</dd></div>
                <div><dt>{move || if expired.get() { "上次测算总成本" } else { "预计总成本" }}</dt><dd>{move || {
                    let current = preview.get();
                    cost_money(&current, current.total_cost_usd(), "待成本")
                }}</dd></div>
                <div><dt>{move || if expired.get() { "上次亏损估计" } else { "最大亏损估计" }}</dt><dd>{move || {
                    let current = preview.get();
                    ready_money(&current, current.max_loss_usd, "待风控")
                }}</dd></div>
                <div><dt>{move || if expired.get() { "上次强平测算" } else { "提交后强平距离" }}</dt><dd>{move || pct_opt(preview.get().liquidation.after_hedge_pct)}</dd></div>
            </dl>
            {move || {
                let message = preview_state.with(|state| state.problem().is_none())
                    .then(|| preview.get().risk.blockers.first().cloned()).flatten();
                message.map(|message| view! {
                    <p class="execution-preflight-blocker" role="status">{message}</p>
                })
            }}
            <details class="execution-evidence-details">
                <summary>
                    <span>"完整交易检查数据依据"</span>
                    <strong>{move || if expired.get() { "上次交易检查 · 已过期".into() } else { evidence_summary(&preview.get()) }}</strong>
                </summary>
                <Show when=move || expired.get()>
                    <p class="execution-preflight-blocker" role="status">"以下为上次交易检查记录，不代表当前可执行；请刷新预览。"</p>
                </Show>
                <RiskChecks preview=preview/>
                <RiskNotes preview=preview/>
            </details>
        </section>
    }
}

fn evidence_summary(preview: &ExecutionPreview) -> String {
    if preview.risk.blockers.is_empty() {
        format!("{} · 无阻断", preview.source)
    } else {
        format!(
            "{} · {} 条阻断",
            preview.source,
            preview.risk.blockers.len()
        )
    }
}

#[component]
fn RiskChecks(preview: Memo<ExecutionPreview>) -> impl IntoView {
    view! {
            <div class="checks-grid execution-checks">
                <CheckItem
                    label="预计净收益"
                    value=move || {
                        let preview = preview.get();
                        net_edge_text(&preview)
                    }
                    state=move || {
                        let preview = preview.get();
                        positive_net_edge_state(&preview)
                    }
                />
                <CheckItem
                    label="预计成本"
                    value=move || {
                        let preview = preview.get();
                        cost_money(&preview, preview.total_cost_usd(), "待成本")
                    }
                    state=move || {
                        let preview = preview.get();
                        cost_edge_state(&preview)
                    }
                />
                <CheckItem
                    label="开仓费"
                    value=move || {
                        let preview = preview.get();
                        cost_money(&preview, preview.open_cost_usd, "待费率")
                    }
                    state=move || {
                        let preview = preview.get();
                        ready_nonnegative_state(&preview, preview.open_cost_usd)
                    }
                />
                <CheckItem
                    label="平仓费"
                    value=move || {
                        let preview = preview.get();
                        cost_money(&preview, preview.close_cost_usd, "待费率")
                    }
                    state=move || {
                        let preview = preview.get();
                        ready_nonnegative_state(&preview, preview.close_cost_usd)
                    }
                />
                <CheckItem
                    label="VWAP 滑点"
                    value=move || {
                        let preview = preview.get();
                        cost_money(&preview, preview.slippage_cost_usd, "待深度")
                    }
                    state=move || {
                        let preview = preview.get();
                        ready_nonnegative_state(&preview, preview.slippage_cost_usd)
                    }
                />
                <CheckItem
                    label="最大亏损"
                    value=move || {
                        let preview = preview.get();
                        ready_money(&preview, preview.max_loss_usd, "待风控")
                    }
                    state=move || {
                        let preview = preview.get();
                        max_loss_state(&preview)
                    }
                />
                <CheckItem
                    label="风险结论"
                    value=move || decision_text(&preview.get())
                    state=move || decision_state(&preview.get())
                />
                <CheckItem
                    label="当前强平"
                    value=move || current_liq_value(&preview.get())
                    state=move || current_liq_state(&preview.get())
                />
                <CheckItem
                    label="提交后强平"
                    value=move || pct_opt(preview.get().liquidation.after_hedge_pct)
                    state=move || liq_distance_state(preview.get().liquidation.after_hedge_pct)
                />
                <For
                    each=move || preview.get().risk.guards
                    key=|guard| guard.key.clone()
                    children=move |guard| {
                        let label = guard.label.clone();
                        let guard_key = StoredValue::new(guard.key);
                        view! {
                            <CheckItem
                                label=label
                                value=move || {
                                    guard_key.with_value(|key| {
                                        current_guard_detail(&preview.get(), key)
                                    })
                                }
                                state=move || {
                                    guard_key.with_value(|key| {
                                        current_guard_state(&preview.get(), key)
                                    })
                                }
                            />
                        }
                    }
                />
            </div>
    }
}

fn current_guard_detail(preview: &ExecutionPreview, key: &str) -> String {
    preview
        .risk
        .guards
        .iter()
        .find(|guard| guard.key == key)
        .map_or_else(|| "等待最新交易检查".into(), guard_detail)
}

fn current_guard_state(preview: &ExecutionPreview, key: &str) -> CheckItemState {
    preview
        .risk
        .guards
        .iter()
        .find(|guard| guard.key == key)
        .map_or(CheckItemState::Missing, guard_state)
}

#[component]
fn RiskNotes(preview: Memo<ExecutionPreview>) -> impl IntoView {
    view! {
            <div class="risk-notes">
                <div>
                    <span>"交易检查来源"</span>
                    <strong>{move || preview.get().source}</strong>
                </div>
                <div>
                    <span>"执行票据"</span>
                    <strong title=move || ticket_venue_availability_detail(&preview.get())>
                        {move || {
                            let current = preview.get();
                            format!(
                                "{} · {}",
                                ticket_text(&current),
                                ticket_venue_availability_summary(&current)
                            )
                        }}
                    </strong>
                </div>
                <div>
                    <span>"预估毛收益"</span>
                    <strong>{move || {
                        let preview = preview.get();
                        ready_money(&preview, preview.estimated_funding_usd, "待交易检查")
                    }}</strong>
                </div>
                <div>
                    <span>"成本拆解"</span>
                    <strong>{move || cost_breakdown_text(&preview.get())}</strong>
                </div>
                <div>
                    <span>"单次净利"</span>
                    <strong title=move || one_cycle_cost_detail(&preview.get())>
                        {move || one_cycle_cost_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"列表收益数据依据"</span>
                    <strong title=move || profit_evidence_detail(&preview.get())>
                        {move || profit_evidence_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"费率数据依据"</span>
                    <strong title=move || fee_evidence_detail(&preview.get())>
                        {move || fee_evidence_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"订单编译"</span>
                    <strong title=move || order_plan_detail(&preview.get())>
                        {move || order_plan_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"多腿 / 空腿名义"</span>
                    <strong>{move || notional_pair(&preview.get())}</strong>
                </div>
                <div>
                    <span>"可执行深度"</span>
                    <strong title=move || depth_detail(&preview.get())>
                        {move || depth_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"已用保证金"</span>
                    <strong title=move || account_preflight_detail(&preview.get())>
                        {move || margin_preflight_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"持仓数据依据"</span>
                    <strong title=move || positions_evidence_detail(&preview.get())>
                        {move || positions_evidence_summary(&preview.get())}
                    </strong>
                </div>
                <div>
                    <span>"交易检查 ID"</span>
                    <strong title=move || full_preview_id(&preview.get())>
                        {move || preview_id(&preview.get())}
                    </strong>
                </div>
                <p>{move || risk_note_text(&preview.get())}</p>
            </div>
    }
}

mod cost;
mod evidence;
mod format;
mod preflight;
mod state;
#[cfg(test)]
mod tests;

use cost::*;
use evidence::*;
use format::*;
use preflight::*;
use state::*;

fn preview_load_notice_text(state: &LoadState<ExecutionPreview>) -> Option<String> {
    match state {
        LoadState::Error(problem) => Some(execution_problem_text("预览失败", problem)),
        LoadState::Stale { problem, .. } if problem.code == "PREVIEW_REFRESHING" => {
            Some(execution_problem_text("预览刷新中", problem))
        }
        LoadState::Stale { problem, .. } => Some(execution_problem_text("预览已失效", problem)),
        LoadState::Loading | LoadState::Ready(_) => None,
    }
}
