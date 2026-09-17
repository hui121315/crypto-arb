use super::*;

pub(super) fn market_diagnostics_panel(
    state: LoadState<MarketDataDiagnosticsSnapshot>,
    access_table: &TableRuntimeHandle<MarketCacheAccessRow>,
    status_table: &TableRuntimeHandle<MarketDataSnapshotStatusRow>,
) -> AnyView {
    let (snapshot, snapshot_problem) = match state {
        LoadState::Ready(snapshot) => (snapshot, None),
        LoadState::Stale {
            value: snapshot,
            problem,
        } => (snapshot, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取行情诊断失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取行情诊断"</div> }.into_any();
        }
    };

    view! {
        <>
            {market_summary(&snapshot)}
            {snapshot_problem.map(|problem| view! {
                <em class="settings-message is-error">
                    {diagnostics_stale_problem_message("行情诊断刷新失败", &problem)}
                </em>
            })}
            {market_status_table(status_table)}
            {market_access_table(access_table)}
        </>
    }
    .into_any()
}

pub(super) fn row_evidence_panel(
    state: LoadState<shared_types::FundingRatesEnvelope>,
    table: &TableRuntimeHandle<MarketDataRowEvidence>,
) -> AnyView {
    let (envelope, envelope_problem) = match state {
        LoadState::Ready(envelope) => (envelope, None),
        LoadState::Stale {
            value: envelope,
            problem,
        } => (envelope, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取逐行行情证据失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取逐行行情证据"</div> }.into_any();
        }
    };
    let row_count = envelope.row_evidence.len();
    let runtime_summary = funding_runtime_summary(&envelope);
    let runtime = table.runtime.get();
    let total = table.total;
    let current_page = table.current_page;
    let visible_rows = runtime
        .rows
        .into_iter()
        .map(row_evidence_row)
        .collect_view();
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"逐行行情证据"</strong>
                <span class="funding-runtime-health">{runtime_summary}</span>
                <em>"只读行情证据，不代表可交易或可对冲"</em>
            </div>
            {envelope_problem.map(|problem| view! {
                <em class="settings-message is-error">
                    {diagnostics_stale_problem_message("逐行行情证据刷新失败", &problem)}
                </em>
            })}
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"标的"</th>
                            <th>"Feed"</th>
                            <th>"健康"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(4, "暂无逐行行情证据")
                            } else {
                                visible_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > ROW_EVIDENCE_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, ROW_EVIDENCE_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

pub(super) fn funding_runtime_summary(envelope: &shared_types::FundingRatesEnvelope) -> String {
    let mut parts = vec![
        format!("{} 条 funding 行", envelope.row_evidence.len()),
        market_health_label(&envelope.health),
    ];
    if let Some(problem) = envelope.health.problem.as_ref() {
        parts.push(operation_problem_label(problem));
    }
    if let Some(retry_after_ms) = envelope
        .retry_after_ms
        .filter(|value| Some(*value) != envelope.health.retry_after_ms)
    {
        parts.push(format!("envelope retry {retry_after_ms}ms"));
    }
    parts.join(" · ")
}

pub(super) fn row_evidence_row(row: MarketDataRowEvidence) -> impl IntoView {
    let health = market_health_label(&row.health);
    view! {
        <tr>
            <td>{row.venue}</td>
            <td>{row.symbol}</td>
            <td>{row.operation.as_str()}</td>
            <td><em>{health}</em></td>
        </tr>
    }
}

pub(super) fn market_summary(snapshot: &MarketDataDiagnosticsSnapshot) -> AnyView {
    let cache = &snapshot.cache;
    let baseline = &snapshot.rest_baseline;
    view! {
        <div class="settings-summary-grid">
            <div>
                <strong>"行情缓存"</strong>
                <span>{format!("hit {} / miss {} / stale {}", cache.hit_total, cache.miss_total, cache.stale_total)}</span>
                <em>{format!("命中率 {}", ratio_label(cache.hit_ratio))}</em>
            </div>
            <div>
                <strong>"快照 stale 服务"</strong>
                <span>{format!("perp {} / spot {}", cache.perp_ticker_snapshot_served_stale_total, cache.spot_tick_snapshot_served_stale_total)}</span>
                <em>"singleflight 忙时返回有界旧快照"</em>
            </div>
            <div>
                <strong>"Orderbook Guard"</strong>
                <span>{format!("keys {} / in-flight {}", baseline.orderbook_guard_keys, baseline.orderbook_in_flight)}</span>
                <em>{format!("wait {} 次 / {}ms", baseline.orderbook_wait_count_total, baseline.orderbook_wait_ms_total)}</em>
            </div>
            <div>
                <strong>"Baseline 生命周期"</strong>
                <span>{format!("evicted {} / oldest idle {}ms", baseline.orderbook_guard_evicted_total, baseline.orderbook_guard_oldest_idle_ms)}</span>
                <em>{format!("snapshot waits {} 次 / {}ms", baseline.snapshot_wait_count_total, baseline.snapshot_wait_ms_total)}</em>
            </div>
        </div>
    }
    .into_any()
}

pub(super) fn market_status_table(
    table: &TableRuntimeHandle<MarketDataSnapshotStatusRow>,
) -> AnyView {
    let runtime = table.runtime.get();
    let row_count = table.total.get();
    let total = table.total;
    let current_page = table.current_page;
    let visible_rows = runtime
        .rows
        .into_iter()
        .map(market_status_row)
        .collect_view();
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"行情运行态"</strong>
                <span>{format!("{row_count} 条 feed 状态")}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"Feed"</th>
                            <th>"健康"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(3, "暂无行情运行态状态")
                            } else {
                                visible_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > STATUS_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, STATUS_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

pub(super) fn market_status_row(row: MarketDataSnapshotStatusRow) -> impl IntoView {
    let health = market_health_label(&row.health);
    view! {
        <tr>
            <td>{row.venue}</td>
            <td>{row.operation.as_str()}</td>
            <td><em>{health}</em></td>
        </tr>
    }
}

pub(super) fn market_access_table(table: &TableRuntimeHandle<MarketCacheAccessRow>) -> AnyView {
    let runtime = table.runtime.get();
    let row_count = table.total.get();
    let total = table.total;
    let current_page = table.current_page;
    let visible_rows = runtime
        .rows
        .into_iter()
        .map(market_access_row)
        .collect_view();
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"缓存访问分布"</strong>
                <span>{format!("{row_count} 个低基数标签")}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"Feed"</th>
                            <th>"结果"</th>
                            <th>"来源"</th>
                            <th>"质量"</th>
                            <th>"次数"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(5, "暂无缓存访问样本")
                            } else {
                                visible_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > ACCESS_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, ACCESS_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

pub(super) fn market_access_row(row: MarketCacheAccessRow) -> impl IntoView {
    view! {
        <tr>
            <td>{row.feed}</td>
            <td>{row.outcome}</td>
            <td>{market_source_label(row.source)}</td>
            <td>{market_quality_label(row.quality)}</td>
            <td>{row.count}</td>
        </tr>
    }
}
