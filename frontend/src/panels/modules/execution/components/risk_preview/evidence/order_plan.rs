//! `HedgeTicket` 订单编译与身份事实的展示文案。

use super::*;

pub(in crate::panels::modules::execution::components::risk_preview) fn order_plan_summary(
    preview: &ExecutionPreview,
) -> String {
    if !preview.is_ready() {
        return "待编译".into();
    }
    if preview.order_plans.is_empty() {
        return "等待订单编译".into();
    }
    let blocker_count = preview
        .order_plans
        .iter()
        .filter(|plan| {
            !plan.blockers.is_empty()
                || !plan.identity_plan().is_execution_ready()
                || (preview.execution_mode_label == "实盘"
                    && plan.validate_sizing_contract().is_err())
        })
        .count();
    if blocker_count > 0 {
        let identities = preview
            .order_plans
            .iter()
            .map(|plan| {
                let identity = plan.identity_plan();
                format!(
                    "{} > {}",
                    plan.symbol,
                    identity.native_symbol.as_deref().unwrap_or("缺原生标识")
                )
            })
            .collect::<Vec<_>>()
            .join(" / ");
        format!("{identities} · {blocker_count} 条阻断")
    } else {
        preview
            .order_plans
            .iter()
            .map(|plan| {
                let sizing = sizing_summary(plan)
                    .map(|summary| format!(" · {summary}"))
                    .unwrap_or_default();
                format!(
                    "{} {} {} > {}{}",
                    leg_role_label(plan.role),
                    order_kind_label(plan.venue_order_kind),
                    plan.identity_plan().canonical_symbol,
                    native_symbol(plan),
                    sizing
                )
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

pub(in crate::panels::modules::execution::components::risk_preview) fn order_plan_detail(
    preview: &ExecutionPreview,
) -> String {
    if preview.order_plans.is_empty() {
        return "等待后端返回订单编译计划".into();
    }
    preview
        .order_plans
        .iter()
        .map(|plan| {
            let identity = plan.identity_plan();
            let mut blockers = plan.blockers.clone();
            blockers.extend(identity.blockers.clone());
            let blocker = if blockers.is_empty() {
                "无阻断".into()
            } else {
                blockers.join("；")
            };
            let sizing = sizing_detail(plan);
            format!(
                "{} {} · canonical {} · native {} · settle {} · quote {} · product {} · {} · public client id {} · venue client id {} · finality {} · evidence {} · {} · payload {} · {}",
                leg_role_label(plan.role),
                plan.exchange,
                identity.canonical_symbol,
                native_symbol(plan),
                identity.settle_asset.as_deref().unwrap_or("MISSING"),
                identity.quote_asset.as_deref().unwrap_or("MISSING"),
                fee_product_label(identity.product),
                sizing,
                identity.client_order_id_policy.public_client_order_id,
                identity
                    .client_order_id_policy
                    .venue_client_order_id
                    .as_deref()
                    .unwrap_or("MISSING"),
                finality_source_label(identity.exchange_order_id_finality_source),
                identity_evidence_summary(&identity.evidence),
                plan.summary,
                payload_policy_label(plan.payload_price_policy),
                blocker
            )
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

fn native_symbol(plan: &shared_types::OrderCompilePlan) -> String {
    plan.instrument_spec
        .as_ref()
        .map(|instrument| instrument.native_symbol.clone())
        .or_else(|| plan.identity_plan().native_symbol)
        .unwrap_or_else(|| "MISSING".into())
}

fn sizing_summary(plan: &shared_types::OrderCompilePlan) -> Option<String> {
    let instrument = plan.instrument_spec.as_ref()?;
    let sizing = plan.sizing_plan?;
    Some(format!(
        "提交 {}：数量 {} / {} 张 · 实际 ${}",
        instrument.native_symbol,
        decimal(sizing.rounded_base_qty),
        decimal(sizing.rounded_contracts),
        decimal(sizing.actual_notional_usd)
    ))
}

fn sizing_detail(plan: &shared_types::OrderCompilePlan) -> String {
    let (Some(instrument), Some(sizing)) = (&plan.instrument_spec, plan.sizing_plan) else {
        return "instrument/sizing MISSING".into();
    };
    let contract_status = plan
        .validate_sizing_contract()
        .map(|()| "VALID")
        .unwrap_or_else(|error| error.code());
    format!(
        "合约原生标识 {} · 计价资产 {} · 结算资产 {} · 合约乘数 {} · 价格刻度 {} · 数量步长 {} · 最小数量 {} · 最小名义本金 {} · 原始数量 {} / {} 张 · 提交数量 {} / {} 张 · 目标 ${} · 实际 ${} · 差额 ${} · 舍入损耗 {} 基点 · 来源 {} · 检查时间 {} · schema {} · 合同 {}",
        instrument.native_symbol,
        instrument.quote_asset.as_deref().unwrap_or("MISSING"),
        instrument.settle_asset.as_deref().unwrap_or("MISSING"),
        optional_decimal(instrument.contract_size),
        decimal(sizing.price_tick),
        decimal(sizing.qty_step),
        optional_decimal(instrument.min_qty),
        optional_decimal(instrument.min_notional),
        decimal(sizing.raw_base_qty),
        decimal(sizing.raw_contracts),
        decimal(sizing.rounded_base_qty),
        decimal(sizing.rounded_contracts),
        decimal(sizing.target_notional_usd),
        decimal(sizing.actual_notional_usd),
        decimal(sizing.rounding_delta_usd),
        decimal_with_precision(sizing.rounding_loss_bps, 4),
        instrument.source_url.as_deref().unwrap_or("MISSING"),
        instrument.checked_at_ms,
        instrument.schema_version.as_deref().unwrap_or("MISSING"),
        contract_status
    )
}

fn optional_decimal(value: Option<f64>) -> String {
    value.map(decimal).unwrap_or_else(|| "MISSING".into())
}

fn decimal(value: f64) -> String {
    decimal_with_precision(value, 12)
}

fn decimal_with_precision(value: f64, precision: usize) -> String {
    let fixed = format!("{value:.precision$}");
    fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn identity_evidence_summary(rows: &[shared_types::OrderIdentityEvidence]) -> String {
    rows.iter()
        .map(|row| {
            format!(
                "{}={} source={} evidence={} detail={}",
                identity_evidence_kind_label(row.kind),
                identity_evidence_status_label(row.status),
                row.source.as_deref().unwrap_or("MISSING"),
                row.evidence_id.as_deref().unwrap_or("MISSING"),
                row.detail.as_deref().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn identity_evidence_kind_label(kind: shared_types::OrderIdentityEvidenceKind) -> &'static str {
    match kind {
        shared_types::OrderIdentityEvidenceKind::Metadata => "metadata",
        shared_types::OrderIdentityEvidenceKind::UserStream => "user-stream",
        shared_types::OrderIdentityEvidenceKind::OrderFinality => "finality",
        shared_types::OrderIdentityEvidenceKind::Fee => "fee",
        shared_types::OrderIdentityEvidenceKind::Unknown => "unknown",
    }
}

fn identity_evidence_status_label(
    status: shared_types::OrderIdentityEvidenceStatus,
) -> &'static str {
    match status {
        shared_types::OrderIdentityEvidenceStatus::Verified => "VERIFIED",
        shared_types::OrderIdentityEvidenceStatus::Mismatched => "MISMATCHED",
        shared_types::OrderIdentityEvidenceStatus::Unavailable => "UNAVAILABLE",
    }
}

fn finality_source_label(source: shared_types::ExchangeOrderIdFinalitySource) -> &'static str {
    match source {
        shared_types::ExchangeOrderIdFinalitySource::PrivateUserStream => "private_user_stream",
        shared_types::ExchangeOrderIdFinalitySource::RestOrderQuery => "rest_order_query",
        shared_types::ExchangeOrderIdFinalitySource::PrivateUserStreamWithRestFallback => {
            "private_user_stream_with_rest_fallback"
        }
        shared_types::ExchangeOrderIdFinalitySource::Unavailable => "UNAVAILABLE",
    }
}

fn fee_product_label(product: shared_types::FeeProduct) -> &'static str {
    match product {
        shared_types::FeeProduct::Spot => "spot",
        shared_types::FeeProduct::Perp => "perp",
        shared_types::FeeProduct::Margin => "margin",
        shared_types::FeeProduct::Unknown => "unknown",
    }
}
