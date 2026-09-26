use super::{derive::ReviewStatePresentation, task_summary::ReviewTaskSummary, ReviewRuntime};
use super::super::data::next_request_gate;
use crate::panels::modules::timestamp::local_date_hm;
use leptos::{prelude::*, task::spawn_local};
use shared_types::review::settlements::*;

#[derive(Clone, Copy)]
pub(super) struct Records {
    query: RwSignal<SettlementReviewQuery>,
    snapshot: RwSignal<Option<SettlementReviewSnapshot>>,
    problem: RwSignal<Option<String>>,
    pub loading: RwSignal<bool>,
    pub summary: Memo<ReviewTaskSummary>,
    pub presentation: Memo<ReviewStatePresentation>,
}

pub(super) fn use_records(runtime: ReviewRuntime, active: Memo<bool>) -> Records {
    let query = RwSignal::new(runtime.settlement_scope.get_untracked().unwrap_or_default());
    let snapshot = RwSignal::new(None::<SettlementReviewSnapshot>);
    let problem = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let attempted = RwSignal::new(None::<(SettlementReviewQuery, u64)>);
    let request_version = RwSignal::new(0_u64);
    let client = runtime.connection.client();
    Effect::new(move |_| {
        let next = runtime.settlement_scope.get().unwrap_or_default();
        if next != query.get_untracked() {
            query.set(next);
            snapshot.set(None);
            problem.set(None);
        }
    });
    Effect::new(move |_| {
        if !active.get() || !runtime.connection.current() { return; }
        let key = (query.get(), runtime.refresh_nonce.get());
        if attempted.with_untracked(|prev| prev.as_ref() == Some(&key)) { return; }
        attempted.set(Some(key.clone()));
        let gate = next_request_gate(request_version);
        loading.set(true);
        let client = client.clone();
        spawn_local(async move {
            if !runtime.connection.current() || !gate.is_latest() { return; }
            let result = client.review_settlements(&key.0).await;
            if !runtime.connection.current() || !gate.is_latest() { return; }
            if query.try_get_untracked().as_ref() == Some(&key.0)
                && runtime.refresh_nonce.try_get_untracked() == Some(key.1) {
                match result {
                    Ok(value) => { snapshot.try_set(Some(value)); problem.try_set(None); }
                    Err(error) => { problem.try_set(Some(error.problem.message)); }
                }
            }
            loading.try_set(false);
        });
    });
    let summary = Memo::new(move |_| ReviewTaskSummary {
        value: snapshot.with(|s| s.as_ref().map(|s| format!("{} 条记录", s.rows.len()))).unwrap_or_else(||
            if loading.get() { "读取中" } else if problem.get().is_some() { "不可用" } else { "按需读取" }.into()),
        badge: if problem.get().is_some() || snapshot.with(|s| s.as_ref().is_some_and(|s| !s.problems.is_empty())) { "读取异常" } else { "原始收支" }.into(),
        tone: if problem.get().is_some() || snapshot.with(|s| s.as_ref().is_some_and(|s| !s.problems.is_empty())) { "is-warning" } else { "" },
    });
    let presentation = Memo::new(move |_| {
        let error = problem.get();
        let saved = snapshot.get();
        let storage_problem = saved.as_ref().filter(|s| !s.problems.is_empty()).map(|s| s.problems.join("；"));
        ReviewStatePresentation {
            summary: if loading.get() { "正在读取本地收支记录".into() }
                else if error.is_some() { if saved.is_some() { "刷新失败 · 显示上次记录" } else { "收支记录读取失败" }.into() }
                else if storage_problem.is_some() { "记录来源异常 · 仅展示已读取部分".into() }
                else { "已保存处理结果 · 只读复盘".into() },
            badge: saved.as_ref().and_then(|s| local_date_hm(s.observed_at_ms)).unwrap_or_default(),
            detail: error.clone().or(storage_problem.clone()).unwrap_or_else(|| "从本地交易记录读取，不重新询价、不触发订单或资金动作；记录时间是处理结果更新时间。".into()),
            tone: if error.is_some() || storage_problem.is_some() { "is-warning" } else { "" }, action: "记录范围",
        }
    });
    Records { query, snapshot, problem, loading, summary, presentation }
}

pub(super) fn panel(records: Records, runtime: ReviewRuntime) -> impl IntoView {
    let rows = Memo::new(move |_| records.snapshot.with(|s| s.as_ref().map(|s| s.rows.clone()).unwrap_or_default()));
    view! {
        <section class="review-settlements" aria-label="链上与股票收支复盘">
            <div class="review-record-scope">
                <div><strong>{move || records.query.get().record.map(|id| format!("关联记录 · {id}")).unwrap_or_else(|| "最近本地保留记录".into())}</strong>
                    <small>"不与通用对冲绩效混算；原币变化、美元折算与已实现利润分开。"</small></div>
                <label>"来源"
                    <select aria-label="收支记录来源" prop:value=move || records.query.get().source.slug()
                        on:change=move |ev| if let Some(source) = SettlementSource::parse(&event_target_value(&ev)) {
                            runtime.settlement_scope.set(Some(SettlementReviewQuery { source, record: None }));
                        }>
                        {[SettlementSource::All, SettlementSource::Onchain, SettlementSource::CrossChain, SettlementSource::Stocks, SettlementSource::StockPeer]
                            .into_iter().map(|source| view! { <option value=source.slug() selected=move || records.query.get().source == source>{source.label()}</option> }).collect_view()}
                    </select>
                </label>
                <Show when=move || records.query.get().record.is_some()>
                    <button class="row-action" on:click=move |_| runtime.settlement_scope.set(Some(SettlementReviewQuery {
                        source: records.query.get_untracked().source, record: None,
                    }))>"查看该来源记录"</button>
                </Show>
            </div>
            {move || records.snapshot.get().map(|s| s.problems.into_iter().map(|p| view! { <p class="review-settlement-warning" role="alert">{p}</p> }).collect_view())}
            <Show when=move || !records.loading.get() && rows.with(Vec::is_empty)>
                <p class="review-settlement-empty" role="status">{move || if records.problem.get().is_some() { "未能读取记录，请重试。" }
                    else if records.query.get().record.is_some() { "未找到这条原记录；不会替换为其他交易，也不代表未成交或收益为零。" }
                    else { "当前来源没有本地保留记录；不代表账户从未交易。" }}</p>
            </Show>
            <div class="review-settlement-list">
                <For each=move || rows.get() key=|row| (row.source.slug(), row.id.clone()) children=move |initial| {
                    let source = initial.source;
                    let id = initial.id.clone();
                    let current = Memo::new(move |_| rows.with(|rows| rows.iter().find(|row| row.source == source && row.id == id).cloned()).unwrap_or_else(|| initial.clone()));
                    view! {
                        <article class="review-settlement-row">
                            <header><div><strong>{move || current.with(|r| if r.title.is_empty() { r.source.label().into() } else { r.title.clone() })}</strong>
                                <small>{move || current.with(|r| format!("{} · {}", r.source.label(), local_date_hm(r.updated_at_ms).unwrap_or_else(|| "更新时间未知".into())))}</small></div>
                                <span class:review-settlement-warning=move || current.with(|r| r.attention)>{move || current.with(|r| r.execution_state.clone())}</span></header>
                            <div class="review-settlement-accounting"><strong>{move || current.with(|r| r.accounting_state.clone())}</strong>
                                <dl>{move || current.with(|r| r.amounts.clone()).into_iter().map(|a| view! {
                                    <div><dt>{a.label}</dt><dd class="num">{a.amount.map(|v| format!("{v} {}", a.asset)).unwrap_or_else(|| "待核算".into())}</dd></div>
                                }).collect_view()}</dl>
                            </div>
                            <details><summary>"原始编号与核算范围"</summary>
                                <code>{move || current.with(|r| r.id.clone())}</code>
                                {move || current.with(|r| r.notes.clone()).into_iter().map(|note| view! { <p>{note}</p> }).collect_view()}
                                <dl>{move || current.with(|r| r.references.clone()).into_iter().map(|(label, value)| view! { <div><dt>{label}</dt><dd>{value}</dd></div> }).collect_view()}</dl>
                            </details>
                        </article>
                    }
                }/>
            </div>
            <Show when=move || records.snapshot.with(|s| s.as_ref().is_some_and(|s| s.truncated))>
                <p class="review-settlement-warning">"当前仅显示最近 100 条；从原执行记录的复盘入口可按编号精确读取更早记录。"</p>
            </Show>
        </section>
    }
}
