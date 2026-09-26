use leptos::prelude::*;
use shared_types::VenueQuality;

use crate::panels::modules::pagination::{page_controls, use_table_runtime};

use super::venue_quality_detail::{venue_quality_detail, QUALITY_DETAIL_ID};
use super::venue_quality_metrics::{
    fill_class, fill_value, jitter_class, jitter_value, latency_class, latency_value,
    operation_class, operation_value, sample_status_class, sample_status_label, slippage_class,
    slippage_value, sorted_by_risk, summary_cards, uptime_class, uptime_value,
};
use super::venue_radar;
use super::VenueQualityChartMeta;
use super::{section_state_row, ReviewSectionRows};

const PAGE_SIZE: usize = 50;
const QUALITY_PAGE_STORAGE_KEY: &str = "crossline.review.venueQuality.page";

pub(in crate::panels::modules::review) fn venue_quality_panel(
    section: Memo<ReviewSectionRows<VenueQuality>>,
    chart_meta: Memo<VenueQualityChartMeta>,
) -> impl IntoView {
    let rows = Memo::new(move |_| section.with(|section| section.rows.clone()));
    let sorted = Memo::new(move |_| sorted_by_risk(rows.get()));
    let dataset_key = Memo::new(move |_| sorted.with(|rows| venue_quality_dataset_key(rows)));
    let table = use_table_runtime(QUALITY_PAGE_STORAGE_KEY, dataset_key, sorted, PAGE_SIZE);
    let cards = Memo::new(move |_| summary_cards(&rows.get()));
    let selected = RwSignal::new(None::<String>);
    let selected_row = Memo::new(move |_| {
        let selected_venue = selected.get()?;
        sorted.with(|rows| rows.iter().find(|row| row.venue == selected_venue).cloned())
    });
    let close_detail = Callback::new(move |()| selected.set(None));
    let has_loaded_context =
        Memo::new(move |_| section.with(ReviewSectionRows::has_loaded_context));

    view! {
        <div class="venue-quality-panel">
            {move || {
                let cards = cards.get();
                (!cards.is_empty()).then(|| view! {
                    <div class="quality-summary-grid">
                        {cards.into_iter().map(|card| view! {
                            <div class=format!("quality-card {}", card.tone.class())>
                                <span>{card.label}</span>
                                <strong>{card.value}</strong>
                                <em>{card.venue}</em>
                            </div>
                        }).collect_view()}
                    </div>
                })
            }}
            <div
                class="review-quality-workbench"
                class:has-selection=move || selected_row.with(Option::is_some)
            >
                <div class="review-quality-primary">
                    <QualityTable rows=table.runtime section=section selected=selected/>
                    {move || (table.total.get() > PAGE_SIZE).then(|| view! {
                        {page_controls(table.total, table.current_page, PAGE_SIZE)}
                    })}
                </div>
                <For each=move || selected_row.get().into_iter() key=|row| row.venue.clone() children=move |initial| {
                    let row = Memo::new(move |_| selected_row.get().filter(|row| row.venue == initial.venue).unwrap_or_else(|| initial.clone()));
                    venue_quality_detail(row, close_detail)
                }/>
            </div>
            <Show when=move || has_loaded_context.get()>
                <details class="review-quality-disclosure">
                    <summary><strong>"场所汇总雷达"</strong><span>"辅助视图 · 不替代表格"</span></summary>
                    {venue_radar(rows, chart_meta)}
                </details>
            </Show>
        </div>
    }
}

fn venue_quality_dataset_key(rows: &[VenueQuality]) -> String {
    let mut parts = Vec::with_capacity(4);
    parts.push(rows.len().to_string());
    for row in rows.iter().take(2) {
        parts.push(venue_quality_row_key(row));
    }
    if let Some(row) = rows.last() {
        parts.push(venue_quality_row_key(row));
    }
    parts.join("|")
}

fn venue_quality_row_key(row: &VenueQuality) -> String {
    format!(
        "{}:{}:{:?}:{}:{}:{:.3}:{:.3}:{:.3}:{}:{:?}",
        row.venue,
        row.source,
        row.sample_status,
        row.avg_rest_latency_ms,
        row.ws_jitter_p99_ms,
        row.fill_rate_pct,
        row.avg_slippage_bps,
        row.uptime_window_pct,
        row.operation_health.len(),
        row.sample_window.latest_operation_observed_at_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_quality_dataset_key_changes_when_row_order_changes() {
        let first = vec![quality("binance", 80), quality("okx", 120)];
        let second = vec![quality("okx", 120), quality("binance", 80)];

        assert_ne!(
            venue_quality_dataset_key(&first),
            venue_quality_dataset_key(&second)
        );
    }

    fn quality(venue: &str, avg_rest_latency_ms: u32) -> VenueQuality {
        VenueQuality {
            venue: venue.to_owned(),
            source: "test".to_owned(),
            sample_status: shared_types::VenueQualitySampleStatus::Ready,
            avg_rest_latency_ms,
            rest_latency_samples: 1,
            ws_jitter_p99_ms: 0,
            ws_jitter_samples: 1,
            fill_rate_pct: 0.0,
            fill_window_samples: 1,
            avg_slippage_bps: 0.0,
            slippage_samples: 0,
            uptime_window_pct: 0.0,
            uptime_window_samples: 1,
            sample_window: shared_types::VenueQualitySampleWindow::default(),
            operation_health: Vec::new(),
            retry_after_ms: None,
            last_problem: None,
        }
    }
}

#[component]
fn QualityTable(
    rows: Memo<crate::panels::modules::pagination::TableRuntime<VenueQuality>>,
    section: Memo<ReviewSectionRows<VenueQuality>>,
    selected: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="table-wrap">
            <table class="clean-table venue-quality-table" data-table-budget="table-runtime">
                <caption class="sr-only">"场所执行质量，按风险优先排序"</caption>
                <colgroup>
                    <col class="quality-col-venue"/>
                    <col class="quality-col-sample"/>
                    <col class="quality-col-rest"/>
                    <col class="quality-col-ws"/>
                    <col class="quality-col-fill"/>
                    <col class="quality-col-slip"/>
                    <col class="quality-col-uptime"/>
                    <col class="quality-col-evidence"/>
                </colgroup>
                <thead>
                    <tr>
                        <th>"场所"</th>
                        <th>"样本"</th>
                        <th>"REST 延迟"</th>
                        <th>"WS P99"</th>
                        <th>"成交率"</th>
                        <th>"平均滑点"</th>
                        <th>"7D 可用率"</th>
                        <th>"运行数据依据"</th>
                    </tr>
                </thead>
                <tbody>
                    <Show when=move || rows.with(|table| table.rows.is_empty())>
                        {move || section_state_row(section.get().empty_text("等待场所执行质量"), "8")}
                    </Show>
                    <For each=move || rows.get().rows key=|row| row.venue.clone() children=move |initial| {
                        let row = Memo::new(move |_| rows.with(|table| table.rows.iter().find(|row| row.venue == initial.venue).cloned().unwrap_or_else(|| initial.clone())));
                        view! { <QualityRow row=row selected=selected/> }
                    }/>
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn QualityRow(row: Memo<VenueQuality>, selected: RwSignal<Option<String>>) -> impl IntoView {
    let selection_venue = row.get_untracked().venue;
    let state_venue = selection_venue.clone();
    let is_selected =
        Memo::new(move |_| selected.with(|current| current.as_ref() == Some(&state_venue)));

    view! {
        <tr
            class:is-selected=move || is_selected.get()
            aria-selected=move || is_selected.get().to_string()
        >
            <td><strong>{move || row.get().venue}</strong></td>
            <td class=move || sample_status_class(&row.get())>{move || sample_status_label(&row.get())}</td>
            <td class=move || latency_class(&row.get())>{move || latency_value(&row.get())}</td>
            <td class=move || jitter_class(&row.get())>{move || jitter_value(&row.get())}</td>
            <td class=move || fill_class(&row.get())>{move || fill_value(&row.get())}</td>
            <td class=move || slippage_class(&row.get())>{move || slippage_value(&row.get())}</td>
            <td class=move || uptime_class(&row.get())>{move || uptime_value(&row.get())}</td>
            <td class=move || operation_class(&row.get())>
                <button
                    class="review-quality-evidence-action"
                    type="button"
                    aria-controls=QUALITY_DETAIL_ID
                    aria-expanded=move || is_selected.get().to_string()
                    on:click=move |_| {
                        let is_selected = selected.with(|current| current.as_deref() == Some(selection_venue.as_str()));
                        selected.set((!is_selected).then(|| selection_venue.clone()));
                    }
                >
                    <strong>{move || operation_value(&row.get())}</strong>
                    <small>{move || if is_selected.get() { "收起" } else { "查看" }}</small>
                </button>
            </td>
        </tr>
    }
}
