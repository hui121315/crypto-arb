fn credentials_configured(venue: &VenueCredentialStatus) -> bool {
    if VenueId::from_exchange_name(&venue.venue) == Some(VenueId::Kraken) {
        return credential_pair_configured(venue, "spot_api_key", "spot_api_secret")
            || credential_pair_configured(venue, "futures_api_key", "futures_api_secret");
    }
    venue.fields.iter().any(|field| field.required)
        && venue
            .fields
            .iter()
            .filter(|field| field.required)
            .all(|field| field.configured)
}

fn credential_pair_configured(venue: &VenueCredentialStatus, key: &str, secret: &str) -> bool {
    [key, secret].iter().all(|expected| {
        venue
            .fields
            .iter()
            .any(|field| field.key == *expected && field.configured)
    })
}

fn credential_status(supported: bool, configured: bool) -> VenueOperationStatus {
    match (supported, configured) {
        (false, _) => VenueOperationStatus::Unsupported,
        (true, false) => VenueOperationStatus::Blocked,
        (true, true) => VenueOperationStatus::Unknown,
    }
}

fn credential_message(venue: &VenueCredentialStatus, supported: bool, configured: bool) -> String {
    match (supported, configured) {
        (false, _) => "当前 adapter 未声明支持该操作".to_owned(),
        (true, false) => format!(
            "需配置 {}：{}",
            venue.label,
            missing_credential_fields(venue)
        ),
        (true, true) => "凭证字段已配置，仍需运行态验证".to_owned(),
    }
}

fn missing_credential_fields(venue: &VenueCredentialStatus) -> String {
    if VenueId::from_exchange_name(&venue.venue) == Some(VenueId::Kraken) {
        return "Spot API Key + Secret 或 Futures API Key + Secret（至少一套完整凭证）".to_owned();
    }
    let fields = venue
        .fields
        .iter()
        .filter(|field| field.required && !field.configured)
        .map(|field| format!("{}（{}）", field.label, field.env_key))
        .collect::<Vec<_>>();
    if fields.is_empty() {
        "必填凭证字段".to_owned()
    } else {
        fields.join("、")
    }
}

fn credential_probe_operation_status(status: VenueCredentialProbeStatus) -> VenueOperationStatus {
    match status {
        VenueCredentialProbeStatus::Ok => VenueOperationStatus::Ok,
        VenueCredentialProbeStatus::Failed => VenueOperationStatus::Blocked,
        VenueCredentialProbeStatus::Unknown => VenueOperationStatus::Unknown,
    }
}

fn credential_probe_supported(venue: &VenueCredentialStatus, kind: &str) -> bool {
    if matches!(
        kind,
        "account_signer_vault_relation"
            | "account_abstraction"
            | "perp_margin_read"
            | "spot_truth_read"
            | "local_format"
    ) {
        return venue.private_read;
    }
    let kind = VenueOperationKind::from_credential_probe_kind(kind);
    if kind.credential_probe_requires_private_read() {
        venue.private_read
    } else if kind.credential_probe_requires_order_write() {
        venue.live_write
    } else {
        false
    }
}

fn credential_probe_message(
    validation_status: VenueCredentialValidationStatus,
    probe: &VenueCredentialProbe,
) -> String {
    format!(
        "凭证保存验证 {} / {}：{}",
        credential_validation_status_label(validation_status),
        credential_probe_status_label(probe.status),
        probe.message
    )
}

fn credential_validation_status_label(status: VenueCredentialValidationStatus) -> &'static str {
    match status {
        VenueCredentialValidationStatus::ReadOnlyOk => "只读通过",
        VenueCredentialValidationStatus::LocalOnly => "仅本地格式",
        VenueCredentialValidationStatus::Unknown => "未证明",
    }
}
