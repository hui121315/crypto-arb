use super::*;
use shared_types::{
    VenueCredentialPermission, VenueCredentialPermissionEvidence, VenueCredentialPermissionStatus,
};

pub(super) fn permission_evidence_table(evidence: &VenueCredentialValidationEvidence) -> AnyView {
    let rows = permission_rows(evidence)
        .into_iter()
        .map(permission_row)
        .collect_view();
    view! {
        <div class="settings-summary-line">
            <strong>"订单权限检查结果"</strong>
            <span>"保存密钥时只能确认部分权限；下单前还会重新检查两边的交易条件。"</span>
        </div>
        <div class="table-wrap">
            <table class="clean-table settings-table" data-settings-table="credential-permissions">
                <thead>
                    <tr>
                        <th>"权限"</th>
                        <th>"是否已确认"</th>
                        <th>"状态"</th>
                        <th>"检查方式"</th>
                        <th>"检查范围"</th>
                        <th>"检查时间"</th>
                        <th>"请求编号"</th>
                        <th>"原始说明"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
    .into_any()
}

fn permission_rows(
    evidence: &VenueCredentialValidationEvidence,
) -> Vec<VenueCredentialPermissionEvidence> {
    if evidence.permission_evidence.is_empty() {
        return evidence
            .clone()
            .with_order_permission_scopes(&[])
            .permission_evidence;
    }
    evidence.permission_evidence.clone()
}

fn permission_row(evidence: VenueCredentialPermissionEvidence) -> impl IntoView {
    let permission = permission_label(evidence.permission);
    let validated = if evidence.status.is_validated() { "是" } else { "否" };
    let status = permission_status_label(evidence.status);
    let class = permission_status_class(evidence.status);
    let raw_status = evidence.status.as_str();
    let request_id = evidence.request_id.unwrap_or_else(|| "-".to_owned());
    let detail = evidence.error.unwrap_or(evidence.message);
    view! {
        <tr>
            <td><strong>{permission}</strong><em>{evidence.permission.as_str()}</em></td>
            <td>{validated}</td>
            <td><span class=class>{status}</span><em>{raw_status}</em></td>
            <td>{evidence.probe_kind}</td>
            <td>{evidence.permission_scope}</td>
            <td>{evidence.checked_at_ms}</td>
            <td>{request_id}</td>
            <td>{detail}</td>
        </tr>
    }
}

fn permission_label(permission: VenueCredentialPermission) -> &'static str {
    match permission {
        VenueCredentialPermission::OpenOrdersRead => "读取挂单",
        VenueCredentialPermission::PlaceOrder => "下单",
        VenueCredentialPermission::CancelOrder => "撤单",
    }
}

fn permission_status_label(status: VenueCredentialPermissionStatus) -> &'static str {
    match status {
        VenueCredentialPermissionStatus::Validated => "已验证",
        VenueCredentialPermissionStatus::Denied => "已拒绝",
        VenueCredentialPermissionStatus::Unproven => "尚未确认",
        VenueCredentialPermissionStatus::Missing => "数据待确认",
    }
}

fn permission_status_class(status: VenueCredentialPermissionStatus) -> &'static str {
    match status {
        VenueCredentialPermissionStatus::Validated => "status-pill ready",
        VenueCredentialPermissionStatus::Denied => "status-pill blocked",
        VenueCredentialPermissionStatus::Unproven | VenueCredentialPermissionStatus::Missing => {
            "status-pill pending"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{VenueCredentialProbeStatus, VenueCredentialValidationStatus};

    #[test]
    fn legacy_payload_projects_three_fail_closed_permission_rows() {
        let evidence = VenueCredentialValidationEvidence {
            status: VenueCredentialValidationStatus::ReadOnlyOk,
            checked_at_ms: 42,
            probes: vec![VenueCredentialProbe {
                kind: "open_orders_read".into(),
                status: VenueCredentialProbeStatus::Ok,
                scope: "private_read.open_orders".into(),
                source: "exchange_adapter.get_open_orders".into(),
                message: "open orders read succeeded".into(),
                checked_at_ms: 42,
                request_id: Some("req-open".into()),
            }],
            permission_evidence: Vec::new(),
        };

        let rows = permission_rows(&evidence);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].status, VenueCredentialPermissionStatus::Validated);
        assert_eq!(rows[1].status, VenueCredentialPermissionStatus::Missing);
        assert_eq!(rows[2].status, VenueCredentialPermissionStatus::Missing);
        assert_eq!(rows[0].request_id.as_deref(), Some("req-open"));
    }

    #[test]
    fn permission_labels_keep_place_and_cancel_distinct() {
        assert_eq!(
            permission_label(VenueCredentialPermission::PlaceOrder),
            "下单"
        );
        assert_eq!(
            permission_label(VenueCredentialPermission::CancelOrder),
            "撤单"
        );
        assert_eq!(
            permission_status_label(VenueCredentialPermissionStatus::Denied),
            "已拒绝"
        );
        assert_eq!(
            permission_status_class(VenueCredentialPermissionStatus::Unproven),
            "status-pill pending"
        );
    }
}
