const SOURCE_HYPERLIQUID_SIGNER_SESSION: &str = "hyperliquid_signer_session";
const HYPERLIQUID_NONCE_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/nonces-and-api-wallets";

fn hyperliquid_signer_session_rows(now_ms: i64) -> Vec<VenueOperationHealth> {
    exchange::hyperliquid_signer_session_health()
        .into_iter()
        .map(|snapshot| hyperliquid_signer_session_row(&snapshot, now_ms))
        .collect()
}

fn hyperliquid_signer_session_row(
    snapshot: &exchange::HyperliquidSignerSessionHealth,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = if snapshot.last_error.is_some() {
        VenueOperationStatus::Blocked
    } else {
        VenueOperationStatus::Ok
    };
    let scope = format!(
        "account={} signer={} vault={}",
        snapshot.account_address,
        snapshot.signer_address,
        snapshot.vault_address.as_deref().unwrap_or("none")
    );
    let message = format!(
        "Hyperliquid signer session {scope}; network={}; ownership={}; last_nonce={}; last_error={}",
        snapshot.network,
        snapshot.ownership_boundary,
        snapshot.last_nonce,
        snapshot.last_error.as_deref().unwrap_or("none")
    );
    let problem = snapshot.last_error.as_ref().map(|error| {
        let mut problem = ApiProblem::new(
            "HYPERLIQUID_SIGNER_SESSION_ERROR",
            format!("{scope}: {error}"),
        )
        .with_source(SOURCE_HYPERLIQUID_SIGNER_SESSION);
        problem.details = Some(serde_json::json!({
            "accountAddress": snapshot.account_address,
            "signerAddress": snapshot.signer_address,
            "vaultAddress": snapshot.vault_address,
            "network": snapshot.network,
            "ownershipBoundary": snapshot.ownership_boundary,
            "lastNonce": snapshot.last_nonce,
        }));
        problem
    });
    VenueOperationHealth {
        venue: "hyperliquid".to_owned(),
        operation: OP_PRIVATE_WS_SESSION.to_owned(),
        status,
        source: SOURCE_HYPERLIQUID_SIGNER_SESSION.to_owned(),
        message,
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(u64::from(snapshot.last_error.is_none())),
        freshness_ms: Some(now_ms.saturating_sub(snapshot.observed_at_ms).max(0)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: snapshot.last_error.clone(),
        evidence: Some(VenueOperationEvidence {
            method: "hyperliquid_ws_post".to_owned(),
            path: "post/action".to_owned(),
            checked_at: snapshot.observed_at_ms.to_string(),
            doc_version: "hyperliquid-nonce-api-wallet".to_owned(),
            schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
            fixture_id: "hyperliquid_signer_session_health".to_owned(),
            parser_test: "signer_session_health_records_scope_nonce_and_last_error".to_owned(),
            request_builder_test: "signer_nonce_key_distinguishes_network_and_is_signer_scoped"
                .to_owned(),
            auth_kind: "L1 API wallet signature".to_owned(),
            request_id: None,
            request_context: vec![
                format!("account_address={}", snapshot.account_address),
                format!("signer_address={}", snapshot.signer_address),
                format!(
                    "vault_address={}",
                    snapshot.vault_address.as_deref().unwrap_or("none")
                ),
                format!("last_nonce={}", snapshot.last_nonce),
                format!(
                    "last_error={}",
                    snapshot.last_error.as_deref().unwrap_or("none")
                ),
            ],
            doc_urls: vec![HYPERLIQUID_NONCE_DOC_URL.to_owned()],
            use_cases: vec!["trade_write".to_owned(), "runtime_health".to_owned()],
            data_kinds: vec!["signer_nonce_session".to_owned()],
            rate_scopes: vec![snapshot.ownership_boundary.to_owned()],
            weight: 0,
        }),
        problem,
        observed_at_ms: snapshot.observed_at_ms,
    }
}

#[cfg(test)]
mod hyperliquid_signer_session_tests {
    use super::*;

    #[test]
    fn signer_session_row_exposes_scope_nonce_and_error() {
        let snapshot = exchange::HyperliquidSignerSessionHealth {
            account_address: "0x1111111111111111111111111111111111111111".into(),
            signer_address: "0x2222222222222222222222222222222222222222".into(),
            vault_address: Some("0x3333333333333333333333333333333333333333".into()),
            network: "mainnet".into(),
            ownership_boundary: "one_api_wallet_per_trading_process",
            last_nonce: 42,
            last_error: Some("nonce rejected".into()),
            observed_at_ms: 90,
        };

        let row = hyperliquid_signer_session_row(&snapshot, 100);

        assert_eq!(row.status, VenueOperationStatus::Blocked);
        assert_eq!(row.operation, OP_PRIVATE_WS_SESSION);
        assert!(row.message.contains("last_nonce=42"));
        assert!(row.message.contains("last_error=nonce rejected"));
        assert!(row.evidence.as_ref().is_some_and(|evidence| {
            evidence
                .request_context
                .iter()
                .any(|item| item == "last_nonce=42")
        }));
    }
}
