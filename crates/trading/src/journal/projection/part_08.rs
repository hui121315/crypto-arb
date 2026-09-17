impl OrderJournal {
    pub fn record_fill_by_order_identity_deferred_sql(
        &self,
        identity: FillOrderIdentity<'_>,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Vec<ExecutionLedgerEvent> {
        self.record_fill_by_order_identity_with_sql_mode(FillLedgerRecordRequest {
            identity,
            input,
            source,
            captured_at_ms,
            transport_metadata: None,
            sql_mode: LedgerSqlWriteMode::DeferredDurable,
        })
    }

    pub fn record_fill_by_order_identity_with_metadata_deferred_sql(
        &self,
        identity: FillOrderIdentity<'_>,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        transport_metadata: &OrderTransportMetadata,
    ) -> Vec<ExecutionLedgerEvent> {
        self.record_fill_by_order_identity_with_sql_mode(FillLedgerRecordRequest {
            identity,
            input,
            source,
            captured_at_ms,
            transport_metadata: Some(transport_metadata),
            sql_mode: LedgerSqlWriteMode::DeferredDurable,
        })
    }

    pub fn record_funding_by_venue_symbol_reported_deferred_sql(
        &self,
        venue: &str,
        symbol: &str,
        input: &FundingLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Result<ExecutionLedgerEvent, FundingPaymentIngestSkipReason> {
        self.record_funding_by_venue_symbol_reported_with_sql_mode(FundingLedgerRecordRequest {
            venue,
            symbol,
            input,
            source,
            captured_at_ms,
            sql_mode: LedgerSqlWriteMode::DeferredDurable,
        })
    }
}
