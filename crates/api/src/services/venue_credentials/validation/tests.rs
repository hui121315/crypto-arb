use super::*;

#[test]
fn hyperliquid_relation_probe_accepts_direct_main_account_signer() {
    let account = "0x0123456789abcdef0123456789abcdef01234567";
    let probe = hyperliquid_account_relation_probe(
        7,
        Some(Ok(HyperliquidAccountRelation {
            main_account: account.into(),
            main_account_role: "user".into(),
            main_account_owner: None,
            signer: account.into(),
            signer_role: "user".into(),
            signer_owner: None,
            vault_address: None,
            vault_role: None,
            vault_leader: None,
        })),
    );

    assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
    assert_eq!(probe.kind, "account_signer_vault_relation");
    assert!(probe.message.contains("derived signer="));
}

#[test]
fn hyperliquid_relation_probe_accepts_approved_agent_for_led_vault() {
    let account = "0x0123456789abcdef0123456789abcdef01234567";
    let probe = hyperliquid_account_relation_probe(
        7,
        Some(Ok(HyperliquidAccountRelation {
            main_account: account.into(),
            main_account_role: "user".into(),
            main_account_owner: None,
            signer: "0x89abcdef0123456789abcdef0123456789abcdef".into(),
            signer_role: "agent".into(),
            signer_owner: Some(account.into()),
            vault_address: Some("0xfedcba9876543210fedcba9876543210fedcba98".into()),
            vault_role: Some("vault".into()),
            vault_leader: Some(account.into()),
        })),
    );

    assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
}

#[test]
fn hyperliquid_relation_probe_rejects_unrelated_signer_or_unproven_vault() {
    let account = "0x0123456789abcdef0123456789abcdef01234567";
    let unrelated_signer = hyperliquid_account_relation_probe(
        0,
        Some(Ok(HyperliquidAccountRelation {
            main_account: account.into(),
            main_account_role: "user".into(),
            main_account_owner: None,
            signer: "0x89abcdef0123456789abcdef0123456789abcdef".into(),
            signer_role: "agent".into(),
            signer_owner: Some("0x1111111111111111111111111111111111111111".into()),
            vault_address: None,
            vault_role: None,
            vault_leader: None,
        })),
    );
    assert_eq!(unrelated_signer.status, VenueCredentialProbeStatus::Failed);

    let missing_vault_leader = hyperliquid_account_relation_probe(
        0,
        Some(Ok(HyperliquidAccountRelation {
            main_account: account.into(),
            main_account_role: "user".into(),
            main_account_owner: None,
            signer: account.into(),
            signer_role: "user".into(),
            signer_owner: None,
            vault_address: Some("0xfedcba9876543210fedcba9876543210fedcba98".into()),
            vault_role: None,
            vault_leader: None,
        })),
    );
    assert_eq!(
        missing_vault_leader.status,
        VenueCredentialProbeStatus::Failed
    );
}

#[test]
fn hyperliquid_relation_probe_accepts_direct_subaccount_scope() {
    let account = "0x0123456789abcdef0123456789abcdef01234567";
    let probe = hyperliquid_account_relation_probe(
        0,
        Some(Ok(HyperliquidAccountRelation {
            main_account: account.into(),
            main_account_role: "subAccount".into(),
            main_account_owner: Some("0x1111111111111111111111111111111111111111".into()),
            signer: account.into(),
            signer_role: "subAccount".into(),
            signer_owner: None,
            vault_address: None,
            vault_role: None,
            vault_leader: None,
        })),
    );

    assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
    assert!(probe.message.contains("role=subAccount"));
}

#[test]
fn hyperliquid_abstraction_and_account_mode_probes_are_fail_closed() {
    let abstraction = hyperliquid_abstraction_probe(
        7,
        Some(Ok(HyperliquidAbstractionState {
            account_address: "0x0123456789abcdef0123456789abcdef01234567".into(),
            user_abstraction: "default".into(),
            user_dex_abstraction: Some("enabled".into()),
        })),
    );
    let relation = VenueCredentialProbe {
        kind: "account_signer_vault_relation".into(),
        status: VenueCredentialProbeStatus::Ok,
        scope: "test".into(),
        source: "test".into(),
        message: "verified".into(),
        checked_at_ms: 7,
        request_id: None,
    };

    let mode = hyperliquid_account_mode_probe(&relation, &abstraction);

    assert_eq!(abstraction.status, VenueCredentialProbeStatus::Ok);
    assert_eq!(abstraction.scope, "default");
    assert!(abstraction.message.contains("userAbstraction=default"));
    assert!(abstraction.message.contains("userDexAbstraction=enabled"));
    assert_eq!(mode.status, VenueCredentialProbeStatus::Ok);
    assert_eq!(mode.scope, "default");

    let unknown = hyperliquid_abstraction_probe(7, None);
    assert_eq!(
        hyperliquid_account_mode_probe(&relation, &unknown).status,
        VenueCredentialProbeStatus::Unknown
    );
}

#[test]
fn hyperliquid_order_permission_is_scoped_to_the_verified_account_relation() {
    let signed_action =
        safe_order_noop_probe("hyperliquid_noop", "hyperliquid.POST /exchange action=noop");
    let verified_relation = probe(
        "account_signer_vault_relation",
        VenueCredentialProbeStatus::Ok,
        "hyperliquid_account_signer_vault",
        "userRole(account)+userRole(signer)+vaultDetails",
        "verified",
    );
    let failed_relation = probe(
        "account_signer_vault_relation",
        VenueCredentialProbeStatus::Failed,
        "hyperliquid_account_signer_vault",
        "userRole(account)+userRole(signer)+vaultDetails",
        "wrong owner",
    );

    assert_eq!(
        hyperliquid_order_permission_probe(&verified_relation, signed_action.clone()).status,
        VenueCredentialProbeStatus::Ok
    );
    let blocked = hyperliquid_order_permission_probe(&failed_relation, signed_action);
    assert_eq!(blocked.status, VenueCredentialProbeStatus::Failed);
    assert!(blocked.message.contains("configured account"));
    assert!(blocked.message.contains("relation=failed"));
}

#[test]
fn hyperliquid_account_read_probes_preserve_partial_source_failure() {
    let read = exchange::VenueAccountRead {
        balances: vec![shared_types::VenueBalanceInfo {
            venue: "hyperliquid:spot".into(),
            currency: "USDC".into(),
            total: 1.0,
            available: 1.0,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        }],
        summaries: Vec::new(),
        asset_valuations: Vec::new(),
        issues: vec![exchange::VenueAccountReadIssue::new(
            "hyperliquid",
            "perp_margin",
            exchange::ExchangeError::Timeout { seconds: 3 },
        )],
    };

    let probes = hyperliquid_account_read_probes(7, Some(Ok(read)));
    let status = |kind: &str| {
        probes
            .iter()
            .find(|probe| probe.kind == kind)
            .map(|probe| probe.status)
    };

    assert_eq!(
        status("spot_truth_read"),
        Some(VenueCredentialProbeStatus::Ok)
    );
    assert_eq!(
        status("perp_margin_read"),
        Some(VenueCredentialProbeStatus::Unknown)
    );
    assert_eq!(
        status("balance_read"),
        Some(VenueCredentialProbeStatus::Unknown)
    );
}
