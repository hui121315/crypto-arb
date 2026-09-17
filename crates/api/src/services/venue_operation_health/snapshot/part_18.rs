fn instrument_registry_rows(state: &AppState, now_ms: i64) -> Vec<VenueOperationHealth> {
    state
        .instrument_registry()
        .runtime_health(now_ms)
        .into_iter()
        .map(|health| {
            let evidence = instrument_registry_evidence(&health);
            let evidence_recorded = evidence.fixture_id != UNRECORDED_EVIDENCE_MARKER;
            VenueOperationHealth {
                venue: health.venue,
                operation: shared_types::OP_REST_INSTRUMENT_SPECS.to_owned(),
                status: health.status,
                source: SOURCE_INSTRUMENT_REGISTRY.to_owned(),
                message: health.message,
                supported: Some(
                    evidence_recorded && health.status != VenueOperationStatus::Unsupported,
                ),
                configured: Some(true),
                requested: Some(1),
                rows: Some(health.rows as u64),
                freshness_ms: health.freshness_ms,
                retry_after_ms: None,
                latency_ms: None,
                latency_p95_ms: None,
                error: health.problem.as_ref().map(|problem| problem.message.clone()),
                evidence: Some(evidence),
                problem: health.problem,
                observed_at_ms: health.checked_at_ms.max(1),
            }
        })
        .collect()
}

fn instrument_registry_evidence(
    health: &crate::services::instrument_registry::InstrumentRegistryRuntimeHealth,
) -> VenueOperationEvidence {
    let endpoint =
        crate::services::instrument_registry::instrument_metadata_evidence(&health.venue);
    let Some(endpoint) = endpoint else {
        return missing_instrument_registry_evidence(health);
    };
    let mut use_cases = endpoint.use_cases.clone();
    push_unique(&mut use_cases, "instrument_registry");
    push_unique(&mut use_cases, "hedge_sizing");
    let mut data_kinds = endpoint.data_kinds.clone();
    push_unique(&mut data_kinds, "instrument_spec");
    VenueOperationEvidence {
        method: endpoint.method.clone(),
        path: endpoint.path.clone(),
        checked_at: endpoint.checked_at.clone(),
        doc_version: endpoint.doc_version.clone(),
        schema_hash: endpoint.schema_hash.clone(),
        fixture_id: endpoint.fixture_id.clone(),
        parser_test: endpoint.parser_test.clone(),
        request_builder_test: endpoint.request_builder_test.clone(),
        auth_kind: endpoint.auth_kind.clone(),
        request_id: None,
        request_context: vec![
            format!("refresh_checked_at_ms={}", health.checked_at_ms),
            format!("rows={}", health.rows),
            format!("execution_ready_rows={}", health.execution_ready_rows),
            format!("schema_versions={}", health.schema_versions.join(",")),
            format!("source_urls={}", health.source_urls.join(",")),
            "fail_closed=true".to_owned(),
        ],
        doc_urls: endpoint.doc_urls.clone(),
        use_cases,
        data_kinds,
        rate_scopes: endpoint.rate_scopes.clone(),
        weight: endpoint.weight,
    }
}

fn missing_instrument_registry_evidence(
    health: &crate::services::instrument_registry::InstrumentRegistryRuntimeHealth,
) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        path: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        checked_at: health.checked_at_ms.to_string(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: "public".to_owned(),
        request_id: None,
        request_context: vec!["instrument_metadata_endpoint_missing=true".to_owned()],
        doc_urls: Vec::new(),
        use_cases: vec!["instrument_registry".to_owned()],
        data_kinds: vec!["instrument_spec".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

fn opportunity_snapshot_row(state: &AppState, now_ms: i64) -> VenueOperationHealth {
    let health = state.opportunity_index().health(now_ms);
    let status = if health.version == 0 {
        VenueOperationStatus::Unknown
    } else {
        VenueOperationStatus::Ok
    };
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: shared_types::OP_OPPORTUNITY_SNAPSHOT.to_owned(),
        status,
        source: SOURCE_OPPORTUNITY_INDEX.to_owned(),
        message: if health.version == 0 {
            "opportunity snapshot has not been published".to_owned()
        } else {
            format!(
                "atomic opportunity snapshot v{} contains {} rows",
                health.version, health.rows
            )
        },
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(health.rows as u64),
        freshness_ms: health.freshness_ms,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: Some(opportunity_snapshot_evidence(&health)),
        problem: None,
        observed_at_ms: health.published_at_ms.unwrap_or(now_ms),
    }
}

fn opportunity_snapshot_evidence(
    health: &crate::services::opportunity_index::OpportunityIndexHealth,
) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "ArcSwap::store".to_owned(),
        path: "opportunity_index".to_owned(),
        checked_at: health.published_at_ms.unwrap_or_default().to_string(),
        doc_version: "versioned-atomic-v1".to_owned(),
        schema_hash: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        fixture_id: "opportunity_index_atomic_publish".to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: "hedge_preview_snapshot_id_contract".to_owned(),
        auth_kind: "internal".to_owned(),
        request_id: None,
        request_context: vec![
            format!("version={}", health.version),
            format!("snapshot_id={}", health.snapshot_id.as_deref().unwrap_or("none")),
            format!("rows={}", health.rows),
        ],
        doc_urls: Vec::new(),
        use_cases: vec!["opportunity_list".to_owned(), "hedge_preview".to_owned()],
        data_kinds: vec!["opportunity_snapshot".to_owned()],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}
