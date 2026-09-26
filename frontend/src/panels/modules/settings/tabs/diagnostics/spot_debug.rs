use super::spot_debug_text::{listing_evidence_label, listing_state_label, spot_tick_row};
use super::*;
use crate::panels::modules::settings::data::{SpotDebugQuery, SpotTicksEnvelope};
use shared_types::{ApiProblem, ListPage, MarketDataFanoutOutcome, VenueCoverageEntry};

pub(super) fn spot_debug_panel(symbol: RwSignal<String>, query: SpotDebugQuery) -> impl IntoView {
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"现货行情查询"</strong>
                <em>"查看指定币种的行情，不会下单，也不会改变收益计算或机会排序"</em>
            </div>
            <div class="api-base-editor adapter-editor">
                <label>
                    <span>"币种（留空查询全部）"</span>
                    <input
                        disabled=move || matches!(query.state.get(), Some(LoadState::Loading))
                        prop:value=move || symbol.get()
                        on:input=move |ev| {
                            symbol.set(event_target_value(&ev));
                            query.state.set(None);
                        }
                    />
                </label>
                <button
                    class="row-action"
                    disabled=move || matches!(query.state.get(), Some(LoadState::Loading))
                    on:click=move |_| query.submit.run(symbol.get_untracked())
                >
                    "查询现货行情"
                </button>
            </div>
            {move || spot_debug_result(query.state.get())}
        </>
    }
}

fn spot_debug_result(state: Option<LoadState<SpotTicksEnvelope>>) -> AnyView {
    let Some(state) = state else {
        return view! { <div class="empty-cell">"尚未查询现货行情"</div> }.into_any();
    };
    let envelope = match state {
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在查询现货行情"</div> }.into_any();
        }
        LoadState::Error(problem) => return problem_cell("现货行情查询失败", &problem),
        LoadState::Ready(envelope)
        | LoadState::Stale {
            value: envelope, ..
        } => envelope,
    };

    spot_debug_envelope(envelope)
}

fn spot_debug_envelope(envelope: SpotTicksEnvelope) -> AnyView {
    let shared_types::MarketDataEnvelope {
        data,
        health,
        row_evidence,
        fanout,
        ..
    } = envelope;
    let health = market_health_label(&health);
    let shared_types::SpotTicksPage {
        ticks,
        page,
        base_listing_coverage,
        query_problems,
        request_id,
    } = data;
    let row_count = ticks.len();
    let evidence_count = row_evidence.len();
    let evidence_rows = row_evidence
        .into_iter()
        .map(row_evidence_row)
        .collect_view();
    let tick_rows = ticks.into_iter().map(spot_tick_row).collect_view();
    let fanout_count = fanout.len();
    let fanout_rows = fanout.into_iter().map(fanout_row).collect_view();
    let listing_count = base_listing_coverage.len();
    let listing_rows = base_listing_coverage
        .into_iter()
        .flat_map(|coverage| {
            let base = coverage.canonical_symbol;
            coverage
                .venues
                .into_iter()
                .map(move |entry| listing_row(base.clone(), entry))
        })
        .collect_view();
    let query_problem_count = query_problems.len();
    let query_problem_rows = query_problems.iter().map(query_problem_row).collect_view();
    let page_summary = page_label(&page, request_id.as_deref());
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"Endpoint 健康"</strong>
                <span>{format!("{row_count} 条 tick")}</span>
                <span>{page_summary}</span>
                <em>{health}</em>
            </div>
            {(query_problem_count > 0).then(|| view! {
                <div class="settings-summary-line">
                    <strong>"查询修正"</strong>
                    <em>{query_problem_rows}</em>
                </div>
            })}
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"标的"</th>
                            <th>"买一价 / 卖一价"</th>
                            <th>"盘口量"</th>
                            <th>"时间戳来源"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(5, "未找到该币种的现货行情，请查看上方的接口状态和错误说明")
                            } else {
                                tick_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"标的"</th>
                            <th>"行情来源"</th>
                            <th>"健康"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if evidence_count == 0 {
                                empty_table_row(4, "尚未读到各条现货行情的数据来源")
                            } else {
                                evidence_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"接口操作"</th>
                            <th>"运行状态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if fanout_count == 0 {
                                empty_table_row(3, "尚未读到各交易所的现货查询结果，请查看接口状态")
                            } else {
                                fanout_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"币种"</th>
                            <th>"交易所"</th>
                            <th>"挂牌"</th>
                            <th>"数据依据"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if listing_count == 0 {
                                empty_table_row(4, "尚未确认这些币种是否已上架；有报价不代表可以交易")
                            } else {
                                listing_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
        </>
    }
    .into_any()
}

fn fanout_row(row: MarketDataFanoutOutcome) -> impl IntoView {
    let health = market_health_label(&row.health);
    view! {
        <tr>
            <td>{row.venue}</td>
            <td>{row.operation.as_str()}</td>
            <td><em>{health}</em></td>
        </tr>
    }
}

fn listing_row(base: String, entry: VenueCoverageEntry) -> impl IntoView {
    let state = listing_state_label(entry.state);
    let evidence = listing_evidence_label(&entry);
    view! {
        <tr>
            <td>{base}</td>
            <td>{entry.venue}</td>
            <td>{state}</td>
            <td><em>{evidence}</em></td>
        </tr>
    }
}

fn query_problem_row(problem: &ApiProblem) -> String {
    let request = problem
        .request_id
        .as_deref()
        .map(|value| format!(" request_id {value}"))
        .unwrap_or_default();
    format!("{}{}", problem.code, request)
}

fn page_label(page: &ListPage, request_id: Option<&str>) -> String {
    let next = page
        .next_cursor
        .as_deref()
        .map(|cursor| format!(" · next {cursor}"))
        .unwrap_or_default();
    let request = request_id
        .map(|value| format!(" · request_id {value}"))
        .unwrap_or_default();
    format!(
        "page {}+{} / {} · limit {}{}{}",
        page.start_offset, page.returned_count, page.total_rows, page.limit, next, request
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_labels_keep_request_id() {
        assert!(page_label(
            &ListPage {
                limit: 64,
                max_limit: 128,
                start_offset: 0,
                returned_count: 64,
                total_rows: 80,
                has_more: true,
                next_cursor: Some("64".into()),
                ..ListPage::default()
            },
            Some("req-spot"),
        )
        .contains("request_id req-spot"));
    }
}
