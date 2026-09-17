fn credential_probe_status_label(status: VenueCredentialProbeStatus) -> &'static str {
    match status {
        VenueCredentialProbeStatus::Ok => "通过",
        VenueCredentialProbeStatus::Failed => "失败",
        VenueCredentialProbeStatus::Unknown => "未知",
    }
}

fn credential_probe_evidence(
    venue: &str,
    validation_status: VenueCredentialValidationStatus,
    probe: &VenueCredentialProbe,
) -> VenueOperationEvidence {
    let request_context = credential_probe_request_context(validation_status, probe);
    if let Some(evidence) = credential_probe_endpoint_evidence(venue, probe) {
        return endpoint_snapshot_operation_evidence(
            &evidence,
            probe.request_id.clone(),
            request_context,
        );
    }
    credential_probe_fallback_evidence(probe, request_context)
}

fn credential_probe_request_context(
    validation_status: VenueCredentialValidationStatus,
    probe: &VenueCredentialProbe,
) -> Vec<String> {
    let mut request_context = vec![
        format!("probe_kind={}", probe.kind),
        format!(
            "probe_status={}",
            credential_probe_status_name(probe.status)
        ),
        format!(
            "validation_status={}",
            credential_validation_status_name(validation_status)
        ),
        format!("probe_source={}", probe.source),
        format!("probe_scope={}", probe.scope),
    ];
    if probe.kind == "order_permission" && probe.status != VenueCredentialProbeStatus::Ok {
        request_context.push("does_not_grant_live_write=true".to_owned());
    }
    request_context
}

fn credential_probe_endpoint_evidence(
    venue: &str,
    probe: &VenueCredentialProbe,
) -> Option<EndpointEvidenceSnapshot> {
    let venue_id = shared_types::VenueId::from_exchange_name(venue)?;
    exchange::venue_spec::ENDPOINT_SPECS
        .iter()
        .filter(|spec| spec.venue == venue_id && credential_probe_endpoint_matches(probe, spec))
        .find_map(|spec| exchange::endpoint_evidence(venue_id.as_str(), spec.method, spec.path))
}

fn credential_probe_endpoint_matches(probe: &VenueCredentialProbe, spec: &EndpointSpec) -> bool {
    match probe.kind.as_str() {
        "balance_read" => {
            spec.use_case == EndpointUseCase::PrivateRead
                && spec.data_kind == EndpointDataKind::AccountBalance
        }
        "positions_read" => {
            spec.use_case == EndpointUseCase::PrivateRead
                && spec.data_kind == EndpointDataKind::AccountPosition
        }
        "open_orders_read" => {
            spec.use_case == EndpointUseCase::PrivateRead
                && spec.data_kind == EndpointDataKind::OrderStatus
                && is_open_orders_path(spec.path)
        }
        "account_mode_read" => {
            spec.use_case == EndpointUseCase::PrivateRead
                && spec.data_kind == EndpointDataKind::AccountConfig
                && account_mode_endpoint_matches_probe(probe, spec.path)
        }
        "order_permission" => order_permission_endpoint_matches(probe, spec),
        _ => false,
    }
}

fn order_permission_endpoint_matches(probe: &VenueCredentialProbe, spec: &EndpointSpec) -> bool {
    if probe.source == "hyperliquid.POST /exchange action=noop" {
        return false;
    }
    spec.use_case == EndpointUseCase::TradeWrite
        && spec.data_kind == EndpointDataKind::OrderAck
        && source_mentions_method_path(&probe.source, spec.method, spec.path)
}

fn source_mentions_method_path(source: &str, method: exchange::HttpMethod, path: &str) -> bool {
    source.split('+').any(|segment| {
        source_segment_method_path(segment, method).is_some_and(|candidate| candidate == path)
    })
}

fn source_segment_method_path(segment: &str, method: exchange::HttpMethod) -> Option<&str> {
    let marker = format!("{} ", method.as_str());
    let after_method = segment.split_once(&marker)?.1.trim_start();
    after_method.split_whitespace().next()
}

fn account_mode_endpoint_matches_probe(probe: &VenueCredentialProbe, path: &str) -> bool {
    probe.source.contains(path)
}

fn is_open_orders_path(path: &str) -> bool {
    path.contains("openOrders")
        || path.contains("orders-pending")
        || path.contains("unfilled-orders")
        || path.ends_with("/orders")
}

fn credential_probe_fallback_evidence(
    probe: &VenueCredentialProbe,
    request_context: Vec<String>,
) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: probe.source.clone(),
        path: probe.scope.clone(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: probe.request_id.clone(),
        request_context,
        doc_urls: Vec::new(),
        use_cases: vec!["credential_validation".to_owned()],
        data_kinds: vec![probe.kind.clone()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn credential_probe_status_name(status: VenueCredentialProbeStatus) -> &'static str {
    match status {
        VenueCredentialProbeStatus::Ok => "ok",
        VenueCredentialProbeStatus::Failed => "failed",
        VenueCredentialProbeStatus::Unknown => "unknown",
    }
}

fn credential_validation_status_name(status: VenueCredentialValidationStatus) -> &'static str {
    match status {
        VenueCredentialValidationStatus::ReadOnlyOk => "read_only_ok",
        VenueCredentialValidationStatus::LocalOnly => "local_only",
        VenueCredentialValidationStatus::Unknown => "unknown",
    }
}

fn market_status(quality: MarketQuality) -> VenueOperationStatus {
    match quality {
        MarketQuality::Fresh => VenueOperationStatus::Ok,
        MarketQuality::Warming => VenueOperationStatus::Unknown,
        MarketQuality::StaleAllowed => VenueOperationStatus::Warn,
        MarketQuality::Missing | MarketQuality::RateLimited | MarketQuality::CircuitOpen => {
            VenueOperationStatus::Blocked
        }
        MarketQuality::Unsupported => VenueOperationStatus::Unsupported,
    }
}

const ACCOUNT_CACHE_REFRESH_GRACE_MS: i64 = 8_000;

fn account_cache_status(
    operation: &str,
    quality: AccountCacheQuality,
    freshness_ms: i64,
) -> VenueOperationStatus {
    match quality {
        AccountCacheQuality::Fresh => VenueOperationStatus::Ok,
        AccountCacheQuality::Stale if freshness_ms <= account_cache_refresh_grace_ms(operation) => {
            VenueOperationStatus::Ok
        }
        AccountCacheQuality::Stale => VenueOperationStatus::Warn,
        AccountCacheQuality::Expired | AccountCacheQuality::WrongEpoch => {
            VenueOperationStatus::Blocked
        }
    }
}

fn missing_cache_status(supported: bool, configured: bool) -> VenueOperationStatus {
    match (supported, configured) {
        (false, _) => VenueOperationStatus::Unsupported,
        (true, false) => VenueOperationStatus::Blocked,
        (true, true) => VenueOperationStatus::Unknown,
    }
}

fn account_cache_message(
    operation: &str,
    quality: AccountCacheQuality,
    freshness_ms: i64,
) -> &'static str {
    match quality {
        AccountCacheQuality::Fresh => "账户缓存有新鲜运行态样本",
        AccountCacheQuality::Stale if freshness_ms <= account_cache_refresh_grace_ms(operation) => {
            "账户快照可用，后台刷新中"
        }
        AccountCacheQuality::Stale => "账户缓存样本可用但已变旧",
        AccountCacheQuality::Expired => "账户缓存样本过旧，不能作为运行态证据",
        AccountCacheQuality::WrongEpoch => "账户缓存来自旧 adapter epoch，不能作为运行态证据",
    }
}

fn account_cache_refresh_grace_ms(operation: &str) -> i64 {
    match operation {
        OP_POSITIONS => POSITION_CACHE_MAX_STALE_MS,
        OP_BALANCE => BALANCE_CACHE_TTL_MS.saturating_add(ACCOUNT_CACHE_REFRESH_GRACE_MS),
        _ => ACCOUNT_CACHE_REFRESH_GRACE_MS,
    }
}

fn missing_cache_message(supported: bool, configured: bool) -> &'static str {
    match (supported, configured) {
        (false, _) => "当前 adapter 未声明支持该账户读取",
        (true, false) => "凭证字段未完整配置，无法读取账户运行态样本",
        (true, true) => "尚未取得账户运行态样本",
    }
}

fn private_ws_missing_message(supported: bool, configured: bool) -> &'static str {
    match (supported, configured) {
        (false, _) => "当前 adapter 未声明支持私有 WS 运行态",
        (true, false) => "凭证字段未完整配置，无法建立私有 WS",
        (true, true) => "尚未取得私有 WS 运行态样本",
    }
}

fn reconciliation_missing_message(supported: bool, configured: bool) -> &'static str {
    match (supported, configured) {
        (false, _) => "当前 adapter 未声明支持订单回查",
        (true, false) => "凭证字段未完整配置，无法执行订单回查",
        (true, true) => "尚未取得订单回查运行态样本",
    }
}

fn run_finality_missing_message(supported: bool, configured: bool) -> &'static str {
    match (supported, configured) {
        (false, _) => "当前 adapter 未声明支持订单终态回查",
        (true, false) => "凭证字段未完整配置，无法执行订单终态回查",
        (true, true) => "尚未取得订单终态回查运行态样本",
    }
}

fn account_cache_error(status: VenueOperationStatus, message: &str) -> Option<String> {
    matches!(
        status,
        VenueOperationStatus::Blocked | VenueOperationStatus::Unsupported
    )
    .then(|| message.to_owned())
}

fn market_message(quality: MarketQuality) -> &'static str {
    match quality {
        MarketQuality::Fresh => "缓存内有新鲜运行态样本",
        MarketQuality::Warming => "WebSocket 订阅预热中，等待缺失标的首帧",
        MarketQuality::StaleAllowed => "缓存样本可用但已变旧",
        MarketQuality::Missing => "运行态样本缺失",
        MarketQuality::RateLimited => "交易所返回限流或处于退避窗口",
        MarketQuality::CircuitOpen => "交易所请求熔断中",
        MarketQuality::Unsupported => "当前 adapter 不支持该操作",
    }
}

fn http_operation_evidence(snapshot: &HttpOutcomeMetricSnapshot) -> VenueOperationEvidence {
    let Some(evidence) = snapshot.endpoint_evidence.as_ref() else {
        return http_fallback_operation_evidence(snapshot);
    };
    endpoint_snapshot_operation_evidence(
        evidence,
        snapshot.last_request_id.clone(),
        snapshot.last_request_context.clone(),
    )
}
