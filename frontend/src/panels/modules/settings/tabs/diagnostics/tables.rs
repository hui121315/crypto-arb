use super::*;

pub(super) struct DiagnosticsTables {
    pub(super) env: TableRuntimeHandle<EnvTemplateLine>,
    pub(super) health: TableRuntimeHandle<VenueOperationHealth>,
    pub(super) access: TableRuntimeHandle<MarketCacheAccessRow>,
    pub(super) status: TableRuntimeHandle<MarketDataSnapshotStatusRow>,
    pub(super) row_evidence: TableRuntimeHandle<MarketDataRowEvidence>,
}

pub(super) fn diagnostics_tables(
    template: SettingsResource<EnvTemplateResponse>,
    operation_health: SettingsResource<VenueOperationHealthSnapshot>,
    market_diagnostics: SettingsResource<MarketDataDiagnosticsSnapshot>,
    funding_rates: SettingsResource<shared_types::FundingRatesEnvelope>,
    health_query: RwSignal<String>,
    health_status_filter: RwSignal<HealthStatusFilter>,
) -> DiagnosticsTables {
    let env_rows = Memo::new(move |_| {
        settings_value(template)
            .map(|template| template.lines)
            .unwrap_or_default()
    });
    let status_rows = Memo::new(move |_| {
        settings_value(market_diagnostics)
            .map(|snapshot| snapshot.status.rows)
            .unwrap_or_default()
    });
    let access_rows = Memo::new(move |_| {
        settings_value(market_diagnostics)
            .map(|snapshot| snapshot.access_rows)
            .unwrap_or_default()
    });
    let row_evidence_rows = Memo::new(move |_| {
        settings_value(funding_rates)
            .map(|envelope| envelope.row_evidence)
            .unwrap_or_default()
    });
    let health_rows = Memo::new(move |_| {
        let normalized_query = normalized_search_query(&health_query.get());
        let selected_filter = health_status_filter.get();
        settings_value(operation_health)
            .map(|snapshot| {
                filtered_operation_health_rows(snapshot.rows, &normalized_query, selected_filter)
            })
            .unwrap_or_default()
    });
    let health_dataset_key =
        Memo::new(move |_| health_filter_key(health_query, health_status_filter));

    DiagnosticsTables {
        env: use_table_runtime(
            DIAGNOSTICS_ENV_PAGE_KEY,
            static_dataset_key("diagnostics:env"),
            env_rows,
            ENV_TEMPLATE_PAGE_SIZE,
        ),
        health: use_table_runtime(
            DIAGNOSTICS_HEALTH_PAGE_KEY,
            health_dataset_key,
            health_rows,
            HEALTH_PAGE_SIZE,
        ),
        access: use_table_runtime(
            DIAGNOSTICS_ACCESS_PAGE_KEY,
            static_dataset_key("diagnostics:market-access"),
            access_rows,
            ACCESS_PAGE_SIZE,
        ),
        status: use_table_runtime(
            DIAGNOSTICS_STATUS_PAGE_KEY,
            static_dataset_key("diagnostics:market-status"),
            status_rows,
            STATUS_PAGE_SIZE,
        ),
        row_evidence: use_table_runtime(
            DIAGNOSTICS_ROW_EVIDENCE_PAGE_KEY,
            static_dataset_key("diagnostics:row-evidence"),
            row_evidence_rows,
            ROW_EVIDENCE_PAGE_SIZE,
        ),
    }
}

pub(super) fn static_dataset_key(key: &'static str) -> Memo<String> {
    Memo::new(move |_| key.to_owned())
}

pub(super) fn health_filter_key(
    query: RwSignal<String>,
    status_filter: RwSignal<HealthStatusFilter>,
) -> String {
    format!(
        "{}:{}",
        normalized_search_query(&query.get()),
        status_filter.get().as_key()
    )
}
