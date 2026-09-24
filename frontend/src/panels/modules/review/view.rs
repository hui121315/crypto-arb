use crate::panels::shared::{ModuleHeader, Surface};
use crate::state::module_runtime::{store_choice, stored_choice};
use leptos::prelude::*;

use super::components::{executed_tab, missed_tab, strategy_tab, venue_quality_panel};
use super::data::{
    use_executed, use_missed, use_perf, use_runtime_projection, use_venue_quality, ReviewRuntime,
};

#[path = "view/availability.rs"]
mod availability;
#[path = "view/derive.rs"]
mod derive;
#[path = "view/tabs.rs"]
mod tabs;
#[path = "view/task_summary.rs"]
mod task_summary;
use availability::{review_available_result, review_result_suggestion, review_task_availability};
use derive::{
    review_page, review_rows, review_state_presentation, venue_quality_chart_meta,
    venue_quality_rows, venue_quality_state_presentation, ReviewStatePresentation,
};
use tabs::{focus_review_tab, review_tab_from_key, tab_button, ReviewTab};
use task_summary::{review_task_summary, venue_quality_task_summary};

const REVIEW_TAB_STORAGE_KEY: &str = "crossline.review.activeTab";

pub(in crate::panels) fn review_module(runtime: ReviewRuntime) -> impl IntoView {
    let active = RwSignal::new(
        stored_choice(REVIEW_TAB_STORAGE_KEY, ReviewTab::from_slug).unwrap_or(ReviewTab::Executed),
    );
    let executed_tab_ref = NodeRef::<leptos::html::Button>::new();
    let missed_tab_ref = NodeRef::<leptos::html::Button>::new();
    let strategy_tab_ref = NodeRef::<leptos::html::Button>::new();
    let venue_quality_tab_ref = NodeRef::<leptos::html::Button>::new();
    let refreshing = use_runtime_projection(runtime);
    let executed = use_executed(runtime);
    let missed = use_missed(runtime);
    let perf = use_perf(runtime);
    let venue_quality = use_venue_quality(runtime);
    let executed_rows = Memo::new(move |_| review_rows(&executed.state.get()));
    let missed_rows = Memo::new(move |_| review_rows(&missed.state.get()));
    let perf_rows = Memo::new(move |_| review_rows(&perf.get()));
    let quality_rows = Memo::new(move |_| venue_quality_rows(&venue_quality.get()));
    let quality_chart_meta = Memo::new(move |_| {
        venue_quality_chart_meta(&venue_quality.get(), js_sys::Date::now() as i64)
    });
    let executed_page = Memo::new(move |_| review_page(&executed.state.get()));
    let missed_page = Memo::new(move |_| review_page(&missed.state.get()));
    let executed_loading = Memo::new(move |_| executed.loading.get());
    let missed_loading = Memo::new(move |_| missed.loading.get());
    let active_state = Memo::new(move |_| match active.get() {
        ReviewTab::Executed => review_state_presentation(&executed.state.get()),
        ReviewTab::Missed => review_state_presentation(&missed.state.get()),
        ReviewTab::Strategy => review_state_presentation(&perf.get()),
        ReviewTab::VenueQuality => venue_quality_state_presentation(&venue_quality.get()),
    });
    let executed_summary = Memo::new(move |_| review_task_summary(&executed.state.get(), "条记录"));
    let missed_summary = Memo::new(move |_| review_task_summary(&missed.state.get(), "条记录"));
    let strategy_summary = Memo::new(move |_| review_task_summary(&perf.get(), "个策略"));
    let venue_quality_summary =
        Memo::new(move |_| venue_quality_task_summary(&venue_quality.get()));
    let content_sized = Memo::new(move |_| match active.get() {
        ReviewTab::Executed => executed_rows.with(|section| section.rows.len() <= 6),
        ReviewTab::Missed => missed_rows.with(|section| section.rows.len() <= 6),
        ReviewTab::Strategy => perf_rows.with(|section| section.rows.len() <= 4),
        ReviewTab::VenueQuality => quality_rows.with(|section| section.rows.len() <= 5),
    });
    let available_result = Memo::new(move |_| {
        review_result_suggestion(
            active.get(),
            [
                review_task_availability(ReviewTab::Executed, &executed_rows.get()),
                review_task_availability(ReviewTab::Strategy, &perf_rows.get()),
                review_task_availability(ReviewTab::VenueQuality, &quality_rows.get()),
                review_task_availability(ReviewTab::Missed, &missed_rows.get()),
            ],
        )
    });
    let open_available_result = Callback::new(move |next: ReviewTab| {
        active.set(next);
        focus_review_tab(
            next,
            executed_tab_ref,
            missed_tab_ref,
            strategy_tab_ref,
            venue_quality_tab_ref,
        );
    });

    Effect::new(move |_| store_choice(REVIEW_TAB_STORAGE_KEY, active.get().slug()));
    Effect::new(move |_| {
        if runtime.scope.get().is_some() {
            active.set(ReviewTab::Executed);
        }
    });

    view! {
        <section
            class="module-page review-page"
            class:is-content-sized=move || content_sized.get()
        >
            <ModuleHeader title="复盘"/>
            <Surface title="复盘工作台" meta="执行账本" class_name="full-surface">
                <div
                    class="review-tabs"
                    role="tablist"
                    aria-label="复盘任务"
                    aria-orientation="horizontal"
                    on:keydown=move |event| {
                        let next = review_tab_from_key(active.get(), event.key().as_str());
                        let Some(next) = next else { return };
                        event.prevent_default();
                        active.set(next);
                        focus_review_tab(
                            next,
                            executed_tab_ref,
                            missed_tab_ref,
                            strategy_tab_ref,
                            venue_quality_tab_ref,
                        );
                    }
                >
                    {tab_button(ReviewTab::Executed.label(), executed_summary, ReviewTab::Executed, active, executed_tab_ref)}
                    {tab_button(ReviewTab::Missed.label(), missed_summary, ReviewTab::Missed, active, missed_tab_ref)}
                    {tab_button(ReviewTab::Strategy.label(), strategy_summary, ReviewTab::Strategy, active, strategy_tab_ref)}
                    {tab_button(ReviewTab::VenueQuality.label(), venue_quality_summary, ReviewTab::VenueQuality, active, venue_quality_tab_ref)}
                </div>
                <div class="review-task-context">
                    <ReviewStateLine state=active_state/>
                    {review_available_result(available_result, open_available_result)}
                    <button class="icon-button review-refresh" title="刷新复盘记录" aria-label="刷新复盘记录"
                        disabled=move || refreshing.get() || executed.loading.get()
                        on:click=move |_| runtime.refresh_nonce.update(|value| *value = value.wrapping_add(1))>"↻"</button>
                </div>
                <div
                    class="review-tab-panel"
                    role="tabpanel"
                    id=ReviewTab::Executed.panel_id()
                    aria-labelledby=ReviewTab::Executed.tab_id()
                    tabindex=move || if active.get() == ReviewTab::Executed { 0 } else { -1 }
                    hidden=move || active.get() != ReviewTab::Executed
                >
                    <Show when=move || runtime.scope.get().is_some()>
                        <div class="review-record-scope" role="status">
                            <div><strong>"关联复盘 · 最近 365 天"</strong>
                                <span>{move || runtime.scope.get().map(|scope| {
                                    scope.close_run_id.map(|id| format!("平仓 {id}")).unwrap_or_else(|| format!("运行 {}", scope.run_id.unwrap_or_default()))
                                })}</span>
                                <small>{move || if executed.loading.get() { "正在读取关联账本" }
                                    else if executed_rows.get().rows.is_empty() { "未找到可核验的关联记录；不代表未成交、已平仓或收益为零。" }
                                    else { "按需读取的关联执行记录；其他页签仍为全局统计。" }}</small>
                            </div>
                            <a class="row-action" href="#review">"全部执行记录"</a>
                        </div>
                    </Show>
                    {executed_tab(
                        executed_rows,
                        executed_page,
                        executed_loading,
                        executed.load_cursor,
                        runtime.scope,
                    )}
                </div>
                <div
                    class="review-tab-panel"
                    role="tabpanel"
                    id=ReviewTab::Missed.panel_id()
                    aria-labelledby=ReviewTab::Missed.tab_id()
                    tabindex=move || if active.get() == ReviewTab::Missed { 0 } else { -1 }
                    hidden=move || active.get() != ReviewTab::Missed
                >
                    {missed_tab(missed_rows, missed_page, missed_loading, missed.load_cursor)}
                </div>
                <div
                    class="review-tab-panel"
                    role="tabpanel"
                    id=ReviewTab::Strategy.panel_id()
                    aria-labelledby=ReviewTab::Strategy.tab_id()
                    tabindex=move || if active.get() == ReviewTab::Strategy { 0 } else { -1 }
                    hidden=move || active.get() != ReviewTab::Strategy
                >
                    {strategy_tab(perf_rows)}
                </div>
                <div
                    class="review-tab-panel"
                    role="tabpanel"
                    id=ReviewTab::VenueQuality.panel_id()
                    aria-labelledby=ReviewTab::VenueQuality.tab_id()
                    tabindex=move || if active.get() == ReviewTab::VenueQuality { 0 } else { -1 }
                    hidden=move || active.get() != ReviewTab::VenueQuality
                >
                    {venue_quality_panel(quality_rows, quality_chart_meta)}
                </div>
            </Surface>
        </section>
    }
}

#[component]
fn ReviewStateLine(state: Memo<ReviewStatePresentation>) -> impl IntoView {
    view! {
        <details class=move || format!("review-state-disclosure {}", state.get().tone)>
            <summary>
                <span class="review-state-copy">
                    <strong>{move || state.get().summary}</strong>
                    <small>{move || state.get().badge}</small>
                </span>
                <span class="review-state-action">{move || state.get().action}</span>
            </summary>
            <p>{move || state.get().detail}</p>
        </details>
    }
}
