use super::*;

pub(super) const MARKET_DATA_SLOT_LABEL: &str = "MarketData";

#[component]
pub fn MarketDataStatusSlot(
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
    operation_problem: Memo<Option<ApiProblem>>,
) -> impl IntoView {
    view! {
        <div
            data-testid="status-market-data"
            class=move || scalar_slot_class(market_data_degraded(
                operation_health.get().as_ref(),
                operation_problem.get().as_ref(),
            ))
            title=move || market_data_title(
                operation_health.get().as_ref(),
                operation_problem.get().as_ref(),
            )
        >
            <span class=move || dot_class(market_data_degraded(
                operation_health.get().as_ref(),
                operation_problem.get().as_ref(),
            ))></span>
            <span class="slot-label">{MARKET_DATA_SLOT_LABEL}</span>
            <span class="num">{move || market_data_label(
                operation_health.get().as_ref(),
                operation_problem.get().as_ref(),
            )}</span>
        </div>
    }
}

pub(super) fn market_data_rows(
    snapshot: &VenueOperationHealthSnapshot,
) -> impl Iterator<Item = &VenueOperationHealth> {
    snapshot
        .rows
        .iter()
        .filter(|row| is_core_market_data_row(row))
}

fn is_core_market_data_row(row: &VenueOperationHealth) -> bool {
    VenueOperationKind::parse(&row.operation).is_market_data_execution_core_status_row()
        && is_market_data_row(row)
        && !is_hyperliquid_builder_venue(&row.venue)
        && !is_disabled_optional_market_data_row(row)
        && VenueOperationKind::parse(&row.operation) != VenueOperationKind::RestIndexCompositions
}

fn is_market_data_row(row: &VenueOperationHealth) -> bool {
    row.supported != Some(false)
        && VenueOperationKind::parse(&row.operation).class()
            == shared_types::VenueOperationClass::MarketData
}

fn is_disabled_optional_market_data_row(row: &VenueOperationHealth) -> bool {
    VenueOperationKind::parse(&row.operation) == VenueOperationKind::WatchlistPrewarm
        && row.configured == Some(false)
}

pub(super) fn market_data_degraded(
    snapshot: Option<&VenueOperationHealthSnapshot>,
    problem: Option<&ApiProblem>,
) -> bool {
    problem.is_some()
        || snapshot.is_none_or(|snapshot| {
            let mut rows = market_data_rows(snapshot).peekable();
            rows.peek().is_none() || rows.any(|row| !row.is_currently_usable())
        })
}

pub(super) fn market_data_label(
    snapshot: Option<&VenueOperationHealthSnapshot>,
    problem: Option<&ApiProblem>,
) -> String {
    if problem.is_some() {
        return "异常".into();
    }
    let Some(snapshot) = snapshot else {
        return "无证据".into();
    };
    let rows = market_data_rows(snapshot).collect::<Vec<_>>();
    if rows.is_empty() {
        return "无证据".into();
    }
    let usable = rows.iter().filter(|row| row.is_currently_usable()).count();
    format!("{usable}/{}", rows.len())
}

pub(super) fn market_data_title(
    snapshot: Option<&VenueOperationHealthSnapshot>,
    problem: Option<&ApiProblem>,
) -> String {
    let operation = snapshot
        .and_then(|snapshot| market_data_rows(snapshot).max_by_key(|row| status_rank(row.status)))
        .map(operation_summary)
        .unwrap_or_else(|| "等待 venue market-data operation-health 证据".into());
    title_parts([
        operation,
        snapshot.map(recovery_market_summary).unwrap_or_default(),
        snapshot.map(builder_market_summary).unwrap_or_default(),
        snapshot.map(index_composition_summary).unwrap_or_default(),
        snapshot.map(disabled_optional_summary).unwrap_or_default(),
        problem.map(api_problem_summary).unwrap_or_default(),
    ])
}

fn recovery_market_summary(snapshot: &VenueOperationHealthSnapshot) -> String {
    let rows = snapshot
        .rows
        .iter()
        .filter(|row| {
            row.supported != Some(false)
                && VenueOperationKind::parse(&row.operation).is_market_data_recovery_status_row()
                && !is_hyperliquid_builder_venue(&row.venue)
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }
    let usable = rows.iter().filter(|row| row.is_currently_usable()).count();
    format!(
        "REST 冷启动/恢复 {usable}/{} 可用（不计实时核心）",
        rows.len()
    )
}

fn index_composition_summary(snapshot: &VenueOperationHealthSnapshot) -> String {
    let rows = snapshot
        .rows
        .iter()
        .filter(|row| {
            row.supported != Some(false)
                && VenueOperationKind::parse(&row.operation)
                    == VenueOperationKind::RestIndexCompositions
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }
    let usable = rows.iter().filter(|row| row.is_currently_usable()).count();
    format!("指数成分验证 {usable}/{} 可用（不计核心）", rows.len())
}

fn builder_market_summary(snapshot: &VenueOperationHealthSnapshot) -> String {
    let rows = snapshot
        .rows
        .iter()
        .filter(|row| is_market_data_row(row) && is_hyperliquid_builder_venue(&row.venue))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }
    let usable = rows.iter().filter(|row| row.is_currently_usable()).count();
    format!(
        "Hyperliquid 扩展市场 {usable}/{} 可用（不计核心）",
        rows.len()
    )
}

fn disabled_optional_summary(snapshot: &VenueOperationHealthSnapshot) -> String {
    if snapshot
        .rows
        .iter()
        .any(|row| is_market_data_row(row) && is_disabled_optional_market_data_row(row))
    {
        "可选 watchlist 预热未启用（不计核心）".to_owned()
    } else {
        String::new()
    }
}
