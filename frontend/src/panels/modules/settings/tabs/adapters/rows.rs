use crate::api::rest::TradingAdapterOption;
use crate::panels::shared::execution_environment_label;
use leptos::prelude::*;
use shared_types::ExecutionEnvironment;

pub(super) fn adapter_row(row: TradingAdapterOption, current: &str) -> impl IntoView {
    let is_current = row.id == current;
    let mode = mode_label(&row);
    let credential = credential_label(&row);
    let capability = capability_label(&row);
    let status = adapter_status(&row);
    let label = format!("{}环境", execution_environment_label(row.environment));
    let row_id = row.id;
    view! {
        <tr class=if is_current { "active" } else { "" }>
            <td>
                <strong>{label}</strong>
                <em>{row_id}</em>
            </td>
            <td>{mode}</td>
            <td>{credential}</td>
            <td>{capability}</td>
            <td>{status}</td>
        </tr>
    }
}

fn mode_label(row: &TradingAdapterOption) -> &'static str {
    execution_environment_label(row.environment)
}

pub(super) fn credential_label(row: &TradingAdapterOption) -> &'static str {
    if row.environment == ExecutionEnvironment::Paper {
        "无需凭证"
    } else if row.credentials_available {
        "字段组已补齐"
    } else {
        "待补齐"
    }
}

fn capability_label(row: &TradingAdapterOption) -> String {
    capability_text(&row.capabilities)
}

pub(super) fn capability_text(caps: &shared_types::TradingAdapterCapabilities) -> String {
    let product = if caps.spot && caps.perp {
        "现货/永续"
    } else if caps.perp {
        "永续"
    } else if caps.spot {
        "现货"
    } else {
        "模拟"
    };
    let mut order_types = Vec::with_capacity(4);
    if caps.limit_orders {
        order_types.push("限价");
    }
    if caps.market_orders {
        order_types.push("市价");
    }
    if caps.post_only {
        order_types.push("Post-only");
    }
    if caps.reduce_only {
        order_types.push("Reduce-only");
    }
    format!("{} / {}", product, order_types.join(" "))
}

pub(super) fn adapter_status(row: &TradingAdapterOption) -> String {
    if let Some(reason) = row.disabled_reason.as_ref() {
        return reason.clone();
    }
    if row.environment == ExecutionEnvironment::Live && row.credentials_available {
        return "可选路由；下单仍需票据级权限与运行状态数据依据".to_owned();
    }
    "-".to_owned()
}
