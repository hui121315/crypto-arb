#![allow(clippy::panic)]

use super::*;
use shared_types::{AccountEquityScope, VenueCredentialValidationStatus};

const FINGERPRINT: &str = "hmac-sha256:0123456789abcdef01234567";

fn validation(
    status: VenueCredentialProbeStatus,
    scope: &str,
) -> VenueCredentialValidationSnapshot {
    VenueCredentialValidationSnapshot {
        venue: "test".to_owned(),
        credential_fingerprint: Some(FINGERPRINT.to_owned()),
        evidence: VenueCredentialValidationEvidence {
            status: VenueCredentialValidationStatus::ReadOnlyOk,
            checked_at_ms: 40,
            probes: vec![VenueCredentialProbe {
                kind: ACCOUNT_MODE_PROBE.to_owned(),
                status,
                scope: scope.to_owned(),
                source: "venue.account_mode".to_owned(),
                message: "probe result".to_owned(),
                checked_at_ms: 40,
                request_id: Some("req-scope-1".to_owned()),
            }],
            permission_evidence: Vec::new(),
        },
    }
}

#[test]
fn verified_scope_requires_probe_and_current_credential_fingerprint() {
    let validation = validation(VenueCredentialProbeStatus::Ok, "classic_futures");

    let row = binding_for_venue(
        "kucoin",
        Some(FINGERPRINT.to_owned()),
        Some(&validation),
        None,
        42,
    );

    assert_eq!(row.status, AccountBindingStatus::Verified);
    assert_eq!(row.account_scope.as_deref(), Some("classic_futures"));
    assert_eq!(row.checked_at_ms, Some(40));
    assert_eq!(row.freshness_ms, Some(2));
    assert!(row.problem.is_none());
}

#[test]
fn stale_scope_without_current_credential_binding_stays_unverified() {
    let validation = validation(VenueCredentialProbeStatus::Ok, "usds_m_futures");

    let row = binding_for_venue("binance", None, Some(&validation), None, 42);

    assert_eq!(row.status, AccountBindingStatus::Unverified);
    assert_eq!(row.account_scope.as_deref(), Some("usds_m_futures"));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::ACCOUNT_CREDENTIAL_BINDING_MISSING)
    );
}

#[test]
fn unknown_scope_keeps_request_context_in_typed_problem() {
    let validation = validation(VenueCredentialProbeStatus::Unknown, "not_probed");

    let row = binding_for_venue(
        "gate",
        Some(FINGERPRINT.to_owned()),
        Some(&validation),
        None,
        42,
    );

    assert_eq!(row.status, AccountBindingStatus::Unverified);
    assert_eq!(row.account_scope, None);
    let problem = row
        .problem
        .unwrap_or_else(|| panic!("scope problem missing"));
    assert_eq!(problem.code, codes::ACCOUNT_SCOPE_UNVERIFIED);
    assert_eq!(problem.request_id.as_deref(), Some("req-scope-1"));
}

#[test]
fn rotated_credential_rejects_stale_scope_probe() {
    let validation = validation(VenueCredentialProbeStatus::Ok, "classic_futures");

    let row = binding_for_venue(
        "kucoin",
        Some("hmac-sha256:aaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
        Some(&validation),
        None,
        42,
    );

    assert_eq!(row.status, AccountBindingStatus::Unverified);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::ACCOUNT_CREDENTIAL_GENERATION_MISMATCH)
    );
}

#[test]
fn persisted_hyperliquid_abstraction_message_recovers_unified_scope() {
    let mut validation = validation(
        VenueCredentialProbeStatus::Ok,
        "hyperliquid_account_role_abstraction",
    );
    validation.venue = "hyperliquid".to_owned();
    validation.evidence.probes.push(VenueCredentialProbe {
        kind: "account_abstraction".to_owned(),
        status: VenueCredentialProbeStatus::Ok,
        scope: "0x1111111111111111111111111111111111111111".to_owned(),
        source: "userAbstraction(account)+userDexAbstraction(account)".to_owned(),
        message: "account=0x1111111111111111111111111111111111111111 userAbstraction=unifiedAccount userDexAbstraction=disabled".to_owned(),
        checked_at_ms: 40,
        request_id: Some("req-scope-1".to_owned()),
    });

    let row = binding_for_venue(
        "hyperliquid:xyz",
        Some(FINGERPRINT.to_owned()),
        Some(&validation),
        None,
        42,
    );

    assert_eq!(row.status, AccountBindingStatus::Verified);
    assert_eq!(row.account_scope.as_deref(), Some("unifiedAccount"));
}

#[test]
fn fresh_runtime_account_summary_verifies_current_credential_binding() {
    let summary = account_summary("gate", "usdt_futures", 40);

    let row = binding_for_venue(
        "gate",
        Some(FINGERPRINT.to_owned()),
        None,
        Some(&summary),
        42,
    );

    assert_eq!(row.status, AccountBindingStatus::Verified);
    assert_eq!(row.account_scope.as_deref(), Some("usdt_futures"));
    assert_eq!(row.source, "official account summary");
    assert_eq!(row.checked_at_ms, Some(40));
    assert_eq!(row.freshness_ms, Some(2));
    assert!(row.problem.is_none());
}

#[test]
fn runtime_summary_without_current_credentials_stays_unverified() {
    let summary = account_summary("gate", "usdt_futures", 40);

    let row = binding_for_venue("gate", None, None, Some(&summary), 42);

    assert_eq!(row.status, AccountBindingStatus::Unverified);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::ACCOUNT_CREDENTIAL_BINDING_MISSING)
    );
}

#[test]
fn stale_runtime_summary_does_not_verify_account_scope() {
    let summary = account_summary("gate", "usdt_futures", 40);

    let row = binding_for_venue(
        "gate",
        Some(FINGERPRINT.to_owned()),
        None,
        Some(&summary),
        60_041,
    );

    assert_eq!(row.status, AccountBindingStatus::Unverified);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::ACCOUNT_SCOPE_UNVERIFIED)
    );
}

#[test]
fn unified_hyperliquid_spot_summary_binds_every_family_scope() {
    let summary = account_summary("hyperliquid:spot", "unifiedAccount", 40);

    let summaries = [summary];
    let rows = ["hyperliquid", "hyperliquid:xyz"]
        .into_iter()
        .map(|venue| {
            binding_for_venue(
                venue,
                Some(FINGERPRINT.to_owned()),
                None,
                runtime_summary_for_venue(venue, &summaries),
                42,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|row| row.status == AccountBindingStatus::Verified));
    assert!(rows
        .iter()
        .all(|row| row.account_scope.as_deref() == Some("unifiedAccount")));
}

fn account_summary(venue: &str, account_type: &str, observed_at_ms: i64) -> VenueAccountSummary {
    VenueAccountSummary {
        venue: venue.to_owned(),
        account_type: account_type.to_owned(),
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd: 100.0,
        total_available_balance_usd: 100.0,
        withdrawable_balance_usd: Some(100.0),
        total_initial_margin_usd: 0.0,
        total_maintenance_margin_usd: 0.0,
        account_im_rate: 0.0,
        account_mm_rate: 0.0,
        source: "official account summary".to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    }
}
