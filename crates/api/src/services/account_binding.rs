use std::collections::{BTreeMap, BTreeSet};

use axum::http::StatusCode;
use shared_types::{
    problem::codes, AccountBindingEvidence, AccountBindingStatus, ApiProblem, VenueAccountSummary,
    VenueCredentialProbe, VenueCredentialProbeStatus, VenueCredentialValidationEvidence,
};

use crate::services::{
    trading_credentials,
    venue_credentials::{self, VenueCredentialValidationSnapshot},
};

const BINDING_SOURCE: &str = "account_binding_runtime";
const ACCOUNT_MODE_PROBE: &str = "account_mode_read";
const RUNTIME_SUMMARY_MAX_AGE_MS: i64 = 60_000;

pub(crate) fn evidence_for_venues(
    venues: impl IntoIterator<Item = String>,
    observed_at_ms: i64,
) -> Vec<AccountBindingEvidence> {
    evidence_for_venues_with_summaries(venues, &[], observed_at_ms)
}

pub(crate) fn evidence_for_venues_with_summaries(
    venues: impl IntoIterator<Item = String>,
    summaries: &[VenueAccountSummary],
    observed_at_ms: i64,
) -> Vec<AccountBindingEvidence> {
    let validations = venue_credentials::validation_evidence_snapshot()
        .into_iter()
        .map(|row| (shared_types::normalized_venue_name(&row.venue), row))
        .collect::<BTreeMap<_, _>>();
    let venues = venues
        .into_iter()
        .filter(|venue| !venue.trim().is_empty())
        .map(|venue| shared_types::normalized_venue_name(&venue))
        .collect::<BTreeSet<_>>();

    venues
        .into_iter()
        .map(|venue| {
            let family = shared_types::normalized_venue_name(shared_types::venue_family(&venue));
            binding_for_venue(
                &venue,
                trading_credentials::credential_fingerprint(&family),
                validations.get(&family),
                runtime_summary_for_venue(&venue, summaries),
                observed_at_ms,
            )
        })
        .collect()
}

fn binding_for_venue(
    venue: &str,
    credential_fingerprint: Option<String>,
    validation: Option<&VenueCredentialValidationSnapshot>,
    runtime_summary: Option<&VenueAccountSummary>,
    observed_at_ms: i64,
) -> AccountBindingEvidence {
    if let Some(binding) = runtime_summary_binding(
        venue,
        credential_fingerprint.as_deref(),
        runtime_summary,
        observed_at_ms,
    ) {
        return binding;
    }
    let evidence = validation.map(|row| &row.evidence);
    let probe = evidence.and_then(preferred_account_mode_probe);
    let account_scope = evidence
        .and_then(|evidence| hyperliquid_account_scope(venue, evidence))
        .or_else(|| probe.and_then(probe_scope));
    let source = probe
        .map(|probe| probe.source.clone())
        .unwrap_or_else(|| BINDING_SOURCE.to_owned());
    let checked_at_ms = probe
        .map(|probe| probe.checked_at_ms)
        .or_else(|| evidence.map(|evidence| evidence.checked_at_ms));
    let generation_matches = credential_fingerprint.as_deref()
        == validation.and_then(|row| row.credential_fingerprint.as_deref());
    let (status, problem) = binding_status_problem(
        venue,
        credential_fingerprint.is_some(),
        validation.is_some(),
        generation_matches,
        probe,
        observed_at_ms,
    );
    AccountBindingEvidence {
        venue: venue.to_owned(),
        account_scope,
        status,
        source,
        checked_at_ms,
        freshness_ms: checked_at_ms
            .map(|checked_at_ms| observed_at_ms.saturating_sub(checked_at_ms).max(0) as u64),
        credential_fingerprint,
        problem,
    }
}

fn runtime_summary_for_venue<'a>(
    venue: &str,
    summaries: &'a [VenueAccountSummary],
) -> Option<&'a VenueAccountSummary> {
    let venue = shared_types::normalized_venue_name(venue);
    let family = shared_types::normalized_venue_name(shared_types::venue_family(&venue));
    if family == "hyperliquid" {
        if let Some(summary) = summaries.iter().find(|summary| {
            shared_types::normalized_venue_name(&summary.venue) == "hyperliquid:spot"
                && is_consolidated_hyperliquid_scope(&summary.account_type)
        }) {
            return Some(summary);
        }
    }
    summaries
        .iter()
        .find(|summary| shared_types::normalized_venue_name(&summary.venue) == venue)
}

fn runtime_summary_binding(
    venue: &str,
    credential_fingerprint: Option<&str>,
    summary: Option<&VenueAccountSummary>,
    observed_at_ms: i64,
) -> Option<AccountBindingEvidence> {
    let summary = summary?;
    let credential_fingerprint = credential_fingerprint?;
    let account_scope = runtime_summary_scope(summary)?;
    let freshness_ms = observed_at_ms.saturating_sub(summary.observed_at_ms).max(0);
    if summary.problem.is_some()
        || summary.observed_at_ms <= 0
        || freshness_ms > RUNTIME_SUMMARY_MAX_AGE_MS
    {
        return None;
    }
    Some(AccountBindingEvidence {
        venue: venue.to_owned(),
        account_scope: Some(account_scope),
        status: AccountBindingStatus::Verified,
        source: summary.source.clone(),
        checked_at_ms: Some(summary.observed_at_ms),
        freshness_ms: Some(freshness_ms as u64),
        credential_fingerprint: Some(credential_fingerprint.to_owned()),
        problem: None,
    })
}

fn runtime_summary_scope(summary: &VenueAccountSummary) -> Option<String> {
    let scope = summary.account_type.trim();
    (!scope.is_empty() && !scope.eq_ignore_ascii_case("unknown")).then(|| scope.to_owned())
}

fn is_consolidated_hyperliquid_scope(scope: &str) -> bool {
    matches!(
        scope.trim().to_ascii_lowercase().as_str(),
        "unifiedaccount" | "portfoliomargin"
    )
}

fn preferred_account_mode_probe(
    validation: &VenueCredentialValidationEvidence,
) -> Option<&VenueCredentialProbe> {
    validation
        .probes
        .iter()
        .filter(|probe| probe.kind == ACCOUNT_MODE_PROBE)
        .max_by_key(|probe| probe_status_rank(probe.status))
}

fn hyperliquid_account_scope(
    venue: &str,
    validation: &VenueCredentialValidationEvidence,
) -> Option<String> {
    if shared_types::normalized_venue_name(shared_types::venue_family(venue)) != "hyperliquid" {
        return None;
    }
    let probe = validation.probes.iter().find(|probe| {
        probe.kind == "account_abstraction" && probe.status == VenueCredentialProbeStatus::Ok
    })?;
    let scope = probe.scope.trim();
    if is_hyperliquid_abstraction_scope(scope) {
        return Some(scope.to_owned());
    }
    probe
        .message
        .split_whitespace()
        .find_map(|token| token.strip_prefix("userAbstraction="))
        .filter(|scope| is_hyperliquid_abstraction_scope(scope))
        .map(str::to_owned)
}

fn is_hyperliquid_abstraction_scope(scope: &str) -> bool {
    matches!(
        scope,
        "unifiedAccount" | "portfolioMargin" | "default" | "disabled" | "dexAbstraction"
    )
}

fn probe_status_rank(status: VenueCredentialProbeStatus) -> u8 {
    match status {
        VenueCredentialProbeStatus::Ok => 3,
        VenueCredentialProbeStatus::Failed => 2,
        VenueCredentialProbeStatus::Unknown => 1,
    }
}

fn probe_scope(probe: &VenueCredentialProbe) -> Option<String> {
    let scope = probe.scope.trim();
    (!scope.is_empty() && scope != "not_probed" && scope != "unknown").then(|| scope.to_owned())
}

fn binding_status_problem(
    venue: &str,
    credential_bound: bool,
    validation_present: bool,
    generation_matches: bool,
    probe: Option<&VenueCredentialProbe>,
    observed_at_ms: i64,
) -> (AccountBindingStatus, Option<ApiProblem>) {
    if !credential_bound {
        return (
            AccountBindingStatus::Unverified,
            Some(binding_problem(
                codes::ACCOUNT_CREDENTIAL_BINDING_MISSING,
                "account data is not bound to a complete current credential generation",
                venue,
                probe,
                observed_at_ms,
            )),
        );
    }
    if validation_present && !generation_matches {
        return (
            AccountBindingStatus::Unverified,
            Some(binding_problem(
                codes::ACCOUNT_CREDENTIAL_GENERATION_MISMATCH,
                "account scope evidence belongs to a different credential generation",
                venue,
                probe,
                observed_at_ms,
            )),
        );
    }
    match probe.map(|probe| probe.status) {
        Some(VenueCredentialProbeStatus::Ok) => (AccountBindingStatus::Verified, None),
        Some(VenueCredentialProbeStatus::Failed) => (
            AccountBindingStatus::Failed,
            Some(binding_problem(
                codes::ACCOUNT_SCOPE_PROBE_FAILED,
                "account scope probe was rejected",
                venue,
                probe,
                observed_at_ms,
            )),
        ),
        Some(VenueCredentialProbeStatus::Unknown) | None => (
            AccountBindingStatus::Unverified,
            Some(binding_problem(
                codes::ACCOUNT_SCOPE_UNVERIFIED,
                "account scope has not been verified for the current credential generation",
                venue,
                probe,
                observed_at_ms,
            )),
        ),
    }
}

fn binding_problem(
    code: &str,
    message: &str,
    venue: &str,
    probe: Option<&VenueCredentialProbe>,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message)
        .with_status(StatusCode::OK.as_u16())
        .with_request_id(probe.and_then(|probe| probe.request_id.clone()))
        .with_source(BINDING_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": venue,
        "operation": ACCOUNT_MODE_PROBE,
        "scope": probe.and_then(probe_scope),
        "probeStatus": probe.map(|probe| format!("{:?}", probe.status).to_ascii_lowercase()),
        "probeSource": probe.map(|probe| probe.source.as_str()),
        "observedAtMs": observed_at_ms,
    }));
    problem
}

#[cfg(test)]
#[path = "account_binding_tests.rs"]
mod tests;
