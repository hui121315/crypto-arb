use super::*;

#[cfg(not(test))]
pub(super) async fn optional_account_mode_probe<F>(
    fallback_scope: &str,
    fallback_source: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<Option<VenueAccountModeInfo>>>,
{
    match tokio::time::timeout(
        std::time::Duration::from_secs(OPTIONAL_READ_PROBE_TIMEOUT_SECS),
        request,
    )
    .await
    {
        Ok(Ok(Some(info))) => account_mode_probe_from_info(&info),
        Ok(Ok(None)) => probe(
            "account_mode_read",
            VenueCredentialProbeStatus::Unknown,
            fallback_scope,
            fallback_source,
            "read-only account mode probe not proven: adapter returned no account-mode evidence",
        ),
        Ok(Err(error)) => {
            let (status, message) = classify_optional_probe_error("account mode", &error);
            probe(
                "account_mode_read",
                status,
                fallback_scope,
                fallback_source,
                &message,
            )
        }
        Err(_) => probe(
            "account_mode_read",
            VenueCredentialProbeStatus::Unknown,
            fallback_scope,
            fallback_source,
            "read-only account mode probe timed out; save kept balance evidence",
        ),
    }
}

pub(super) fn account_mode_probe_from_info(info: &VenueAccountModeInfo) -> VenueCredentialProbe {
    let scope = info
        .account_scope
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("exchange_account");
    let message = account_mode_probe_message(info, scope);
    probe(
        "account_mode_read",
        VenueCredentialProbeStatus::Ok,
        scope,
        &info.source,
        &message,
    )
}

pub(super) fn account_mode_not_probed_probe(checked_at_ms: i64) -> VenueCredentialProbe {
    VenueCredentialProbe {
        kind: "account_mode_read".into(),
        status: VenueCredentialProbeStatus::Unknown,
        scope: "account_mode".into(),
        source: "not_probed".into(),
        message: "account mode is not proven by credential save".into(),
        checked_at_ms,
        request_id: common::request_id::current(),
    }
}

fn account_mode_probe_message(info: &VenueAccountModeInfo, scope: &str) -> String {
    let mode = info.mode.trim();
    let mode = if mode.is_empty() { "<empty>" } else { mode };
    if info.venue.eq_ignore_ascii_case("kucoin") && scope == "classic_futures" {
        return format!(
            "read-only account mode probe succeeded: KuCoin Classic Futures positionMode={mode}; balance_read uses Classic Futures /api/v1/account-overview availableMargin as futures buying power; UTA wallet scope is separate at /api/ua/v1/unified/account/overview and remains official beta/non-production"
        );
    }
    format!("read-only account mode probe succeeded: {mode}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_adds_not_probed_account_mode_probe() {
        let evidence = evidence(
            VenueCredentialValidationStatus::ReadOnlyOk,
            vec![probe(
                "balance_read",
                VenueCredentialProbeStatus::Ok,
                "USDT",
                "exchange_adapter.get_balance",
                "read-only balance probe succeeded",
            )],
        );

        assert!(evidence.probes.iter().any(|probe| {
            probe.kind == "account_mode_read"
                && probe.status == VenueCredentialProbeStatus::Unknown
                && probe.source == "not_probed"
                && probe.scope == "account_mode"
        }));
    }

    #[test]
    fn evidence_does_not_duplicate_explicit_account_mode_probe() {
        let evidence = evidence(
            VenueCredentialValidationStatus::ReadOnlyOk,
            vec![probe(
                "account_mode_read",
                VenueCredentialProbeStatus::Ok,
                "classic_futures",
                "kucoin.GET /api/v2/position/getPositionMode",
                "read-only account mode probe succeeded",
            )],
        );

        let account_mode_count = evidence
            .probes
            .iter()
            .filter(|probe| probe.kind == "account_mode_read")
            .count();

        assert_eq!(account_mode_count, 1);
        assert_eq!(evidence.probes[0].status, VenueCredentialProbeStatus::Ok);
        assert_eq!(
            evidence.probes[0].source,
            "kucoin.GET /api/v2/position/getPositionMode"
        );
    }

    #[test]
    fn kucoin_account_mode_probe_explains_classic_futures_scope() {
        let probe = account_mode_probe_from_info(&VenueAccountModeInfo {
            venue: "kucoin".to_owned(),
            mode: "hedge".to_owned(),
            source: "kucoin.GET /api/v2/position/getPositionMode".to_owned(),
            checked_at_ms: 42,
            freshness_ms: Some(0),
            account_scope: Some("classic_futures".to_owned()),
        });

        assert_eq!(probe.kind, "account_mode_read");
        assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
        assert_eq!(probe.scope, "classic_futures");
        assert_eq!(probe.source, "kucoin.GET /api/v2/position/getPositionMode");
        assert!(probe.message.contains("positionMode=hedge"));
        assert!(probe.message.contains("availableMargin"));
        assert!(probe.message.contains("UTA wallet scope is separate"));
        assert!(probe
            .message
            .contains("/api/ua/v1/unified/account/overview"));
    }

    #[test]
    fn binance_account_mode_probe_preserves_official_position_side_source() {
        let probe = account_mode_probe_from_info(&VenueAccountModeInfo {
            venue: "binance".to_owned(),
            mode: "hedge".to_owned(),
            source: "binance.GET /fapi/v1/positionSide/dual".to_owned(),
            checked_at_ms: 42,
            freshness_ms: Some(0),
            account_scope: Some("usds_m_futures".to_owned()),
        });

        assert_eq!(probe.kind, "account_mode_read");
        assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
        assert_eq!(probe.scope, "usds_m_futures");
        assert_eq!(probe.source, "binance.GET /fapi/v1/positionSide/dual");
        assert!(probe.message.contains("hedge"));
    }

    #[test]
    fn okx_account_mode_probe_preserves_account_config_source() {
        let probe = account_mode_probe_from_info(&VenueAccountModeInfo {
            venue: "okx".to_owned(),
            mode: "long_short_mode".to_owned(),
            source: "okx.GET /api/v5/account/config".to_owned(),
            checked_at_ms: 42,
            freshness_ms: Some(0),
            account_scope: None,
        });

        assert_eq!(probe.kind, "account_mode_read");
        assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
        assert_eq!(probe.scope, "exchange_account");
        assert_eq!(probe.source, "okx.GET /api/v5/account/config");
        assert!(probe.message.contains("long_short_mode"));
    }

    #[test]
    fn bybit_account_mode_probe_preserves_official_account_info_source() {
        let probe = account_mode_probe_from_info(&VenueAccountModeInfo {
            venue: "bybit".to_owned(),
            mode: "uta2; marginMode=REGULAR_MARGIN".to_owned(),
            source: "bybit.GET /v5/account/info".to_owned(),
            checked_at_ms: 42,
            freshness_ms: Some(0),
            account_scope: Some("uta2".to_owned()),
        });

        assert_eq!(probe.status, VenueCredentialProbeStatus::Ok);
        assert_eq!(probe.scope, "uta2");
        assert_eq!(probe.source, "bybit.GET /v5/account/info");
        assert!(probe.message.contains("REGULAR_MARGIN"));
    }
}
