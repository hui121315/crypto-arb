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
        LoadState::Error(problem) => return problem_cell("读取逐行行情数据依据失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取逐行行情数据依据"</div> }.into_any();
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
                <strong>"逐行行情数据依据"</strong>
                <span class="funding-runtime-health">{runtime_summary}</span>
                <em>"只读行情数据依据，不代表可交易或可对冲"</em>
            </div>
            {envelope_problem.map(|problem| view! {
                <em class="settings-message is-error">
                    {diagnostics_stale_problem_message("逐行行情数据依据刷新失败", &problem)}
                </em>
            })}
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"标的"</th>
                            <th>"行情类型"</th>
                            <th>"状态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(4, "暂无逐行行情数据依据")
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
        format!("{} 条资金费数据", envelope.row_evidence.len()),
        market_health_label(&envelope.health),
    ];
    if let Some(problem) = envelope.health.problem.as_ref() {
        parts.push(operation_problem_label(problem));
    }
    if let Some(retry_after_ms) = envelope
        .retry_after_ms
        .filter(|value| Some(*value) != envelope.health.retry_after_ms)
    {
        parts.push(format!("整批数据 {retry_after_ms}ms 后重试"));
    }
    parts.join(" · ")
}

pub(super) fn row_evidence_row(row: MarketDataRowEvidence) -> impl IntoView {
    let health = market_health_label(&row.health);
    view! {
        <tr>
            <td>{row.venue}</td>
            <td>{row.symbol}</td>
            <td title=row.operation.as_str()>{feed_label(row.operation.as_str())}</td>
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
                <strong>"本地行情读取"</strong>
                <span>{format!("已读取 {} / 未找到 {} / 已过期 {}", cache.hit_total, cache.miss_total, cache.stale_total)}</span>
                <em>{format!("直接使用本地数据的比例 {}", ratio_label(cache.hit_ratio))}</em>
            </div>
            <div>
                <strong>"暂用上次行情"</strong>
                <span>{format!("永续 {} 次 / 现货 {} 次", cache.perp_ticker_snapshot_served_stale_total, cache.spot_tick_snapshot_served_stale_total)}</span>
                <em>"新数据读取中，暂时保留允许时间内的上次数据"</em>
            </div>
            <div>
                <strong>"买卖报价查询"</strong>
                <span>{format!("跟踪 {} 项 / 正在查询 {} 项", baseline.orderbook_guard_keys, baseline.orderbook_in_flight)}</span>
                <em>{format!("累计等待 {} 次 / {}ms", baseline.orderbook_wait_count_total, baseline.orderbook_wait_ms_total)}</em>
            </div>
            <div>
                <strong>"闲置查询清理"</strong>
                <span>{format!("已清理 {} 项 / 最长闲置 {}ms", baseline.orderbook_guard_evicted_total, baseline.orderbook_guard_oldest_idle_ms)}</span>
                <em>{format!("行情查询累计等待 {} 次 / {}ms", baseline.snapshot_wait_count_total, baseline.snapshot_wait_ms_total)}</em>
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
                <strong>"行情运行状态"</strong>
                <span>{format!("{row_count} 项行情状态")}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"行情类型"</th>
                            <th>"状态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(3, "尚未收到行情状态")
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
            <td title=row.operation.as_str()>{feed_label(row.operation.as_str())}</td>
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
                <strong>"行情读取统计"</strong>
                <span>{format!("{row_count} 类读取记录")}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"行情类型"</th>
                            <th>"结果"</th>
                            <th>"来源"</th>
                            <th>"质量"</th>
                            <th>"次数"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                empty_table_row(5, "暂无行情读取记录")
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
    let feed = feed_label(&row.feed).to_owned();
    let outcome = match row.outcome.as_str() {
        "hit" => "已读取",
        "miss" => "未找到",
        "stale" => "已过期",
        other => other,
    }.to_owned();
    view! {
        <tr>
            <td title=row.feed>{feed}</td>
            <td title=row.outcome>{outcome}</td>
            <td>{market_source_label(row.source)}</td>
            <td>{market_quality_label(row.quality)}</td>
            <td>{row.count}</td>
        </tr>
    }
}

fn feed_label(feed: &str) -> &str {
    match feed {
        "funding_rates" => "资金费率",
        "perp_tickers" => "永续行情",
        "spot_ticks" => "现货行情",
        "orderbooks" => "买卖报价与数量",
        "index_compositions" => "指数价格组成",
        "metadata" => "基础资料",
        "fee_schedule" => "交易费率",
        "ws_funding" => "资金费率实时推送",
        "ws_ticker" => "合约行情实时推送",
        "ws_spot_ticks" => "现货行情实时推送",
        other => other,
    }
}
