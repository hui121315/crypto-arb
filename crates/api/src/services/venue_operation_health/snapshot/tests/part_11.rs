#[tokio::test]
async fn snapshot_exposes_instrument_and_opportunity_generation_evidence() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;

    let health = snapshot(&state);
    let opportunity = health
        .rows
        .iter()
        .find(|row| row.operation == shared_types::OP_OPPORTUNITY_SNAPSHOT)
        .expect("opportunity snapshot health row");
    assert_eq!(opportunity.venue, SYSTEM_VENUE);
    assert_eq!(opportunity.status, VenueOperationStatus::Unknown);
    assert!(opportunity.evidence.as_ref().is_some_and(|evidence| {
        evidence.path == "opportunity_index" && evidence.doc_version == "versioned-atomic-v1"
    }));

    let instruments = health
        .rows
        .iter()
        .filter(|row| row.operation == shared_types::OP_REST_INSTRUMENT_SPECS)
        .collect::<Vec<_>>();
    assert!(!instruments.is_empty());
    assert!(instruments.iter().all(|row| {
        row.evidence.as_ref().is_some_and(|evidence| {
            let recorded = evidence.fixture_id != UNRECORDED_EVIDENCE_MARKER
                    && evidence.parser_test != UNRECORDED_EVIDENCE_MARKER
                    && evidence.request_builder_test != UNRECORDED_EVIDENCE_MARKER
                    && evidence.schema_hash != UNRECORDED_EVIDENCE_MARKER
                    && evidence
                        .doc_urls
                        .iter()
                        .all(|url| url.starts_with("https://"))
                    && evidence
                        .data_kinds
                        .iter()
                        .any(|kind| kind == "instrument_spec")
                    && evidence
                        .request_context
                        .iter()
                        .any(|context| context == "fail_closed=true");
            recorded
                || (row.supported == Some(false)
                    && evidence.request_context.iter().any(|context| {
                        context == "instrument_metadata_endpoint_missing=true"
                    }))
        })
    }));
    let binance = instruments
        .iter()
        .find(|row| row.venue == "binance")
        .and_then(|row| row.evidence.as_ref())
        .expect("binance instrument evidence");
    assert_eq!(binance.path, "/fapi/v1/exchangeInfo");
    assert_eq!(
        binance.doc_version,
        "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11"
    );
    Ok(())
}
