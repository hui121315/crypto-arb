use super::*;

pub(in super::super) async fn validate_hyperliquid(
    values: &FieldValues<'_>,
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError> {
    let account_address = values.hyperliquid_account_address()?;
    let private_key = values.get("private_key")?;
    let vault_address = values.optional("vault_address");
    let _agent_address = exchange::signing::hyperliquid::address_from_private_key(&private_key)
        .map_err(|error| CredentialUpdateError::Validation(error.to_string()))?;
    if !valid_evm_address(&account_address) {
        Err(CredentialUpdateError::Validation(
            "hyperliquid account address must be a 0x-prefixed EVM address".into(),
        ))
    } else if vault_address
        .as_deref()
        .is_some_and(|vault| !valid_evm_address(vault))
    {
        Err(CredentialUpdateError::Validation(
            "hyperliquid vault address must be a 0x-prefixed EVM address".into(),
        ))
    } else {
        let adapter = Hyperliquid::new(HyperliquidConfig {
            credentials: Some(HyperliquidCredentials {
                user_address: account_address,
                private_key: Some(private_key),
                vault_address,
            }),
            timeout_secs: HTTP_TIMEOUT_SECS,
            ..Default::default()
        })
        .map_err(|error| validation_error(&error))?;
        let account_relation =
            optional_hyperliquid_account_relation_probe(adapter.credential_relation());
        let account_abstraction =
            optional_hyperliquid_abstraction_probe(adapter.account_abstraction_state());
        let account_read =
            optional_hyperliquid_account_read_probes(adapter.get_account_read(Some("USDC")));
        let private_reads = optional_private_read_probes(&adapter);
        let order_permission = optional_safe_order_noop_probe(
            "hyperliquid_noop",
            "hyperliquid.POST /exchange action=noop",
            adapter.validate_safe_noop_permission(),
        );
        let (account_relation, account_abstraction, account_read, private_reads, order_permission) = tokio::join!(
            account_relation,
            account_abstraction,
            account_read,
            private_reads,
            order_permission,
        );
        let order_permission =
            hyperliquid_order_permission_probe(&account_relation, order_permission);
        let account_mode = hyperliquid_account_mode_probe(&account_relation, &account_abstraction);
        let mut probes = vec![
            probe(
                "local_format",
                VenueCredentialProbeStatus::Ok,
                "account_address/private_key",
                "hyperliquid_signing",
                "account address and API/Agent private-key derivation passed",
            ),
            account_relation,
            account_abstraction,
            account_mode,
            order_permission,
        ];
        probes.extend(account_read);
        probes.extend(private_reads);
        Ok(read_only_evidence_with_order_permission_scopes(
            probes,
            &[
                VenueCredentialPermission::PlaceOrder,
                VenueCredentialPermission::CancelOrder,
            ],
        ))
    }
}

fn valid_evm_address(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() == 42
        && trimmed.starts_with("0x")
        && trimmed[2..].chars().all(|ch| ch.is_ascii_hexdigit())
}
