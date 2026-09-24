use leptos::prelude::*;
use shared_types::{
    AccountFieldQuality, AccountFieldQualityStatus, HardLimitsUsage, PositionRow, RiskSnapshot,
    VAR_99_MIN_SAMPLES,
};

use super::super::data::PortfolioAccountAccess;
use super::account_setup::account_data_placeholder;
use super::format::{money, pct, signed_money};
use super::section_state::SectionData;

#[path = "risk_panel/field_quality.rs"]
mod field_quality;
use field_quality::{position_field_quality_rows, render_position_field_quality};

#[path = "risk_panel/summary.rs"]
mod summary;
pub(in crate::panels::modules::positions) use summary::risk_summary_panel;

pub(in crate::panels::modules::positions) fn risk_panel(
    snapshot: Memo<SectionData<Option<RiskSnapshot>>>,
    field_quality: Memo<Vec<AccountFieldQuality>>,
    nav_evidence_status: Memo<Option<AccountFieldQualityStatus>>,
    account_access: Memo<PortfolioAccountAccess>,
    position_values_known: Memo<bool>,
    positions: Memo<SectionData<Vec<PositionRow>>>,
) -> impl IntoView {
    view! {
        <div class="risk-panel">
            {move || {
                if account_access.get().account_data_unavailable() {
                    return account_data_placeholder(
                        "风险指标等待账户接入",
                        "配置余额与持仓读取权限后计算 VaR、保证金占用和集中度。",
                    );
                }
                let section = snapshot.get();
                let position_quality = position_field_quality_rows(field_quality.get());
                match section.value {
                    Some(snapshot) => render_snapshot(
                        snapshot,
                        section.status.stale_note("风险刷新失败，显示上次快照"),
                        &position_quality,
                        nav_evidence_status.get(),
                        position_values_known.get(),
                        summary::funding_evidence_missing_count(&positions.get().value),
                    ),
                    None => empty_risk(&section, &position_quality),
                }
            }}
        </div>
    }
}

fn render_snapshot(
    snapshot: RiskSnapshot,
    stale_note: Option<String>,
    position_quality: &[AccountFieldQuality],
    nav_evidence_status: Option<AccountFieldQualityStatus>,
    positions_known: bool,
    missing_funding: usize,
) -> AnyView {
    let limits_section = render_limits(&snapshot, nav_evidence_status);
    view! {
        {stale_note.map(status_note)}
        {render_position_field_quality(position_quality)}
        {limits_section}
        <RiskSection title="Funding 结算窗口">
            {if positions_known && missing_funding == 0 { snapshot.funding_clustering.into_iter().map(|cluster| {
                view! {
                    <div class="risk-row">
                        <span>{format!("未来 {}m 内", cluster.settles_in_minutes)}</span>
                        <strong>{format!("{} 仓位", cluster.position_count)}</strong>
                        <em class="negative">{signed_money(-cluster.total_outflow_usd)}</em>
                    </div>
                }
            }).collect_view().into_any() } else {
                view! { <p class="risk-empty">{if positions_known {
                    format!("{missing_funding} 个仓位待补结算证据，暂不汇总 Funding")
                } else { "持仓数据待确认，不能判断结算窗口".to_owned() }}</p> }.into_any()
            }}
        </RiskSection>
        <RiskSection title="Delta 集中度">
            {if positions_known { snapshot.delta_concentration.into_iter().map(|delta| {
                let tone = if delta.net_notional_usd >= 0.0 { "positive" } else { "negative" };
                view! {
                    <div class="risk-row">
                        <span>{delta.asset}</span>
                        <strong class="num">{format!("{:+.4}", delta.net_qty)}</strong>
                        <em class=tone>{signed_money(delta.net_notional_usd)}</em>
                    </div>
                }
            }).collect_view().into_any() } else {
                view! { <p class="risk-empty">"持仓数据待确认，不能按零敞口计算"</p> }.into_any()
            }}
        </RiskSection>
        <RiskSection title="保证金占用">
            {snapshot.margin_utilization.into_iter().map(|venue| {
                let utilization = venue.utilization_pct;
                let initial_margin = venue.initial_margin_usd
                    .map(money)
                    .unwrap_or_else(|| "未知".to_owned());
                view! {
                    <div class="risk-meter-row">
                        <div>
                            <span>{venue.venue}</span>
                            <strong>{utilization.map(pct).unwrap_or_else(|| "未知".to_owned())}</strong>
                        </div>
                        <Meter pct=utilization.unwrap_or(0.0)/>
                        <em>"初始 " {initial_margin} " / 权益 " {money(venue.equity_usd)}</em>
                        <em>"维持 " {money(venue.maintenance_margin_usd)}</em>
                        {venue.estimated.then(|| view! {
                            <em class="muted">"账户权益缺失 · 仓位口径估算"</em>
                        })}
                    </div>
                }
            }).collect_view()}
        </RiskSection>
    }.into_any()
}

fn empty_risk(
    section: &SectionData<Option<RiskSnapshot>>,
    position_quality: &[AccountFieldQuality],
) -> AnyView {
    let text = section
        .status
        .empty_text("暂无风险快照", "读取风险快照中", "风险快照读取失败");
    view! {
        {render_position_field_quality(position_quality)}
        <div class="risk-empty">{text}</div>
    }
    .into_any()
}

fn status_note(text: String) -> impl IntoView {
    view! { <div class="risk-empty stale-note">{text}</div> }
}

fn render_limits(
    snapshot: &RiskSnapshot,
    nav_evidence_status: Option<AccountFieldQualityStatus>,
) -> AnyView {
    let (value, sub, tone) = var_display(
        snapshot.var_99_1d_usd,
        snapshot.var_pct_of_nav,
        snapshot.var_sample_size,
        nav_evidence_status,
    );
    let limits = snapshot.hard_limits.clone();
    view! {
        <RiskSection title="风险限额">
            <div class="risk-row">
                <span>"VaR-99 (1日)"</span>
                <strong class=tone>{value}</strong>
                <em>{sub}</em>
            </div>
            <LimitRows limits/>
        </RiskSection>
    }
    .into_any()
}

/// 样本不足（< `VAR_99_MIN_SAMPLES`）时不展示可信 VaR，而是提示「样本不足」，
/// 避免把塔缩到最差单点的分位数当作真实风险数。
fn var_display(
    var_usd: f64,
    var_pct: f64,
    sample: usize,
    nav_evidence_status: Option<AccountFieldQualityStatus>,
) -> (String, String, &'static str) {
    if sample >= VAR_99_MIN_SAMPLES {
        if nav_evidence_status != Some(AccountFieldQualityStatus::Actual) {
            return (money(var_usd), "权益占比缺证据".to_owned(), "muted");
        }
        let tone = if var_pct > 5.0 { "negative" } else { "num" };
        (money(var_usd), format!("占权益 {}", pct(var_pct)), tone)
    } else {
        (
            "样本不足".to_owned(),
            format!("{sample}/{VAR_99_MIN_SAMPLES} 样本"),
            "muted",
        )
    }
}

#[component]
fn LimitRows(limits: HardLimitsUsage) -> impl IntoView {
    let orders_pct = usage_pct(
        limits.open_orders_used as f64,
        limits.open_orders_max as f64,
    );

    let kill_switch_class = if limits.kill_switch_active {
        "kill-switch active"
    } else {
        "kill-switch"
    };
    let kill_switch_text = if limits.kill_switch_active {
        "Kill Switch 开启"
    } else {
        "Kill Switch 关闭"
    };

    view! {
        <div class="risk-limits">
            <LimitRow label="挂单数" value=format!("{}/{}", limits.open_orders_used, limits.open_orders_max) pct=orders_pct/>
            <StaticLimitRow label="最大单标敞口" value=money(limits.max_symbol_notional_usd)/>
            <StaticLimitRow label="单笔下单上限" value=money(limits.max_order_notional_usd)/>
            <div class=kill_switch_class>{kill_switch_text}</div>
        </div>
    }
}

#[component]
fn LimitRow(#[prop(into)] label: String, #[prop(into)] value: String, pct: f64) -> impl IntoView {
    view! {
        <div class="risk-meter-row">
            <div>
                <span>{label}</span>
                <strong>{value}</strong>
            </div>
            <Meter pct=pct/>
        </div>
    }
}

#[component]
fn StaticLimitRow(#[prop(into)] label: String, #[prop(into)] value: String) -> impl IntoView {
    view! {
        <div class="risk-meter-row static-limit">
            <div>
                <span>{label}</span>
                <strong>{value}</strong>
            </div>
        </div>
    }
}

#[component]
fn RiskSection(#[prop(into)] title: String, children: Children) -> impl IntoView {
    view! {
        <section class="risk-section">
            <h3>{title}</h3>
            <div>{children()}</div>
        </section>
    }
}

#[component]
fn Meter(pct: f64) -> impl IntoView {
    view! {
        <div class="meter"><i style=format!("width:{:.1}%;", clamp_pct(pct))></i></div>
    }
}

fn usage_pct(used: f64, max: f64) -> f64 {
    if max <= 0.0 {
        0.0
    } else {
        used / max * 100.0
    }
}

fn clamp_pct(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_display_flags_insufficient_sample() {
        let (value, sub, tone) =
            var_display(300.0, 12.0, 3, Some(AccountFieldQualityStatus::Actual));
        assert_eq!(value, "样本不足");
        assert_eq!(sub, format!("3/{VAR_99_MIN_SAMPLES} 样本"));
        assert_eq!(tone, "muted");
    }

    #[test]
    fn var_display_shows_value_when_sample_sufficient() {
        let (value, _sub, _tone) = var_display(
            300.0,
            1.2,
            VAR_99_MIN_SAMPLES,
            Some(AccountFieldQualityStatus::Actual),
        );
        assert_ne!(value, "样本不足");
    }

    #[test]
    fn var_display_hides_nav_ratio_without_actual_equity() {
        let (value, sub, tone) = var_display(
            300.0,
            12.0,
            VAR_99_MIN_SAMPLES,
            Some(AccountFieldQualityStatus::Missing),
        );

        assert_ne!(value, "样本不足");
        assert_eq!(sub, "权益占比缺证据");
        assert_eq!(tone, "muted");
    }
}
