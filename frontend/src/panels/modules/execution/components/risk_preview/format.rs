use super::*;

pub(super) fn balance_rows_summary(rows: &[VenueBalanceInfo]) -> String {
    let mut parts = rows
        .iter()
        .take(8)
        .map(balance_row_summary)
        .collect::<Vec<_>>();
    if rows.len() > parts.len() {
        parts.push(format!("+{} 行", rows.len() - parts.len()));
    }
    parts.join(" / ")
}

pub(super) fn account_data_health_summary(rows: &[AccountDataHealth]) -> String {
    let mut parts = rows
        .iter()
        .take(4)
        .map(account_data_health_line)
        .collect::<Vec<_>>();
    if rows.len() > parts.len() {
        parts.push(format!("+{} 条", rows.len() - parts.len()));
    }
    parts.join(" / ")
}

pub(super) fn account_data_health_line(row: &AccountDataHealth) -> String {
    let mut parts = vec![account_data_health_subject(row), row.source.clone()];
    if let Some(freshness_ms) = row.freshness_ms {
        parts.push(format!("新鲜度 {freshness_ms}ms"));
    }
    if let Some(last_success_ms) = row.last_success_ms {
        parts.push(format!("成功 {last_success_ms}"));
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        parts.push(format!("重试 {retry_after_ms}ms"));
    }
    if let Some(request_id) = row.request_id.as_deref() {
        parts.push(format!("请求 {}", short_id(request_id)));
    }
    if let Some(problem) = row.last_error.as_ref() {
        parts.push(format!("{} {}", problem.code, problem.message));
    }
    parts.join(" ")
}

pub(super) fn account_data_health_subject(row: &AccountDataHealth) -> String {
    let venue = row.subject.venue.as_deref().unwrap_or("账户");
    match row.subject.kind {
        AccountFieldSubjectKind::Account => venue.to_owned(),
        AccountFieldSubjectKind::Balance => match row.subject.currency.as_deref() {
            Some(currency) => format!("{venue} {currency}"),
            None => venue.to_owned(),
        },
        AccountFieldSubjectKind::Position => match row.subject.symbol.as_deref() {
            Some(symbol) => format!("{venue} {symbol}"),
            None => venue.to_owned(),
        },
        AccountFieldSubjectKind::OpenOrder => match (
            row.subject.symbol.as_deref(),
            row.subject.order_id.as_deref(),
        ) {
            (Some(symbol), Some(order_id)) => format!("{venue} {symbol} {order_id}"),
            (Some(symbol), None) => format!("{venue} {symbol}"),
            _ => venue.to_owned(),
        },
    }
}

pub(super) fn balance_row_summary(row: &VenueBalanceInfo) -> String {
    format!(
        "{} {} 可用 {} 总额 {} 占用 {}",
        row.venue,
        row.currency,
        money(row.available),
        money(row.total),
        money(row.frozen)
    )
}

pub(super) fn preflight_status_label(status: HedgePreflightStatus) -> &'static str {
    match status {
        HedgePreflightStatus::Passed => "通过",
        HedgePreflightStatus::Blocked => "阻断",
        HedgePreflightStatus::Failed => "失败",
        HedgePreflightStatus::Skipped => "跳过",
    }
}

pub(super) fn leg_role_label(role: HedgeLegRole) -> &'static str {
    match role {
        HedgeLegRole::Long => "多腿",
        HedgeLegRole::Short => "空腿",
    }
}

pub(super) fn order_kind_label(kind: VenueOrderKind) -> &'static str {
    match kind {
        VenueOrderKind::Limit => "限价",
        VenueOrderKind::PostOnly => "Post-only",
        VenueOrderKind::NativeMarket => "原生市价",
        VenueOrderKind::ProtectedIoc => "保护 IOC",
        VenueOrderKind::PriceZeroIoc => "0价 IOC",
        VenueOrderKind::MarketLike => "显式 market-like",
        VenueOrderKind::MarketLikeRequired => "待选型",
    }
}

pub(super) fn payload_policy_label(policy: OrderPayloadPricePolicy) -> &'static str {
    match policy {
        OrderPayloadPricePolicy::LimitPrice => "限价",
        OrderPayloadPricePolicy::Omit => "不带价格",
        OrderPayloadPricePolicy::ProtectionPrice => "保护价",
        OrderPayloadPricePolicy::ZeroPrice => "价格 0",
        OrderPayloadPricePolicy::MarketLikeNoPrice => "待原生类型",
    }
}

pub(super) fn preview_id(preview: &ExecutionPreview) -> String {
    short_id(&full_preview_id(preview))
}

pub(super) fn full_preview_id(preview: &ExecutionPreview) -> String {
    preview
        .idempotency_key
        .as_deref()
        .unwrap_or(&preview.opportunity_id)
        .to_string()
}

pub(super) fn ticket_text(preview: &ExecutionPreview) -> String {
    preview
        .ticket_id
        .as_deref()
        .map(short_id)
        .unwrap_or_else(|| "等待票据".into())
}

pub(super) fn risk_note_text(preview: &ExecutionPreview) -> String {
    let blockers = preview
        .risk
        .blockers
        .iter()
        .map(String::as_str)
        .filter(|blocker| *blocker != preview.risk.note)
        .collect::<Vec<_>>()
        .join("；");
    if blockers.is_empty() {
        preview.risk.note.clone()
    } else {
        format!("{}：{blockers}", preview.risk.note)
    }
}

pub(super) fn ready_money(preview: &ExecutionPreview, value: f64, pending: &'static str) -> String {
    if preview.is_ready() {
        money(value)
    } else {
        pending.into()
    }
}

pub(super) fn net_edge_text(preview: &ExecutionPreview) -> String {
    if !preview.is_ready() {
        return "待预检".into();
    }
    if preview.one_cycle_cost.is_none() {
        return "缺成本证据".into();
    }
    money(preview.net_edge_usd())
}

pub(super) fn cost_money(preview: &ExecutionPreview, value: f64, pending: &'static str) -> String {
    if !preview.is_ready() {
        return pending.into();
    }
    if preview.one_cycle_cost.is_none() {
        return "缺成本证据".into();
    }
    money(value)
}

pub(super) fn short_id(value: &str) -> String {
    value.chars().take(18).collect()
}

pub(super) fn pct(value: f64) -> String {
    format!("{value:.1}%")
}

pub(super) fn pct_from_bps(value_bps: f64) -> String {
    format!("{:+.3}%", value_bps / 100.0)
}

pub(super) fn pct_opt(value: Option<f64>) -> String {
    value.map(pct).unwrap_or_else(|| "--".into())
}

pub(super) fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let value = value.abs();
    if value >= 1_000_000.0 {
        format!("{sign}${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{sign}${:.1}K", value / 1_000.0)
    } else if value >= 100.0 {
        format!("{sign}${value:.0}")
    } else if value >= 1.0 {
        format!("{sign}${value:.2}")
    } else if value >= 0.01 {
        format!("{sign}${value:.3}")
    } else if value > 0.0 {
        format!("{sign}${value:.4}")
    } else {
        "$0".into()
    }
}
