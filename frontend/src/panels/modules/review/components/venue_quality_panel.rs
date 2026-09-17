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
                {move || selected_row.get().map(|row| venue_quality_detail(row, close_detail))}
            </div>
            {move || has_loaded_context.get().then(|| view! {
                <details class="review-quality-disclosure">
                    <summary><strong>"场所汇总雷达"</strong><span>"辅助视图 · 不替代表格"</span></summary>
                    {venue_radar(rows, chart_meta)}
                </details>
            })}
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
                        <th>"运行证据"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        let section = section.get();
                        let rows = rows.get().rows;
                        if rows.is_empty() {
                            return view! {
                                {section_state_row(section.empty_text("等待场所执行质量"), "8")}
                            }.into_any();
                        }
                        rows.into_iter().map(|row| view! { <QualityRow row=row selected=selected/> }).collect_view().into_any()
                    }}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn QualityRow(row: VenueQuality, selected: RwSignal<Option<String>>) -> impl IntoView {
    let sample = sample_status_class(&row);
    let sample_text = sample_status_label(&row);
    let latency = latency_class(&row);
    let latency_text = latency_value(&row);
    let jitter = jitter_class(&row);
    let jitter_text = jitter_value(&row);
    let fill = fill_class(&row);
    let fill_text = fill_value(&row);
    let slip = slippage_class(&row);
    let slip_text = slippage_value(&row);
    let uptime = uptime_class(&row);
    let uptime_text = uptime_value(&row);
    let operation = operation_class(&row);
    let operation_text = operation_value(&row);
    let selection_venue = row.venue.clone();
    let state_venue = row.venue.clone();
    let aria_venue = row.venue.clone();
    let expanded_venue = row.venue.clone();
    let label_venue = row.venue.clone();
    let display_venue = row.venue;

    view! {
        <tr
            class:is-selected=move || selected.with(|current| current.as_deref() == Some(state_venue.as_str()))
            aria-selected=move || selected.with(|current| current.as_deref() == Some(aria_venue.as_str())).to_string()
        >
            <td><strong>{display_venue}</strong></td>
            <td class=sample>{sample_text}</td>
            <td class=latency>{latency_text}</td>
            <td class=jitter>{jitter_text}</td>
            <td class=fill>{fill_text}</td>
            <td class=slip>{slip_text}</td>
            <td class=uptime>{uptime_text}</td>
            <td class=operation>
                <button
                    class="review-quality-evidence-action"
                    type="button"
                    aria-controls=QUALITY_DETAIL_ID
                    aria-expanded=move || selected.with(|current| current.as_deref() == Some(expanded_venue.as_str())).to_string()
                    on:click=move |_| {
                        let is_selected = selected.with(|current| current.as_deref() == Some(selection_venue.as_str()));
                        selected.set((!is_selected).then(|| selection_venue.clone()));
                    }
                >
                    <strong>{operation_text}</strong>
                    <small>{move || if selected.with(|current| current.as_deref() == Some(label_venue.as_str())) { "收起" } else { "查看" }}</small>
                </button>
            </td>
        </tr>
    }
}
