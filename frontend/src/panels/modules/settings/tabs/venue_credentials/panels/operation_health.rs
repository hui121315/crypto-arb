//! 交易运行态证据表：当前可用性仅由运行态链路判定。

use super::*;

pub(super) fn trading_runtime_evidence_panel(
    venue_id: &str,
    evidence: &TradingRuntimeEvidence,
) -> AnyView {
    let summary = trading_runtime_summary(venue_id, evidence);
    let status = trading_runtime_status_label(evidence);
    let status_class = trading_runtime_status_class(evidence);
    let rows = [
        trading_runtime_evidence_row(
            venue_id,
            "下单/撤单权限",
            VenueOperationKind::CredentialProbeOrderPermission,
            evidence.order_permission.as_ref(),
        ),
        trading_runtime_evidence_row(
            venue_id,
            "写单运行状态",
            VenueOperationKind::OrderWrite,
            evidence.order_write.as_ref(),
        ),
        trading_runtime_evidence_row(
            venue_id,
            "私有订单流",
            VenueOperationKind::PrivateWsOrderStream,
            evidence.private_order_stream.as_ref(),
        ),
        trading_runtime_evidence_row(
            venue_id,
            "订单最终结果",
            VenueOperationKind::OrderFinality,
            evidence.order_finality.as_ref(),
        ),
    ]
    .into_iter()
    .collect_view();

    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"交易运行数据依据"</strong>
                    <em>{summary}</em>
                </div>
                <span class=status_class>{status}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table runtime-health-table">
                    <thead>
                        <tr>
                            <th>"链路"</th>
                            <th>"状态"</th>
                            <th>"来源 / 新鲜度"</th>
                            <th>"request_id"</th>
                            <th>"数据依据"</th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        </div>
    }
    .into_any()
}

fn trading_runtime_evidence_row(
    venue_id: &str,
    label: &'static str,
    kind: VenueOperationKind,
    row: Option<&VenueOperationHealth>,
) -> AnyView {
    let operation = kind.as_str().unwrap_or(match kind {
        VenueOperationKind::CredentialProbeOrderPermission => "credential_probe:order_permission",
        _ => "unknown",
    });
    let title = row
        .map(runtime_evidence_detail)
        .unwrap_or_else(|| kind.product_explanation_zh().to_owned());
    let status = row
        .map(|row| operation_status_label(row.status))
        .unwrap_or("待数据依据");
    let status_class = row
        .map(|row| operation_status_class(row.status))
        .unwrap_or("status-pill pending");
    let source = row
        .map(|row| row.source.clone())
        .unwrap_or_else(|| expected_trading_runtime_source(kind).to_owned());
    let freshness = row
        .map(|row| freshness_label(row.freshness_ms))
        .unwrap_or_else(|| "freshness -".to_owned());
    let request_id = row
        .map(runtime_request_id_label)
        .unwrap_or_else(|| "request_id -".to_owned());
    let message = row
        .map(runtime_health_message)
        .unwrap_or_else(|| format!("{venue_id} 暂无{}运行状态记录", kind.label_zh()));
    let evidence = row
        .map(runtime_evidence_summary)
        .unwrap_or_else(|| expected_trading_runtime_evidence(kind).to_owned());
    let sample = row.map(runtime_sample).unwrap_or_else(|| "-".to_owned());

    view! {
        <tr>
            <td><strong>{label}</strong><em>{operation}</em></td>
            <td><span class=status_class>{status}</span></td>
            <td>{source}<em>{freshness} " · " {sample}</em></td>
            <td>{request_id}</td>
            <td title=title>{message}<em>{evidence}</em></td>
        </tr>
    }
    .into_any()
}

fn expected_trading_runtime_source(kind: VenueOperationKind) -> &'static str {
    match kind {
        VenueOperationKind::CredentialProbeOrderPermission => "credential_validation",
        VenueOperationKind::OrderWrite => "credential_status",
        VenueOperationKind::PrivateWsOrderStream => "private_ws_runtime",
        VenueOperationKind::OrderFinality => "run_finality",
        _ => "operation_health",
    }
}

fn expected_trading_runtime_evidence(kind: VenueOperationKind) -> &'static str {
    match kind {
        VenueOperationKind::CredentialProbeOrderPermission => {
            "保存凭证后需要权限探针数据依据；safe/noop 不授予 live_write"
        }
        VenueOperationKind::OrderWrite => "写单运行状态需 live place/cancel/finality 数据依据",
        VenueOperationKind::PrivateWsOrderStream => "私有订单事件流需要运行状态样本",
        VenueOperationKind::OrderFinality => "未决订单产生后由 REST/WS 最终结果回查写入",
        _ => "等待 operation-health 写入",
    }
}
