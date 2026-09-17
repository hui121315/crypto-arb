//! 保存期凭证验证证据面板：字段配置、私有读探针和下单权限分层展示。

use super::*;
use shared_types::credential_matrix::{
    CredentialLinkStatus, CredentialProbeLink, CredentialProbeMatrix, CredentialReadiness,
};
use shared_types::{VenueCredentialProbe, VenueCredentialValidationEvidence};

mod permissions;
mod supplemental;

use permissions::permission_evidence_table;
use supplemental::supplemental_validation_row;

pub(super) fn validation_evidence_panel(status: Option<VenueCredentialStatus>) -> AnyView {
    let Some(status) = status else {
        return validation_missing_panel("未选择交易所");
    };
    let label = status.label;
    let Some(evidence) = status.validation_evidence else {
        return validation_missing_panel(format!("{label} 暂无保存期校验证据"));
    };
    let summary = validation_evidence_summary(&evidence);
    let status_class = validation_readiness_class(evidence.readiness());
    let status_label = validation_readiness_label(evidence.readiness());
    let required = CredentialProbeLink::live_required();
    let rows = required
        .into_iter()
        .map(|link| validation_link_row(link, &evidence).into_any())
        .chain(
            evidence
                .probes
                .iter()
                .filter(|probe| {
                    !CredentialProbeLink::live_required()
                        .into_iter()
                        .any(|link| link.kind() == probe.kind)
                })
                .map(|probe| supplemental_validation_row(probe).into_any()),
        )
        .collect_view();
    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"保存期验证"</strong>
                    <em>{summary}</em>
                </div>
                <span class=status_class>{status_label}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table runtime-health-table">
                    <thead>
                        <tr>
                            <th>"链路"</th>
                            <th>"状态"</th>
                            <th>"来源"</th>
                            <th>"证据"</th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
            {permission_evidence_table(&evidence)}
        </div>
    }
    .into_any()
}

fn validation_missing_panel(message: impl Into<String>) -> AnyView {
    let message = message.into();
    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"保存期验证"</strong>
                    <em>{message} "；字段已填写不代表私有读、下单权限或账户模式已验证。"</em>
                </div>
                <span class="status-pill pending">"未验证"</span>
            </div>
        </div>
    }
    .into_any()
}

fn validation_link_row(
    link: CredentialProbeLink,
    evidence: &VenueCredentialValidationEvidence,
) -> impl IntoView {
    let link_status = evidence.link_status(link);
    let probe = validation_probe(evidence, link);
    let label = credential_link_label(link);
    let status = validation_link_status_label(link_status);
    let class = validation_link_status_class(link_status);
    let source = validation_probe_source(probe);
    let message = validation_probe_message(probe, link_status);
    let title = validation_probe_title(probe, link_status);

    view! {
        <tr>
            <td><strong>{label}</strong><em>{link.kind()}</em></td>
            <td><span class=class>{status}</span></td>
            <td>{source}</td>
            <td title=title>{message}</td>
        </tr>
    }
}

fn validation_probe(
    evidence: &VenueCredentialValidationEvidence,
    link: CredentialProbeLink,
) -> Option<&VenueCredentialProbe> {
    evidence
        .probes
        .iter()
        .find(|probe| probe.kind == link.kind())
}

pub(super) fn validation_evidence_summary(evidence: &VenueCredentialValidationEvidence) -> String {
    format!(
        "{} · {} · checked {}",
        validation_status_label(evidence.status),
        validation_readiness_label(evidence.readiness()),
        evidence.checked_at_ms
    )
}

fn validation_readiness_label(readiness: CredentialReadiness) -> &'static str {
    match readiness {
        CredentialReadiness::LiveReady => "权限验证完整",
        CredentialReadiness::Blocked => "权限阻断",
        CredentialReadiness::Incomplete => "权限未完整",
    }
}

fn validation_readiness_class(readiness: CredentialReadiness) -> &'static str {
    match readiness {
        CredentialReadiness::LiveReady => "status-pill ready",
        CredentialReadiness::Blocked => "status-pill blocked",
        CredentialReadiness::Incomplete => "status-pill pending",
    }
}

fn validation_link_status_label(status: CredentialLinkStatus) -> &'static str {
    match status {
        CredentialLinkStatus::Ok => "通过",
        CredentialLinkStatus::Failed => "拒绝",
        CredentialLinkStatus::Unknown => "未证明",
        CredentialLinkStatus::Missing => "缺证据",
    }
}

fn validation_link_status_class(status: CredentialLinkStatus) -> &'static str {
    match status {
        CredentialLinkStatus::Ok => "status-pill ready",
        CredentialLinkStatus::Failed => "status-pill blocked",
        CredentialLinkStatus::Unknown | CredentialLinkStatus::Missing => "status-pill pending",
    }
}

fn validation_probe_source(probe: Option<&VenueCredentialProbe>) -> String {
    probe
        .map(|probe| format!("{} · {}", probe.scope, probe.source))
        .unwrap_or_else(|| "required probe missing".to_owned())
}

fn validation_probe_message(
    probe: Option<&VenueCredentialProbe>,
    status: CredentialLinkStatus,
) -> String {
    probe
        .map(|probe| probe.message.clone())
        .unwrap_or_else(|| format!("{} 保存期探针缺失", validation_link_status_label(status)))
}

fn validation_probe_title(
    probe: Option<&VenueCredentialProbe>,
    status: CredentialLinkStatus,
) -> String {
    probe.map_or_else(
        || validation_link_status_label(status).to_owned(),
        |probe| {
            format!(
                "{} · source {} · checked {}",
                probe.scope, probe.source, probe.checked_at_ms
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{VenueCredentialProbeStatus, VenueCredentialValidationStatus};

    #[test]
    fn validation_summary_is_fail_closed_for_missing_required_links() {
        let evidence = evidence(vec![probe(
            CredentialProbeLink::BalanceRead,
            VenueCredentialProbeStatus::Ok,
        )]);

        let summary = validation_evidence_summary(&evidence);

        assert!(summary.contains("只读验证"));
        assert!(summary.contains("权限未完整"));
        assert!(!summary.contains("权限验证完整"));
    }

    #[test]
    fn validation_probe_message_names_missing_link_status() {
        assert_eq!(
            validation_probe_message(None, CredentialLinkStatus::Missing),
            "缺证据 保存期探针缺失"
        );
    }

    #[test]
    fn validation_probe_source_preserves_scope_and_source() {
        let probe = probe(
            CredentialProbeLink::OpenOrdersRead,
            VenueCredentialProbeStatus::Unknown,
        );

        assert_eq!(
            validation_probe_source(Some(&probe)),
            "live · credential_validation"
        );
    }

    #[test]
    fn validation_probe_message_preserves_kucoin_classic_futures_explanation() {
        let probe = VenueCredentialProbe {
            kind: CredentialProbeLink::AccountModeRead.kind().to_owned(),
            status: VenueCredentialProbeStatus::Ok,
            scope: "classic_futures".to_owned(),
            source: "kucoin.GET /api/v2/position/getPositionMode".to_owned(),
            message: "read-only account mode probe succeeded: KuCoin Classic Futures positionMode=hedge; balance_read uses Classic Futures account-overview availableMargin as futures buying power; UTA account mode is separate".to_owned(),
            checked_at_ms: 42,
            request_id: None,
        };

        let message = validation_probe_message(Some(&probe), CredentialLinkStatus::Ok);

        assert!(message.contains("Classic Futures"));
        assert!(message.contains("availableMargin"));
        assert!(message.contains("UTA account mode is separate"));
    }

    fn evidence(probes: Vec<VenueCredentialProbe>) -> VenueCredentialValidationEvidence {
        VenueCredentialValidationEvidence {
            status: VenueCredentialValidationStatus::ReadOnlyOk,
            checked_at_ms: 42,
            probes,
            permission_evidence: Vec::new(),
        }
    }

    fn probe(
        link: CredentialProbeLink,
        status: VenueCredentialProbeStatus,
    ) -> VenueCredentialProbe {
        VenueCredentialProbe {
            kind: link.kind().to_owned(),
            status,
            scope: "live".to_owned(),
            source: "credential_validation".to_owned(),
            message: format!("{} {status:?}", link.kind()),
            checked_at_ms: 42,
            request_id: None,
        }
    }
}
