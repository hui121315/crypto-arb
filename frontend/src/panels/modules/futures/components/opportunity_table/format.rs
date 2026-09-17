//! 期货机会表格单元格的纯文案派生：腿价/执行原因/成本与盈亏/来源新鲜度等。
//! 表格与单元格组件见父模块 `opportunity_table.rs`。

#[path = "format/rates.rs"]
mod rates;

use crate::panels::modules::futures::columns::ColumnId;
use crate::panels::modules::futures::data::FuturesOpportunity;
use crate::panels::modules::opportunity_format::{is_missing_quote_label, quote_price_line};
use crate::panels::modules::rate_format::{signed_bps_percent, unsigned_bps_percent};
use crate::panels::modules::timestamp::now_ms;
use shared_types::{HedgeLegRole, OpportunityListLegFunding, StrategyKind};

pub(super) use rates::{alignment_text, hours_text, signed_bps_text};

pub(super) fn leg_price_line(price: &str, evidence: Option<&str>) -> String {
    quote_price_line(price, evidence)
}

pub(super) fn has_price_quote(price: &str) -> bool {
    !is_missing_quote_label(price)
}

pub(super) fn leg_funding_text(funding: &OpportunityListLegFunding) -> String {
    let percent = if funding.rate.is_finite() {
        let value = funding.rate * 100.0;
        format!("{:+.4}%", if value.abs() < 0.000_05 { 0.0 } else { value })
    } else {
        "未知".into()
    };
    let interval = funding
        .interval_hours
        .map_or_else(|| "周期未知".into(), |hours| format!("{hours}h"));
    let settlement = funding.next_funding_time_ms.map_or_else(
        || "结算时间未知".into(),
        |next_ms| {
            let remaining_ms = next_ms.saturating_sub(now_ms());
            if remaining_ms <= 0 {
                "结算同步中".into()
            } else if remaining_ms < 60_000 {
                "<1m 后结算".into()
            } else {
                format!("{}m 后结算", remaining_ms.saturating_add(59_999) / 60_000)
            }
        },
    );
    format!("Funding {percent} / {interval} · {settlement}")
}

pub(super) fn leg_funding_cashflow_text(
    funding: &OpportunityListLegFunding,
    role: HedgeLegRole,
) -> (String, &'static str) {
    if !funding.rate.is_finite() {
        return ("现金流未知".into(), "is-unknown");
    }
    let cashflow_rate = match role {
        HedgeLegRole::Long => -funding.rate,
        HedgeLegRole::Short => funding.rate,
    };
    let percent = cashflow_rate.abs() * 100.0;
    if percent < 0.000_05 {
        ("预计持平".into(), "is-neutral")
    } else if cashflow_rate > 0.0 {
        (format!("预计收 {percent:.4}%"), "is-credit")
    } else {
        (format!("预计付 {percent:.4}%"), "is-debit")
    }
}

pub(super) fn execution_title(opp: &FuturesOpportunity) -> String {
    if opp.execution_eligible {
        "后端判定可进入对冲预览".into()
    } else {
        opp.execution_blockers
            .first()
            .cloned()
            .unwrap_or_else(|| "后端判定当前不可执行".into())
    }
}

pub(super) fn execution_reason(opp: &FuturesOpportunity) -> Option<&'static str> {
    opp.execution_blocker_summary()
}

pub(super) fn detail_value(opp: &FuturesOpportunity, col: ColumnId) -> String {
    match col {
        ColumnId::FundingCyclePercentile => opp.funding_stats.detail_text(),
        ColumnId::BorrowCost => signed_bps_text(opp.borrow_cost_bps_per_day, "未接入"),
        ColumnId::FundingAlignment => alignment_text(opp.funding_alignment_minutes),
        ColumnId::FundingCapDistance => signed_bps_text(opp.funding_cap_distance_bps, "待窗口"),
        ColumnId::MinHold => hours_text(opp.min_hold_hours),
        ColumnId::CostBreakeven => cost_breakeven_detail_text(opp),
        ColumnId::GrossOneCycleBps => format!("毛边际 {}", opp.gross_one_cycle_text()),
        ColumnId::OneCycleNetBps => one_cycle_detail_text(opp),
        ColumnId::RoundTripCostBps => {
            format!(
                "回合成本 {} · {}",
                opp.round_trip_cost_text(),
                opp.cost_evidence_label()
            )
        }
        ColumnId::IndexComposition => opp.index_composition.detail.clone(),
        ColumnId::SettlementCountdown => settlement_countdown_text(opp),
        ColumnId::StrategyKind => opp.spot_leg_mode_label().map_or_else(
            || opp.strategy_label.clone(),
            |label| format!("{} · 现货腿 {label}", opp.strategy_label),
        ),
        _ => "-".into(),
    }
}

pub(super) fn settlement_countdown_text(opp: &FuturesOpportunity) -> String {
    match opp.settlement_countdown_seconds {
        Some(seconds) if seconds <= 0 => "结算同步中".into(),
        Some(seconds) if seconds < 60 => "<1m".into(),
        Some(seconds) => format!("{}m", seconds.saturating_add(59) / 60),
        None if opp.time_to_settlement_ms > 0 && opp.time_to_settlement_ms < 60_000 => "<1m".into(),
        None if opp.time_to_settlement_ms > 0 => {
            format!(
                "{}m",
                opp.time_to_settlement_ms.saturating_add(59_999) / 60_000
            )
        }
        None => "结算时间缺证据".into(),
    }
}

pub(super) fn breakeven_text(opp: &FuturesOpportunity) -> String {
    if !opp.cost_verified {
        return "成本未验证".into();
    }
    if is_spot_cross_strategy(opp) {
        return format!("即时费后 {}", opp.one_cycle_net_text());
    }
    if is_convergence_strategy(opp) {
        return "等待价差收敛".into();
    }
    if is_projected_basis_strategy(opp) {
        return "退出基差待绑定".into();
    }
    if opp.breakeven_periods == 0 {
        return if opp.one_cycle_net_bps > 0.0 {
            "兑现条件未满足".into()
        } else {
            "单次未覆盖成本".into()
        };
    }
    format!("{}次 / {:.1}h", opp.breakeven_periods, opp.breakeven_hours)
}

pub(super) fn breakeven_context_text(opp: &FuturesOpportunity) -> String {
    if !opp.cost_verified {
        return opp.cost_evidence_label();
    }
    if is_spot_cross_strategy(opp) {
        return "双腿终态后确认".into();
    }
    if is_convergence_strategy(opp) {
        return if opp.recommended_hold_hours > 0.0 {
            format!(
                "历史目标 {:.1}h · 预测 {}",
                opp.recommended_hold_hours,
                signed_bps_percent(opp.net_bps_at_recommended_hold)
            )
        } else {
            "历史收敛只作统计证据".into()
        };
    }
    if is_projected_basis_strategy(opp) {
        return format!(
            "下一 Funding {} · 预测 {}",
            settlement_countdown_text(opp),
            signed_bps_percent(opp.net_bps_at_recommended_hold)
        );
    }
    if opp.breakeven_periods == 0 {
        return "查看动作旁阻断".into();
    }
    format!(
        "建议 {:.1}h · 净 {}",
        opp.recommended_hold_hours,
        signed_bps_percent(opp.net_bps_at_recommended_hold)
    )
}

pub(super) fn cost_text(opp: &FuturesOpportunity) -> String {
    if !opp.cost_verified {
        return format!("等待后端成本证据 · {}", opp.cost_evidence_label());
    }
    format!(
        "成本 {} · 磨损 {} · {}",
        unsigned_bps_percent(opp.cost_total_bps),
        unsigned_bps_percent(opp.cost_wear_bps),
        opp.cost_evidence_label()
    )
}

fn cost_breakeven_detail_text(opp: &FuturesOpportunity) -> String {
    if !opp.cost_verified {
        return cost_text(opp);
    }
    if is_spot_cross_strategy(opp) {
        return format!("{} · 即时费后 {}", cost_text(opp), opp.one_cycle_net_text());
    }
    if is_convergence_strategy(opp) {
        return format!(
            "{} · 等待价差收敛 · 预测 {}",
            cost_text(opp),
            signed_bps_percent(opp.net_bps_at_recommended_hold)
        );
    }
    if is_projected_basis_strategy(opp) {
        return format!(
            "{} · 退出基差待绑定 · 预测 {}",
            cost_text(opp),
            signed_bps_percent(opp.net_bps_at_recommended_hold)
        );
    }
    format!(
        "{} · 建议 {:.1}h · 净 {}",
        cost_text(opp),
        opp.recommended_hold_hours,
        signed_bps_percent(opp.net_bps_at_recommended_hold)
    )
}

fn is_spot_cross_strategy(opp: &FuturesOpportunity) -> bool {
    opp.strategy_kind == Some(StrategyKind::SpotCross)
}

fn is_convergence_strategy(opp: &FuturesOpportunity) -> bool {
    opp.strategy_kind == Some(StrategyKind::PerpPriceSpread)
}

fn is_projected_basis_strategy(opp: &FuturesOpportunity) -> bool {
    matches!(
        opp.strategy_kind,
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
    )
}

fn one_cycle_detail_text(opp: &FuturesOpportunity) -> String {
    let label = if is_convergence_strategy(opp) || is_projected_basis_strategy(opp) {
        "预测费后边际"
    } else {
        "费后净边际"
    };
    if !opp.cost_verified {
        return format!("{label}未验证");
    }
    format!(
        "{label} {}{}",
        signed_bps_percent(opp.one_cycle_net_bps),
        if opp.one_cycle_covers_cost {
            ""
        } else {
            " · 未覆盖成本"
        }
    )
}

pub(super) fn source_age_text(opp: &FuturesOpportunity) -> String {
    format!(
        "来源 {} · 更新 {}",
        source_text(&opp.data_source),
        age_text(opp.updated_at_ms)
    )
}

fn source_text(value: &str) -> &str {
    let value = value.trim();
    if value.is_empty() {
        "未知"
    } else {
        value
    }
}

fn age_text(updated_at_ms: i64) -> String {
    if updated_at_ms <= 0 {
        return "未知".into();
    }
    let age_ms = now_ms().saturating_sub(updated_at_ms).max(0);
    let age_minutes = age_ms / 60_000;
    if age_minutes < 1 {
        "刚刚".into()
    } else if age_minutes < 60 {
        format!("{age_minutes}m前")
    } else {
        format!("{}h前", age_minutes / 60)
    }
}

#[cfg(test)]
#[path = "format/tests.rs"]
mod tests;
