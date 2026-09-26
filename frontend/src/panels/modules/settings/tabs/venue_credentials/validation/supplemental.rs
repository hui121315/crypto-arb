use super::*;
use shared_types::VenueCredentialProbeStatus;

pub(super) fn supplemental_validation_row(probe: &VenueCredentialProbe) -> impl IntoView {
    let label = supplemental_probe_label(&probe.kind);
    let status = probe_link_status(probe.status);
    let status_label = validation_link_status_label(status);
    let class = validation_link_status_class(status);
    let source = format!("{} · {}", probe.scope, probe.source);
    let title = validation_probe_title(Some(probe), status);
    let message = probe.message.clone();
    let kind_label = probe.kind.clone();
    let kind_attr = kind_label.clone();

    view! {
        <tr data-validation-probe=kind_attr>
            <td><strong>{label}</strong><em>{kind_label}</em></td>
            <td><span class=class>{status_label}</span></td>
            <td>{source}</td>
            <td title=title>{message}</td>
        </tr>
    }
}

fn supplemental_probe_label(kind: &str) -> &'static str {
    match kind {
        "account_signer_vault_relation" => "账户 / Signer / Vault",
        "account_abstraction" => "账户抽象",
        "perp_margin_read" => "Perp Margin",
        "spot_truth_read" => "Spot Truth",
        "local_format" => "本地格式",
        _ => "补充数据依据",
    }
}

fn probe_link_status(status: VenueCredentialProbeStatus) -> CredentialLinkStatus {
    match status {
        VenueCredentialProbeStatus::Ok => CredentialLinkStatus::Ok,
        VenueCredentialProbeStatus::Failed => CredentialLinkStatus::Failed,
        VenueCredentialProbeStatus::Unknown => CredentialLinkStatus::Unknown,
    }
}
