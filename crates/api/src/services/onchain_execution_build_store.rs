use dashmap::DashMap;
use shared_types::{OnchainComparisonConfig, OnchainExecutionBuildResponse};

const MAX_STORED_BUILDS: usize = 128;

#[derive(Debug, Clone)]
struct StoredBuild {
    response: OnchainExecutionBuildResponse,
    config: OnchainComparisonConfig,
    claimed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedOnchainBuild {
    pub(crate) response: OnchainExecutionBuildResponse,
    pub(crate) config: OnchainComparisonConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BuildClaimError {
    Missing,
    Expired,
    AlreadyClaimed(String),
}

#[derive(Debug, Default)]
pub(crate) struct OnchainExecutionBuildStore {
    builds: DashMap<String, StoredBuild>,
}

impl OnchainExecutionBuildStore {
    pub(crate) fn insert(
        &self,
        response: OnchainExecutionBuildResponse,
        config: OnchainComparisonConfig,
        now_ms: i64,
    ) {
        self.prune(now_ms);
        self.builds.insert(
            response.build_id.clone(),
            StoredBuild {
                response,
                config,
                claimed_by: None,
            },
        );
        if self.builds.len() > MAX_STORED_BUILDS {
            self.remove_oldest();
        }
    }

    pub(crate) fn claim(
        &self,
        build_id: &str,
        run_id: &str,
        now_ms: i64,
    ) -> Result<ClaimedOnchainBuild, BuildClaimError> {
        let mut entry = self
            .builds
            .get_mut(build_id)
            .ok_or(BuildClaimError::Missing)?;
        if entry.response.valid_until_ms < now_ms {
            drop(entry);
            self.builds.remove(build_id);
            return Err(BuildClaimError::Expired);
        }
        if let Some(existing) = entry.claimed_by.as_ref() {
            return Err(BuildClaimError::AlreadyClaimed(existing.clone()));
        }
        entry.claimed_by = Some(run_id.to_owned());
        Ok(ClaimedOnchainBuild {
            response: entry.response.clone(),
            config: entry.config.clone(),
        })
    }

    pub(crate) fn release(&self, build_id: &str, run_id: &str) {
        let Some(mut entry) = self.builds.get_mut(build_id) else {
            return;
        };
        if entry.claimed_by.as_deref() == Some(run_id) {
            entry.claimed_by = None;
        }
    }

    pub(crate) fn finish(&self, build_id: &str, run_id: &str) {
        let remove = self
            .builds
            .get(build_id)
            .is_some_and(|entry| entry.claimed_by.as_deref() == Some(run_id));
        if remove {
            self.builds.remove(build_id);
        }
    }

    fn prune(&self, now_ms: i64) {
        self.builds
            .retain(|_, entry| entry.response.valid_until_ms >= now_ms);
    }

    fn remove_oldest(&self) {
        let oldest = self
            .builds
            .iter()
            .min_by_key(|entry| entry.response.built_at_ms)
            .map(|entry| entry.key().clone());
        if let Some(build_id) = oldest {
            self.builds.remove(&build_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use shared_types::{
        InstrumentAssetClass, InstrumentListingStatus, InstrumentMetadataSource,
        OnchainCexOrderPlan, OnchainComparisonDirection, OnchainUnsignedTransaction, OrderSide,
        OrderSizingPlan, VenueInstrument,
    };

    use super::*;

    fn build(id: &str, built_at_ms: i64, valid_until_ms: i64) -> OnchainExecutionBuildResponse {
        OnchainExecutionBuildResponse {
            build_id: id.to_owned(),
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            provider: "jupiter_swap_v2".to_owned(),
            chain: "solana".to_owned(),
            wallet_address: "wallet".to_owned(),
            input_token: "quote".to_owned(),
            output_token: "base".to_owned(),
            input_amount_raw: "1".to_owned(),
            output_amount_raw: "1".to_owned(),
            chain_transaction: OnchainUnsignedTransaction::SolanaVersioned {
                transaction_base64: "tx".to_owned(),
                request_id: "request".to_owned(),
                router: "router".to_owned(),
                mode: "fast".to_owned(),
                last_valid_block_height: None,
                expire_at_ms: None,
            },
            minimum_output_amount_raw: None,
            chain_input_adjustment: None,
            settlement_assets: None,
            cex_order: OnchainCexOrderPlan {
                venue: "kraken".to_owned(),
                native_symbol: "SOL/USD".to_owned(),
                client_order_id: "client".to_owned(),
                side: OrderSide::Sell,
                base_quantity: 1.0,
                reference_price: 100.0,
                estimated_quote_amount: 100.0,
                instrument_spec: instrument(),
                sizing_plan: OrderSizingPlan::default(),
            },
            quote_conversion_order: None,
            quote_usd_valuation: None,
            replenishment_costs: Vec::new(),
            approval_costs: Vec::new(),
            estimated_net_profit_usd: 1.0,
            estimated_net_spread_bps: 100.0,
            quote_observed_at_ms: built_at_ms,
            cex_observed_at_ms: built_at_ms,
            built_at_ms,
            valid_until_ms,
            official_docs_url: "docs".to_owned(),
            build_ready: true,
            submit_ready: false,
            blockers: Vec::new(),
        }
    }

    fn instrument() -> VenueInstrument {
        VenueInstrument {
            venue: "kraken".to_owned(),
            native_symbol: "SOL/USD".to_owned(),
            canonical_symbol: "SOL".to_owned(),
            display_symbol: "SOL/USD".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("spot".to_owned()),
            quote_asset: Some("USD".to_owned()),
            settle_asset: None,
            margin_asset: None,
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.001),
            qty_step: Some(0.001),
            min_qty: Some(0.001),
            min_notional: Some(1.0),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: None,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("https://api.kraken.com/0/public/AssetPairs".to_owned()),
            checked_at_ms: 1,
            schema_version: Some("kraken-spot-v1".to_owned()),
        }
    }

    #[test]
    fn claim_is_atomic_one_time_and_expiry_is_fail_closed() {
        let store = OnchainExecutionBuildStore::default();
        store.insert(
            build("ready", 10, 20),
            OnchainComparisonConfig::default(),
            10,
        );
        assert!(store.claim("ready", "run-a", 15).is_ok());
        assert_eq!(
            store.claim("ready", "run-b", 15),
            Err(BuildClaimError::AlreadyClaimed("run-a".to_owned()))
        );
        store.release("ready", "run-a");
        assert!(store.claim("ready", "run-b", 15).is_ok());
        store.finish("ready", "run-b");
        assert_eq!(
            store.claim("ready", "run-c", 15),
            Err(BuildClaimError::Missing)
        );

        store.insert(
            build("expired", 20, 25),
            OnchainComparisonConfig::default(),
            20,
        );
        assert_eq!(
            store.claim("expired", "run", 26),
            Err(BuildClaimError::Expired)
        );
    }
}
