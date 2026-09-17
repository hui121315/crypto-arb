use leptos::prelude::*;

use super::super::data::PortfolioAccountAccess;

pub(in crate::panels::modules::positions) fn account_setup_prompt(
    access: Memo<PortfolioAccountAccess>,
    ledger_flow_active: Memo<bool>,
    open_settings: Callback<()>,
) -> impl IntoView {
    view! {
        {move || {
            let access = access.get();
            render_prompt(&access, ledger_flow_active.get(), open_settings)
        }}
    }
}

pub(in crate::panels::modules::positions) fn account_data_placeholder(
    title: &'static str,
    detail: &'static str,
) -> AnyView {
    view! {
        <div class="account-data-placeholder">
            <strong>{title}</strong>
            <span>{detail}</span>
        </div>
    }
    .into_any()
}

fn render_prompt(
    access: &PortfolioAccountAccess,
    ledger_flow_active: bool,
    open_settings: Callback<()>,
) -> AnyView {
    let has_configured_venue = access.has_configured_venue();
    let coverage_complete = has_configured_venue && !access.coverage_incomplete();
    let title = if coverage_complete {
        "账户读取覆盖完整"
    } else if has_configured_venue {
        "已接入部分交易所账户"
    } else {
        "尚未配置交易所 API 凭证"
    };
    let summary = account_setup_summary(access, ledger_flow_active);
    let venues = venue_list(&access.unconfigured_venues);
    let action_label = if coverage_complete {
        "管理 API 凭证"
    } else {
        "配置 API 凭证"
    };
    let class = if coverage_complete {
        "account-setup is-complete"
    } else {
        "account-setup"
    };
    view! {
        <section class=class aria-labelledby="account-setup-title">
            <div class="account-setup-copy">
                <span>"账户接入"</span>
                <strong id="account-setup-title">{title}</strong>
                <p>{summary}</p>
            </div>
            <div class="account-setup-action">
                <span>{if coverage_complete {
                    format!("已接入：{} 家", access.configured_venues.len())
                } else {
                    venues
                }}</span>
                <button type="button" class="primary-blue" on:click=move |_| open_settings.run(())>
                    {action_label}
                </button>
            </div>
            <div class="account-coverage-grid" aria-label="交易所账户覆盖">
                {coverage_rows(access)}
            </div>
            <details class="account-setup-requirements">
                <summary>"查看接入要求"</summary>
                <div>
                    <strong>"凭证字段"</strong>
                    <span>"API Key + Secret；交易所要求时再填 Passphrase、UID 或账户地址"</span>
                </div>
                <div>
                    <strong>"账户权限"</strong>
                    <span>"读取余额、持仓和订单；启用实盘执行时再授予交易权限"</span>
                </div>
                <div>
                    <strong>"安全边界"</strong>
                    <span>"不要授予提现权限；保存后运行凭证校验与账户范围探测"</span>
                </div>
            </details>
        </section>
    }
    .into_any()
}

fn coverage_rows(access: &PortfolioAccountAccess) -> AnyView {
    let rows = access
        .configured_venues
        .iter()
        .map(|venue| coverage_row(venue, true))
        .chain(
            access
                .unconfigured_venues
                .iter()
                .map(|venue| coverage_row(venue, false)),
        )
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return view! {
            <div class="account-coverage-empty">
                "尚未取得交易所账户覆盖清单"
            </div>
        }
        .into_any();
    }
    rows.into_iter().collect_view().into_any()
}

fn coverage_row(venue: &str, configured: bool) -> AnyView {
    let class = if configured {
        "account-coverage-row is-configured"
    } else {
        "account-coverage-row is-unconfigured"
    };
    let status = if configured { "已接入" } else { "未配置" };
    view! {
        <div class=class>
            <strong>{venue.to_ascii_uppercase()}</strong>
            <span>{status}</span>
        </div>
    }
    .into_any()
}

fn account_setup_summary(access: &PortfolioAccountAccess, ledger_flow_active: bool) -> String {
    if access.has_configured_venue() && !access.coverage_incomplete() {
        format!(
            "当前 NAV、持仓、余额与账户风险按已接入的 {} 家交易所计算。",
            access.configured_venues.len(),
        )
    } else if access.has_configured_venue() {
        format!(
            "当前 NAV、持仓和余额按已接入的 {} 家计算；其余 {} 家未配置，不计入当前账户视图。",
            access.configured_venues.len(),
            access.unconfigured_venues.len()
        )
    } else if ledger_flow_active {
        "执行账本模拟持仓与配对平仓流程可用；NAV、交易所私有持仓、余额与账户风险指标仍待配置。"
            .to_owned()
    } else {
        "当前没有可读取账户数据的交易所，NAV、持仓、余额和风险指标会保持未知。".to_owned()
    }
}

fn venue_list(venues: &[String]) -> String {
    const INLINE_LIMIT: usize = 5;
    if venues.is_empty() {
        return "未取得待配置交易所清单".to_owned();
    }
    let shown = venues
        .iter()
        .take(INLINE_LIMIT)
        .map(|venue| venue.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(" / ");
    if venues.len() > INLINE_LIMIT {
        format!("待配置：{shown} 等 {} 家", venues.len())
    } else {
        format!("待配置：{shown}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_list_is_bounded_but_keeps_total() {
        let venues = ["binance", "bitget", "bybit", "gate", "okx"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        let label = venue_list(&venues);

        assert!(label.contains("BINANCE / BITGET / BYBIT / GATE / OKX"));
        assert!(label.contains("等 6 家"));
        assert!(!label.contains("OKX"));
    }

    #[test]
    fn projected_positions_are_not_described_as_unavailable() {
        let access = PortfolioAccountAccess {
            configured_venues: Vec::new(),
            unconfigured_venues: vec!["binance".into(), "okx".into()],
        };

        let summary = account_setup_summary(&access, true);

        assert!(summary.contains("执行账本模拟持仓与配对平仓流程可用"));
        assert!(!summary.contains("持仓、余额和风险指标会保持未知"));
    }

    #[test]
    fn partial_account_coverage_describes_current_nav_scope() {
        let access = PortfolioAccountAccess {
            configured_venues: vec!["binance".into(), "bitget".into()],
            unconfigured_venues: vec!["okx".into()],
        };

        let summary = account_setup_summary(&access, false);

        assert!(summary.contains("按已接入的 2 家计算"));
        assert!(summary.contains("其余 1 家未配置"));
        assert!(!summary.contains("NAV 与相关账户指标保持未知"));
    }

    #[test]
    fn complete_account_coverage_omits_missing_venue_copy() {
        let access = PortfolioAccountAccess {
            configured_venues: vec!["binance".into(), "bitget".into()],
            unconfigured_venues: Vec::new(),
        };

        let summary = account_setup_summary(&access, false);

        assert!(summary.contains("已接入的 2 家交易所"));
        assert!(!summary.contains("其余 0 家未配置"));
    }
}
