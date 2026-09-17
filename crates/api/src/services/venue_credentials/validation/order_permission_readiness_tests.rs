use super::order_permission_matrix_tests::ORDER_PERMISSION_PROBE_CASES;
use super::*;
use shared_types::credential_matrix::{
    CredentialProbeLink, CredentialProbeMatrix, CredentialReadiness,
};
use shared_types::{VenueCredentialValidationEvidence, VenueCredentialValidationStatus};

fn ready_probe(link: CredentialProbeLink) -> VenueCredentialProbe {
    probe(
        link.kind(),
        VenueCredentialProbeStatus::Ok,
        "test",
        "test",
        "ok",
    )
}

#[test]
fn safe_order_permission_probes_do_not_grant_live_readiness() {
    for case in ORDER_PERMISSION_PROBE_CASES
        .iter()
        .filter(|case| case.expected_status == VenueCredentialProbeStatus::Unknown)
    {
        let order_probe = (case.build)(case.scope, case.source);
        assert_eq!(order_probe.status, VenueCredentialProbeStatus::Unknown);
        assert!(order_probe
            .message
            .contains("does not grant live-write readiness"));
        assert!(!order_probe.scope.contains(".order_write.live_place_cancel"));
        assert!(!order_probe.source.contains("live_order_proof_runtime"));

        let mut probes = CredentialProbeLink::live_required()
            .into_iter()
            .filter(|link| *link != CredentialProbeLink::OrderPermission)
            .map(ready_probe)
            .collect::<Vec<_>>();
        probes.push(order_probe);

        let evidence = VenueCredentialValidationEvidence {
            status: VenueCredentialValidationStatus::ReadOnlyOk,
            checked_at_ms: 1,
            probes,
            permission_evidence: Vec::new(),
        };
        assert!(!evidence.is_live_trading_ready());
        assert_eq!(evidence.readiness(), CredentialReadiness::Incomplete);
        assert!(evidence
            .blocking_links()
            .contains(&CredentialProbeLink::OrderPermission));
    }
}
