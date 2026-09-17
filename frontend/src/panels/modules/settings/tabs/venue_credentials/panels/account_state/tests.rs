use super::selection::selected_account_evidence;
use super::*;
use shared_types::{
    AccountBindingEvidence, AccountBindingStatus, AccountDataHealth, AccountFieldQuality,
    AccountFieldQualityStatus, AccountFieldSubject,
};

#[test]
fn selected_account_evidence_keeps_parent_venue_children_but_not_sibling_scopes() {
    let snapshot = AccountStateSnapshot {
        field_quality: vec![
            AccountFieldQuality::new(
                AccountFieldSubject::balance("hyperliquid:xyz", "USDC"),
                "available",
                AccountFieldQualityStatus::Actual,
                "test",
                Some(1),
            ),
            AccountFieldQuality::new(
                AccountFieldSubject::balance("hyperliquid:km", "USDC"),
                "available",
                AccountFieldQualityStatus::Invalid,
                "test",
                Some(1),
            ),
        ],
        ..Default::default()
    };

    let parent = selected_account_evidence(&snapshot, "hyperliquid");
    let exact = selected_account_evidence(&snapshot, "hyperliquid:xyz");

    assert_eq!(parent.field_quality.len(), 2);
    assert_eq!(exact.field_quality.len(), 1);
    assert_eq!(
        exact.field_quality[0].subject.venue.as_deref(),
        Some("hyperliquid:xyz")
    );
}

#[test]
fn selected_account_evidence_keeps_row_health_binding_and_scoped_problem() {
    let row_health = AccountDataHealth::new(
        AccountFieldSubject::balance("okx", "USDT"),
        "account_balance_runtime",
        10,
    );
    let binding = AccountBindingEvidence {
        venue: "okx".to_owned(),
        account_scope: Some("cross".to_owned()),
        status: AccountBindingStatus::Verified,
        source: "credential_probe".to_owned(),
        checked_at_ms: Some(10),
        freshness_ms: Some(20),
        credential_fingerprint: Some("fingerprint".to_owned()),
        problem: None,
    };
    let mut problem = ApiProblem::new("BALANCE_FIELD_UNAVAILABLE", "bad field");
    problem.details = Some(serde_json::json!({ "venue": "okx" }));
    let balances = shared_types::VenueBalanceEnvelope::new(
        Vec::new(),
        shared_types::ListStatus::Fresh,
        "account_balance_runtime",
        10,
        Vec::new(),
        Vec::new(),
    )
    .with_row_health(vec![row_health])
    .with_account_bindings(vec![binding]);
    let snapshot = AccountStateSnapshot {
        balances,
        problems: vec![problem],
        ..Default::default()
    };

    let selection = selected_account_evidence(&snapshot, "okx");

    assert_eq!(selection.row_health.len(), 1);
    assert_eq!(selection.bindings.len(), 1);
    assert_eq!(selection.problems.len(), 1);
}
