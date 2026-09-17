use super::*;

pub(super) fn static_capability_evidence_panel(status: Option<VenueCredentialStatus>) -> AnyView {
    let Some(status) = status else {
        return static_capability_missing_panel();
    };
    let summary = static_capability_summary(&status);
    let rows = static_capability_rows(&status)
        .into_iter()
        .map(static_capability_row)
        .collect_view();
    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"静态能力证据"</strong>
                    <em>{summary}</em>
                </div>
                <span class="status-pill pending">"静态声明"</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table runtime-health-table">
                    <thead>
                        <tr>
                            <th>"能力"</th>
                            <th>"声明"</th>
                            <th>"证据边界"</th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        </div>
    }
    .into_any()
}

fn static_capability_missing_panel() -> AnyView {
    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"静态能力证据"</strong>
                    <em>"请选择交易所；静态声明不等于保存期探针或运行态验证。"</em>
                </div>
                <span class="status-pill pending">"待选择"</span>
            </div>
        </div>
    }
    .into_any()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaticCapabilityRow {
    label: &'static str,
    declared: bool,
    detail: String,
}

fn static_capability_rows(status: &VenueCredentialStatus) -> Vec<StaticCapabilityRow> {
    vec![
        static_capability_row_data(
            "公开行情",
            status.public_market,
            "公开行情 adapter 声明；运行态 freshness/source 仍以行情诊断为准。",
        ),
        static_capability_row_data(
            "私有读取",
            status.private_read,
            "私有读取声明；仍需 balance/positions/open_orders/account_mode 保存期探针。",
        ),
        static_capability_row_data(
            "测试环境写侧",
            status.testnet_write,
            "测试网写侧声明；仍需 adapter/request-builder 与运行态订单终态证据。",
        ),
        static_capability_row_data(
            "实盘写侧",
            status.live_write,
            "静态写单声明；仍需 order_permission/private WS/order_finality 证据。",
        ),
        static_capability_row_data("说明", true, &status.note),
    ]
}

fn static_capability_row_data(
    label: &'static str,
    declared: bool,
    detail: &str,
) -> StaticCapabilityRow {
    StaticCapabilityRow {
        label,
        declared,
        detail: detail.to_owned(),
    }
}

fn static_capability_row(row: StaticCapabilityRow) -> impl IntoView {
    let declared = static_capability_declared_label(row.declared);
    let class = static_capability_declared_class(row.declared);
    view! {
        <tr>
            <td><strong>{row.label}</strong></td>
            <td><span class=class>{declared}</span></td>
            <td>{row.detail}</td>
        </tr>
    }
}

fn static_capability_summary(status: &VenueCredentialStatus) -> String {
    let write = if status.live_write {
        "实盘写侧静态声明"
    } else {
        "实盘未声明写侧"
    };
    format!(
        "{} · {} · {}",
        status.label, write, "保存期探针与运行态证据决定是否可提交"
    )
}

fn static_capability_declared_label(declared: bool) -> &'static str {
    if declared {
        "已声明"
    } else {
        "未声明"
    }
}

fn static_capability_declared_class(declared: bool) -> &'static str {
    if declared {
        "status-pill pending"
    } else {
        "status-pill"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_capability_summary_does_not_claim_live_readiness() {
        let status = credential_status(true);

        let summary = static_capability_summary(&status);

        assert!(summary.contains("实盘写侧静态声明"));
        assert!(summary.contains("运行态证据"));
        assert!(!summary.contains("可下单"));
        assert!(!summary.contains("实盘就绪"));
        assert!(!summary.contains("权限验证完整"));
    }

    #[test]
    fn static_capability_rows_preserve_false_declarations() {
        let status = credential_status(false);
        let rows = static_capability_rows(&status);
        let live = rows.iter().find(|row| row.label == "实盘写侧");

        assert_eq!(live.map(|row| row.declared), Some(false));
        assert_eq!(
            live.map(|row| static_capability_declared_label(row.declared)),
            Some("未声明")
        );
        assert!(live.is_some_and(|row| row.detail.contains("order_permission")));
    }

    fn credential_status(live_write: bool) -> VenueCredentialStatus {
        VenueCredentialStatus {
            venue: "okx".to_owned(),
            label: "OKX".to_owned(),
            fields: Vec::new(),
            public_market: true,
            private_read: true,
            testnet_write: false,
            live_write,
            note: "实盘受保护".to_owned(),
            missing_fields: Vec::new(),
            validation_evidence: None,
        }
    }
}
