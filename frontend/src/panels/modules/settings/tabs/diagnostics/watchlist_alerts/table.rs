use crate::panels::modules::settings::tabs::problem_message;
use leptos::prelude::*;
use shared_types::ApiProblem;

pub(super) const WATCHLIST_HEADERS: &[&str] = &[
    "ID",
    "标的",
    "多头 venue",
    "空头 venue",
    "阈值",
    "版本/持久化",
    "Prewarm",
    "资源",
    "诊断",
];
pub(super) const ALERT_HEADERS: &[&str] = &[
    "规则",
    "Watchlist",
    "投递",
    "版本/持久化",
    "运行状态",
    "资源",
    "冷却/最近",
    "诊断",
];

pub(super) struct RuntimeCell {
    text: String,
    class: &'static str,
}

impl RuntimeCell {
    pub(super) fn text(value: impl Into<String>) -> Self {
        Self {
            text: value.into(),
            class: "",
        }
    }

    pub(super) fn status(value: impl Into<String>, class: &'static str) -> Self {
        Self {
            text: value.into(),
            class,
        }
    }
}

pub(super) fn runtime_table(
    title: &'static str,
    summary: String,
    problem: Option<ApiProblem>,
    problem_context: &'static str,
    headers: &'static [&'static str],
    rows: Vec<Vec<RuntimeCell>>,
) -> AnyView {
    view! {
        <>
            <div class="settings-summary-line">
                <strong>{title}</strong>
                <span>{summary}</span>
            </div>
            {problem.map(|problem| view! {
                <em class="settings-message is-error">{problem_message(problem_context, &problem)}</em>
            })}
            <div class="table-wrap">
                <table class="clean-table settings-table" data-table-budget="bounded-small">
                    <thead><tr>{headers.iter().map(|header| view! { <th>{*header}</th> }).collect_view()}</tr></thead>
                    <tbody>{rows.into_iter().map(runtime_row).collect_view()}</tbody>
                </table>
            </div>
        </>
    }
    .into_any()
}

fn runtime_row(cells: Vec<RuntimeCell>) -> impl IntoView {
    view! {
        <tr>{cells.into_iter().map(|cell| view! {
            <td><span class=cell.class>{cell.text}</span></td>
        }).collect_view()}</tr>
    }
}
