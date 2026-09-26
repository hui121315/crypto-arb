use super::capability_text;
use crate::api::rest::TradingVenueCapability;
use crate::panels::shared::execution_environment_label;
use leptos::prelude::*;
use shared_types::{OrderType, TimeInForce, VenueCapabilityMatrix, VenueOrderKind};

pub(super) fn venue_capabilities_table(venues: &[TradingVenueCapability]) -> AnyView {
    if venues.is_empty() {
        return view! {
            <div class="empty-cell">"Venue capability matrix 暂不可用"</div>
        }
        .into_any();
    }
    let configured_count = venues
        .iter()
        .filter(|venue| venue.credentials_available)
        .count();
    let problem_count = venues
        .iter()
        .filter(|venue| venue.problem.is_some())
        .count();
    let coverage = format!(
        "{} 场所 · {} 凭证字段已填 · {} 当前问题",
        venues.len(),
        configured_count,
        problem_count
    );
    let state = if problem_count == 0 {
        "静态参考".to_owned()
    } else {
        format!("{problem_count} 项需关注")
    };
    let state_class = if problem_count == 0 {
        "settings-capability-state"
    } else {
        "settings-capability-state is-warning"
    };
    view! {
        <details class="settings-capability-disclosure">
            <summary>
                <span class="settings-capability-copy">
                    <strong>"场所执行能力参考"</strong>
                    <small>{coverage}</small>
                </span>
                <span class=state_class>{state}</span>
            </summary>
            <div class="settings-capability-body">
                <p>"静态能力不等于当前可提交；实盘仍以票据级权限、运行状态和双腿交易检查为准。"</p>
                <div class="table-wrap">
                    <table
                        class="clean-table settings-table"
                        data-settings-table="venue-capabilities"
                        data-table-budget="bounded-small"
                    >
                        <thead>
                            <tr>
                                <th>"实盘 Venue"</th>
                                <th>"订单 / TIF"</th>
                                <th>"账户 / Client ID"</th>
                                <th>"最终结果数据依据"</th>
                                <th>"运行状态"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {venues
                                .iter()
                                .cloned()
                                .map(venue_capability_row)
                                .collect_view()}
                        </tbody>
                    </table>
                </div>
            </div>
        </details>
    }
    .into_any()
}

fn venue_capability_row(row: TradingVenueCapability) -> impl IntoView {
    let warning = row.problem.is_some();
    let status = row.problem.unwrap_or_else(|| {
        if row.credentials_available {
            "凭证字段已填写；仍需票据级运行 gate".into()
        } else {
            "静态契约可用；凭证字段待填写".into()
        }
    });
    let venue = row.venue;
    let mode = execution_environment_label(row.environment);
    let source = row.source;
    let credential = if row.credentials_available {
        "字段已填写"
    } else {
        "字段待填写"
    };
    let order_contract = if row.matrix.orders.is_empty() {
        capability_text(&row.capabilities)
    } else {
        order_contract_text(&row.matrix)
    };
    let account_contract = account_contract_text(&row.matrix);
    let finality_contract = finality_contract_text(&row.matrix);
    view! {
        <tr class=if warning { "warning" } else { "" }>
            <td>
                <strong>{venue}</strong>
                <em>{format!("{mode} · {credential}")}</em>
                <em>{source.clone()}</em>
            </td>
            <td title=source>{order_contract}</td>
            <td>{account_contract}</td>
            <td>{finality_contract}</td>
            <td>{status}</td>
        </tr>
    }
}

fn order_contract_text(matrix: &VenueCapabilityMatrix) -> String {
    matrix
        .orders
        .iter()
        .map(|order| {
            let order_type = match order.requested_order_type {
                OrderType::Limit => "限价",
                OrderType::Market => market_kind_label(order.venue_order_kind),
                OrderType::PostOnly => "Post-only",
            };
            let tif = order
                .time_in_force
                .iter()
                .map(|value| time_in_force_label(*value))
                .collect::<Vec<_>>()
                .join("/");
            if order.market_order_styles.is_empty() {
                format!("{order_type} {tif}")
            } else {
                let styles = order
                    .market_order_styles
                    .iter()
                    .map(|style| style.label())
                    .collect::<Vec<_>>()
                    .join("/");
                format!("{order_type} {tif} ({styles})")
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn market_kind_label(kind: VenueOrderKind) -> &'static str {
    match kind {
        VenueOrderKind::ProtectedIoc => "保护 IOC 市价",
        VenueOrderKind::PriceZeroIoc => "Price-zero IOC 市价",
        VenueOrderKind::MarketLike | VenueOrderKind::MarketLikeRequired => "Market-like",
        _ => "市价",
    }
}

fn time_in_force_label(value: TimeInForce) -> &'static str {
    match value {
        TimeInForce::Ioc => "IOC",
        TimeInForce::Fok => "FOK",
        TimeInForce::Gtc => "GTC",
        TimeInForce::Gtx => "GTX",
    }
}

fn account_contract_text(matrix: &VenueCapabilityMatrix) -> String {
    let margin = if matrix.account.order_margin_modes.is_empty() {
        "保证金模式按账户"
    } else {
        "逐单 Cross/Isolated"
    };
    let query = yes_no(matrix.client_order_id.supports_query_by_client_id);
    let cancel = yes_no(matrix.client_order_id.supports_cancel_by_client_id);
    format!(
        "{margin} · {} · Client ID {} · 查询{query}/撤单{cancel}",
        matrix.account.account_mode_scope, matrix.client_order_id.venue_field
    )
}

fn finality_contract_text(matrix: &VenueCapabilityMatrix) -> String {
    let mut paths = Vec::with_capacity(3);
    if matrix.finality.private_order_stream {
        paths.push("私有订单流");
    }
    if matrix.finality.private_fill_stream {
        paths.push("私有成交流");
    }
    if matrix.finality.order_status_read {
        paths.push("订单回查");
    }
    format!("受理确认 非最终结果 · {}", paths.join("/"))
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "支持"
    } else {
        "不支持"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_matrix_copy_keeps_non_native_market_and_finality_semantics_visible() {
        let gate = exchange_fixture(
            "gate",
            VenueOrderKind::PriceZeroIoc,
            OrderType::Market,
            shared_types::OrderPayloadPricePolicy::ZeroPrice,
        );
        let hyperliquid = exchange_fixture(
            "hyperliquid",
            VenueOrderKind::ProtectedIoc,
            OrderType::Limit,
            shared_types::OrderPayloadPricePolicy::ProtectionPrice,
        );

        assert!(order_contract_text(&gate).contains("Price-zero IOC 市价"));
        assert!(!order_contract_text(&gate).contains("GTX"));
        assert!(order_contract_text(&hyperliquid).contains("保护 IOC 市价"));
        assert!(finality_contract_text(&hyperliquid).starts_with("受理确认 非最终结果"));
    }

    fn exchange_fixture(
        venue: &str,
        kind: VenueOrderKind,
        effective_order_type: OrderType,
        price_policy: shared_types::OrderPayloadPricePolicy,
    ) -> VenueCapabilityMatrix {
        VenueCapabilityMatrix {
            venue: venue.into(),
            orders: vec![shared_types::VenueOrderCapability {
                requested_order_type: OrderType::Market,
                effective_order_type,
                time_in_force: vec![TimeInForce::Ioc],
                market_order_styles: Vec::new(),
                venue_order_kind: kind,
                payload_price_policy: price_policy,
            }],
            finality: shared_types::VenueFinalityCapability {
                ack_is_final: false,
                order_status_read: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
