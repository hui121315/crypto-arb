use super::*;
mod balance;
mod binance;
mod cache;
impl TradingService {
    fn private_record_account_matches(&self, record: &OrderRecord) -> bool {
        if self.engine.ensure_order_account(record).is_ok() { return true; }
        let identity = record.identity_snapshot();
        self.account_reader.load_full().is_some_and(|reader| {
            let key = (shared_types::normalized_venue_name(&record.intent.exchange), identity.product);
            reader.account_scopes.get(&key).is_some_and(|scope| trading::ExecutionEngine::record_matches_account(record, scope))
        })
    }

    fn private_order_record(&self, delta: &PrivateOrderDelta) -> Option<OrderRecord> {
        let record = self.journal.fill_record_for_identity(&FillOrderIdentity {
            venue: Some(&delta.order.exchange), exchange_order_id: Some(&delta.order.order_id),
            client_order_id: Some(&delta.client_order_id), symbol: Some(&delta.order.symbol), side: Some(delta.order.side),
        })?;
        self.private_record_account_matches(&record).then_some(record)
    }

    pub(crate) async fn apply_private_ws_event(
        &self,
        event: PrivateWsEvent,
    ) -> PrivateWsApplyOutcome {
        match event {
            PrivateWsEvent::Order(delta) => self.apply_private_ws_order(delta).await,
            PrivateWsEvent::OpenOrders(snapshot) => {
                self.replace_open_order_cache(&snapshot.venue, snapshot.rows);
                PrivateWsApplyOutcome {
                    open_order_cache_updated: true,
                    ..PrivateWsApplyOutcome::default()
                }
            }
            PrivateWsEvent::BinanceOrderTrade(delta) => {
                self.apply_binance_order_trade(*delta).await
            }
            PrivateWsEvent::Fill(delta) => self.apply_private_ws_fill(&delta, None),
            PrivateWsEvent::FillWithEvidence(delta) => {
                self.apply_private_ws_fill(&delta.fill, Some(&delta.transport_metadata))
            }
            PrivateWsEvent::Funding(delta) => self.apply_private_ws_funding(&delta),
            PrivateWsEvent::Liquidation(delta) => self.apply_private_ws_liquidation(&delta),
            PrivateWsEvent::NonUserCancel(delta) => self.apply_private_ws_non_user_cancel(&delta),
            PrivateWsEvent::Positions(snapshot) => {
                self.replace_position_cache(&snapshot.venue, snapshot.rows);
                PrivateWsApplyOutcome {
                    account_cache_updated: true,
                    ..PrivateWsApplyOutcome::default()
                }
            }
            PrivateWsEvent::PositionPatch(patch) => {
                if self.upsert_position_cache(&patch.venue, patch.rows) {
                    PrivateWsApplyOutcome {
                        account_cache_updated: true,
                        ..PrivateWsApplyOutcome::default()
                    }
                } else {
                    let dirty = self.mark_private_event_account_dirty(PrivateAccountDirty::new(
                        patch.venue,
                        PrivateAccountScope::Positions,
                        "position_patch_requires_rest_seed",
                    ));
                    PrivateWsApplyOutcome {
                        account_cache_dirty: Some(dirty),
                        ..PrivateWsApplyOutcome::default()
                    }
                }
            }
            PrivateWsEvent::Balances(snapshot) => {
                let balance_ledger_updated =
                    self.record_private_balance_events("snapshot", &snapshot.rows);
                self.replace_balance_cache(&snapshot.venue, snapshot.rows);
                PrivateWsApplyOutcome {
                    account_cache_updated: true,
                    balance_ledger_updated,
                    ..PrivateWsApplyOutcome::default()
                }
            }
            PrivateWsEvent::BalancePatch(patch) => {
                let balance_ledger_updated =
                    self.record_private_balance_events("patch", &patch.rows);
                self.upsert_balance_cache(&patch.venue, patch.rows);
                PrivateWsApplyOutcome {
                    account_cache_updated: true,
                    balance_ledger_updated,
                    ..PrivateWsApplyOutcome::default()
                }
            }
            PrivateWsEvent::AssetValuations(snapshot) => {
                self.replace_asset_valuations(&snapshot.venue, snapshot.rows);
                PrivateWsApplyOutcome {
                    account_cache_updated: true,
                    ..PrivateWsApplyOutcome::default()
                }
            }
            PrivateWsEvent::AccountSummary(summary) => {
                self.record_account_summaries(vec![summary]);
                PrivateWsApplyOutcome {
                    account_cache_updated: true,
                    ..PrivateWsApplyOutcome::default()
                }
            }
            PrivateWsEvent::AccountDirty(dirty) => {
                let dirty = self.mark_private_event_account_dirty(dirty);
                PrivateWsApplyOutcome {
                    account_cache_dirty: Some(dirty),
                    ..PrivateWsApplyOutcome::default()
                }
            }
        }
    }

    fn apply_private_ws_fill(
        &self,
        delta: &PrivateFillDelta,
        transport_metadata: Option<&OrderTransportMetadata>,
    ) -> PrivateWsApplyOutcome {
        let ledger_events = self.record_private_fill_delta(delta, transport_metadata);
        let dirty = self.mark_private_event_account_dirty(PrivateAccountDirty::new(
            &delta.venue,
            PrivateAccountScope::All,
            "fill_event",
        ));
        PrivateWsApplyOutcome {
            ledger_updated: !ledger_events.is_empty(),
            ledger_events,
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        }
    }

    fn apply_private_ws_liquidation(
        &self,
        delta: &PrivateLiquidationDelta,
    ) -> PrivateWsApplyOutcome {
        tracing::warn!(
            venue_event_id = %delta.venue_event_id,
            liquidator = %delta.liquidator,
            liquidated_user = %delta.liquidated_user,
            notional_position = delta.notional_position,
            account_value = delta.account_value,
            occurred_at_ms = delta.occurred_at_ms,
            venue = %delta.venue,
            "private WS liquidation event marked scoped account cache stale"
        );
        let dirty = self.mark_private_event_account_dirty(PrivateAccountDirty::new(
            &delta.venue,
            PrivateAccountScope::All,
            "liquidation_event",
        ));
        PrivateWsApplyOutcome {
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        }
    }

    fn apply_private_ws_non_user_cancel(
        &self,
        delta: &PrivateNonUserCancelDelta,
    ) -> PrivateWsApplyOutcome {
        let exchange_order_id = delta.exchange_order_id.trim();
        if exchange_order_id.is_empty() {
            self.mark_open_order_cache_stale(&delta.venue);
            return PrivateWsApplyOutcome::default();
        }
        if !self.journal.get_by_exchange_order_id(exchange_order_id).is_some_and(|record| {
            record.intent.exchange == delta.venue && self.private_record_account_matches(&record)
        }) { return PrivateWsApplyOutcome::default(); }
        let message = format!(
            "hyperliquid nonUserCancel: coin={}, event={}",
            delta.coin, delta.venue_event_id
        );
        let updated = self.journal.update_state_by_exchange_order_id_from_source(
            exchange_order_id,
            shared_types::LiveOrderState::Cancelled,
            Some(message),
            delta.occurred_at_ms,
            OrderUpdateSource::PrivateWs,
        );
        if let Some(record) = updated.as_ref() {
            self.live_order_proof_health
                .record_private_ws_cancel_finality_from_record(
                    record,
                    "private_ws_non_user_cancel",
                );
        }
        let open_order_cache_updated =
            self.remove_open_order_cache(&delta.venue, exchange_order_id);
        PrivateWsApplyOutcome {
            order: updated,
            open_order_cache_updated,
            ..PrivateWsApplyOutcome::default()
        }
    }

    fn record_private_fill_delta(
        &self,
        delta: &PrivateFillDelta,
        transport_metadata: Option<&OrderTransportMetadata>,
    ) -> Vec<ExecutionLedgerEvent> {
        let exchange_order_id = clean_identity_text(&delta.exchange_order_id);
        let client_order_id = delta
            .client_order_id
            .as_deref()
            .and_then(clean_identity_text);
        if exchange_order_id.is_none() && client_order_id.is_none() {
            return Vec::new();
        }
        let input = FillLedgerInput {
            venue_event_id: delta.venue_event_id.clone(),
            quantity: delta.quantity,
            price: delta.price,
            fee_amount: delta.fee_amount,
            fee_currency: delta.fee_currency.clone(),
            occurred_at_ms: delta.occurred_at_ms,
        };
        let identity = FillOrderIdentity {
            venue: Some(&delta.venue),
            exchange_order_id,
            client_order_id,
            symbol: delta.symbol.as_deref(),
            side: delta.side,
        };
        if !self.journal.fill_record_for_identity(&identity).is_some_and(|record| self.private_record_account_matches(&record)) {
            return Vec::new();
        }
        let captured_at_ms = common::time::now_ms();
        match transport_metadata {
            Some(metadata) => self
                .journal
                .record_fill_by_order_identity_with_metadata_deferred_sql(
                    identity,
                    &input,
                    OrderUpdateSource::PrivateWs,
                    captured_at_ms,
                    metadata,
                ),
            None => self.journal.record_fill_by_order_identity_deferred_sql(
                identity,
                &input,
                OrderUpdateSource::PrivateWs,
                captured_at_ms,
            ),
        }
    }
    async fn apply_private_ws_order(&self, delta: PrivateOrderDelta) -> PrivateWsApplyOutcome {
        let open_order_cache_updated = self.apply_open_order_cache(&delta.order);
        let updated = self.apply_private_order_delta(&delta);
        if let Some(record) = updated.as_ref() {
            self.live_order_proof_health
                .record_private_ws_cancel_finality_from_record(record, "private_ws_order");
        }
        let account_changed = updated.as_ref().is_some_and(|record| {
            crate::trading_service::filled_quantity_changes_account_state(record.filled_quantity)
        }) || crate::trading_service::order_info_changes_account_state(
            &delta.order,
        );
        if account_changed {
            let venue = updated
                .as_ref()
                .map_or(delta.order.exchange.as_str(), |record| {
                    record.intent.exchange.as_str()
                });
            let dirty = self.mark_private_event_account_dirty(PrivateAccountDirty::new(
                venue,
                PrivateAccountScope::All,
                "terminal_order_update",
            ));
            PrivateWsApplyOutcome {
                order: updated,
                open_order_cache_updated,
                account_cache_dirty: Some(dirty),
                ..PrivateWsApplyOutcome::default()
            }
        } else {
            PrivateWsApplyOutcome {
                order: updated,
                open_order_cache_updated,
                ..PrivateWsApplyOutcome::default()
            }
        }
    }

    fn apply_private_order_delta(&self, delta: &PrivateOrderDelta) -> Option<OrderRecord> {
        self.private_order_record(delta)?;
        if !delta.client_order_id.is_empty() {
            if let Some(record) = self
                .journal
                .apply_order_info_by_client_order_id_from_source(
                    &delta.client_order_id,
                    &delta.order,
                    delta.received_at_ms,
                    OrderUpdateSource::PrivateWs,
                )
            {
                return Some(record);
            }
        }
        if delta.order.order_id.is_empty() {
            return None;
        }
        self.journal
            .apply_order_info_by_exchange_order_id_from_source(
                &delta.order.order_id,
                &delta.order,
                delta.received_at_ms,
                OrderUpdateSource::PrivateWs,
            )
    }
}

fn clean_identity_text(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}
