use super::*;

pub(super) struct MobileRiskSummaryInput {
    pub(super) row: Arc<PositionRow>,
    pub(super) quality_rows: Memo<Vec<AccountFieldQuality>>,
    pub(super) pair_display: String,
    pub(super) pair_support: &'static str,
    pub(super) hedge_href: String,
    pub(super) hedge_label: String,
    pub(super) has_pair: bool,
}

pub(super) fn mobile_risk_summary(input: MobileRiskSummaryInput) -> impl IntoView {
    let MobileRiskSummaryInput {
        row,
        quality_rows,
        pair_display,
        pair_support,
        hedge_href,
        hedge_label,
        has_pair,
    } = input;
    move || {
        let quality = quality_rows.get();
        let mark_quality = position_quality_by_field(&quality, "markPrice");
        let margin_quality = position_quality_by_field(&quality, "margin");
        let funding_quality = position_quality_by_field(&quality, "fundingRate8h");
        let mark = value_or_missing(mark_quality.as_ref(), price(row.mark_price));
        let pnl = pnl_display(row.unrealized_pnl_usd, mark_quality.as_ref());
        let margin = value_or_missing(margin_quality.as_ref(), money(row.margin_usd));
        let funding = funding_display(&row, funding_quality.as_ref());
        let liquidation_distance = liquidation_distance_label(&row);
        let liquidation_price = liquidation_price_label(row.liquidation_price);
        view! {
            <tr class="position-mobile-risk-row">
                <td colspan="8">
                    <div class="position-mobile-risk-grid">
                        {mobile_risk_item("PnL / 保证金", pnl.value, margin, pnl.class)}
                        {mobile_risk_item("强平距离 / 价格", liquidation_distance, liquidation_price, "")}
                        {mobile_risk_item("标记 / 入场", mark, price(row.entry_price), "")}
                        {mobile_pair_risk_item(
                            pair_display.clone(),
                            pair_support,
                            hedge_href.clone(),
                            hedge_label.clone(),
                            has_pair,
                        )}
                        {mobile_funding_item(funding)}
                    </div>
                </td>
            </tr>
        }
    }
}

fn mobile_funding_item(funding: FundingDisplay) -> impl IntoView {
    view! {
        <span class="position-mobile-risk-item position-mobile-funding-item">
            <small>"资金费"</small>
            <strong class=funding.class>{funding.window}</strong>
            <em>{funding.detail.unwrap_or_else(|| "等待费率数据依据".to_owned())}</em>
        </span>
    }
}

fn mobile_pair_risk_item(
    value: String,
    detail: &'static str,
    hedge_href: String,
    hedge_label: String,
    has_pair: bool,
) -> impl IntoView {
    view! {
        <span class="position-mobile-risk-item position-mobile-pair-item">
            <small>"配对状态"</small>
            <strong>{value}</strong>
            {if has_pair {
                view! { <em>{detail}</em> }.into_any()
            } else {
                view! { <a href=hedge_href aria-label=hedge_label>"筛选独立机会"</a> }.into_any()
            }}
        </span>
    }
}

fn mobile_risk_item(
    label: &'static str,
    value: String,
    detail: String,
    value_class: &'static str,
) -> impl IntoView {
    view! {
        <span class="position-mobile-risk-item">
            <small>{label}</small>
            <strong class=value_class>{value}</strong>
            <em>{detail}</em>
        </span>
    }
}
