use leptos::prelude::*;
use shared_types::{CloseLegStatus, CloseRun, CloseRunStatus};

use super::{close_runs_panel::close_run_status_label, format::money, SectionData};
use crate::panels::modules::timestamp::local_date_hm;

pub(in crate::panels::modules::positions) fn close_history_panel(
    source: Memo<SectionData<Vec<CloseRun>>>,
) -> impl IntoView {
    let rows = Memo::new(move |_| {
        let mut rows = source.get().value;
        rows.sort_by_key(|run| std::cmp::Reverse(run.updated_at_ms));
        rows
    });
    view! {
        <section class="position-close-history" aria-label="平仓记录">
            <header>
                <h3>"平仓记录"</h3>
                <span>{move || format!("最近 {} 条 · 本地时间", rows.get().len())}</span>
            </header>
            {move || source.get().status.stale_note("刷新失败，显示上次记录").map(|note| view! {
                <p class="position-history-warning" role="status">{note}</p>
            })}
            {move || rows.get().is_empty().then(|| view! {
                <p class="positions-activity-empty" role="status">{source.get().status.empty_text(
                    "暂无平仓记录", "正在读取平仓记录", "平仓记录读取失败",
                )}</p>
            })}
            <div class="position-history-head" aria-hidden="true">
                <span>"状态 / 时间"</span><span>"仓位"</span><span>"成交确认"</span><span>"剩余裸露"</span>
            </div>
            <For
                each=move || { rows.get().into_iter().map(|run| run.id).collect::<Vec<_>>() }
                key=|id| id.clone()
                children=move |id| {
                    let record = Memo::new(move |_| rows.with(|rows| rows.iter().find(|run| run.id == id).cloned()));
                    view! {
                        <details class="position-history-record">
                            <summary>
                                {move || record.get().map(|run| {
                                    let label = if run.status == CloseRunStatus::Submitted { "等待成交" } else { close_run_status_label(run.status) };
                                    let tone = history_tone(&run);
                                    let markets = run.legs.iter().map(|leg| format!("{} · {}", leg.symbol, leg.venue.to_uppercase())).collect::<Vec<_>>().join(" / ");
                                    view! {
                                        <span><strong class=tone>{label}</strong><small>{local_date_hm(run.updated_at_ms).unwrap_or_else(|| "时间未知".into())}</small></span>
                                        <span class="position-history-market"><strong>{if markets.is_empty() { "仓位资料待确认".into() } else { markets }}</strong><small>{run.message.clone()}</small></span>
                                        <span class="num"><strong>{confirmed_legs_label(&run)}</strong><small>"已确认成交 / 计划腿数"</small></span>
                                        <span class="num"><strong>{money(run.naked_exposure_usd)}</strong><small>"详情"</small></span>
                                    }
                                })}
                            </summary>
                            <div class="position-history-detail">
                                {move || record.get().map(|run| {
                                    let problem = run.finality_problem.as_ref().or(run.problem.as_ref());
                                    let evidence = problem.map(|problem| format!("{} · {}", problem.code, problem.message));
                                    view! {
                                        <dl>
                                            <div><dt>"记录 ID"</dt><dd>{run.id}</dd></div>
                                            <div><dt>"开始时间"</dt><dd>{local_date_hm(run.started_at_ms).unwrap_or_else(|| "未知".into())}</dd></div>
                                            <div><dt>"请求 ID"</dt><dd>{run.request_id.unwrap_or_else(|| "未提供".into())}</dd></div>
                                            <div><dt>"快照"</dt><dd>{run.snapshot_version}</dd></div>
                                        </dl>
                                        {evidence.map(|message| view! { <p class="position-history-warning">{message}</p> })}
                                        <ul>{run.legs.into_iter().map(|leg| view! {
                                            <li><span>{format!("{} · {}", leg.venue.to_uppercase(), leg.symbol)}</span><strong>{leg_status_label(leg.status)}</strong></li>
                                        }).collect_view()}</ul>
                                    }
                                })}
                            </div>
                        </details>
                    }
                }
            />
        </section>
    }
}

fn confirmed_legs_label(run: &CloseRun) -> String {
    let filled = run
        .legs
        .iter()
        .filter(|leg| leg.status == CloseLegStatus::Filled)
        .count();
    format!("{filled}/{}", run.expected_leg_count)
}

fn history_tone(run: &CloseRun) -> &'static str {
    if run.finality_problem.is_some() {
        return "warning";
    }
    match run.status {
        CloseRunStatus::Succeeded | CloseRunStatus::Compensated => "positive",
        CloseRunStatus::Failed
        | CloseRunStatus::UnwindRequired
        | CloseRunStatus::CompensationFailed => "negative",
        CloseRunStatus::ManuallyResolved => "muted",
        _ => "warning",
    }
}

fn leg_status_label(status: CloseLegStatus) -> &'static str {
    match status {
        CloseLegStatus::Submitted => "已提交，等待受理",
        CloseLegStatus::Accepted => "已受理，等待成交",
        CloseLegStatus::PartiallyFilled => "部分成交",
        CloseLegStatus::Filled => "已确认成交",
        CloseLegStatus::CancelRequested => "撤单中",
        CloseLegStatus::Cancelled => "已撤单",
        CloseLegStatus::Rejected => "已拒绝",
        CloseLegStatus::Failed => "失败",
        CloseLegStatus::Skipped => "未提交",
    }
}
