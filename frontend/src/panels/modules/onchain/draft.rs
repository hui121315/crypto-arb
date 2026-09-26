use leptos::prelude::*;
use shared_types::{
    onchain_chain_preset, onchain_quote_provider, onchain_quote_provider_supported,
    onchain_quote_providers_independent, OnchainComparisonConfig, OnchainComparisonConfigPatch,
    OnchainComparisonSnapshot, OnchainCrossChainConfigPatch, OnchainDexComparisonConfigPatch,
    OnchainRpcMode, OnchainSourceConfigPatch, OnchainSpreadAlertConfigPatch,
    OnchainSpreadAlertMode, OnchainTokenIdentity, ONCHAIN_QUOTE_PROVIDERS,
};

use crate::state::load_state::LoadState;

#[path = "draft/values.rs"]
mod values;

use values::{decimal_to_raw_units, decimal_units, percent_input, percent_to_bps, raw_units};
pub(super) use values::{explicit_pair_assets, normalized_asset};

#[derive(Clone, Copy)]
pub(super) struct OnchainConfigDraft {
    pub chain: RwSignal<String>,
    pub provider: RwSignal<String>,
    pub dex_compare_enabled: RwSignal<bool>,
    pub peer_provider: RwSignal<String>,
    pub cross_chain_enabled: RwSignal<bool>,
    pub cross_chain_peer_item_id: RwSignal<String>,
    pub pool_or_route: RwSignal<String>,
    pub base_token: RwSignal<String>,
    pub quote_token: RwSignal<String>,
    pub base_identity_resolved: RwSignal<bool>,
    pub quote_identity_resolved: RwSignal<bool>,
    pub venue: RwSignal<String>,
    pub symbol: RwSignal<String>,
    pub base_mint: RwSignal<String>,
    pub quote_mint: RwSignal<String>,
    pub base_decimals: RwSignal<String>,
    pub quote_decimals: RwSignal<String>,
    pub base_amount: RwSignal<String>,
    pub quote_amount: RwSignal<String>,
    pub wallet_address: RwSignal<String>,
    pub cex_fee: RwSignal<String>,
    pub gas_usd: RwSignal<String>,
    pub slippage: RwSignal<String>,
    pub min_liquidity: RwSignal<String>,
    pub max_age: RwSignal<String>,
    pub rpc_mode: RwSignal<OnchainRpcMode>,
    pub custom_rpc_url: RwSignal<String>,
    pub alert_enabled: RwSignal<bool>,
    pub alert_mode: RwSignal<OnchainSpreadAlertMode>,
    pub alert_threshold: RwSignal<String>,
    pub alert_raw_threshold: RwSignal<String>,
    pub alert_cooldown: RwSignal<String>,
}

impl OnchainConfigDraft {
    pub(super) fn new(state: RwSignal<LoadState<OnchainComparisonSnapshot>>) -> Self {
        let draft = Self::from_config(&OnchainComparisonConfig::default());
        let hydrated = RwSignal::new(false);
        Effect::new(move |_| {
            if hydrated.get_untracked() {
                return;
            }
            let Some(snapshot) = state.with(|state| state.value().cloned()) else {
                return;
            };
            draft.hydrate(&snapshot);
            hydrated.set(true);
        });
        Effect::new(move |_| {
            let Some(snapshot) = state.with(|state| state.value().cloned()) else {
                return;
            };
            let config = snapshot.config;
            if !untrack(|| draft.matches_applied_config(&config)) {
                return;
            }
            draft.sync_runtime_identity(&config);
        });
        draft
    }

    pub(super) fn from_config(config: &OnchainComparisonConfig) -> Self {
        Self {
            chain: RwSignal::new(config.chain.clone()),
            provider: RwSignal::new(config.provider.clone()),
            dex_compare_enabled: RwSignal::new(config.dex_comparison.enabled),
            peer_provider: RwSignal::new(config.dex_comparison.peer_provider.clone()),
            cross_chain_enabled: RwSignal::new(config.cross_chain.enabled),
            cross_chain_peer_item_id: RwSignal::new(config.cross_chain.peer_item_id.clone()),
            pool_or_route: RwSignal::new(config.pool_or_route.clone()),
            base_token: RwSignal::new(config.base_token.clone()),
            quote_token: RwSignal::new(config.quote_token.clone()),
            base_identity_resolved: RwSignal::new(config.base_identity_resolved),
            quote_identity_resolved: RwSignal::new(config.quote_identity_resolved),
            venue: RwSignal::new(config.cex_venue.clone()),
            symbol: RwSignal::new(config.cex_symbol.clone()),
            base_mint: RwSignal::new(config.base_mint.clone()),
            quote_mint: RwSignal::new(config.quote_mint.clone()),
            base_decimals: RwSignal::new(config.base_decimals.to_string()),
            quote_decimals: RwSignal::new(config.quote_decimals.to_string()),
            base_amount: RwSignal::new(config.base_amount_raw.clone()),
            quote_amount: RwSignal::new(decimal_units(
                &config.quote_amount_raw,
                config.quote_decimals,
            )),
            wallet_address: RwSignal::new(config.wallet_address.clone()),
            cex_fee: RwSignal::new(percent_input(config.cex_taker_fee_bps)),
            gas_usd: RwSignal::new(config.gas_usd.to_string()),
            slippage: RwSignal::new(percent_input(config.slippage_bps)),
            min_liquidity: RwSignal::new(config.min_liquidity_usd.to_string()),
            max_age: RwSignal::new(config.max_age_ms.to_string()),
            rpc_mode: RwSignal::new(config.rpc.mode),
            custom_rpc_url: RwSignal::new(String::new()),
            alert_enabled: RwSignal::new(config.spread_alert.enabled),
            alert_mode: RwSignal::new(config.spread_alert.mode),
            alert_threshold: RwSignal::new(percent_input(config.spread_alert.min_net_spread_bps)),
            alert_raw_threshold: RwSignal::new(percent_input(
                config.spread_alert.min_raw_spread_bps,
            )),
            alert_cooldown: RwSignal::new((config.spread_alert.cooldown_ms / 1_000).to_string()),
        }
    }

    pub(super) fn apply_chain_preset(self, chain: &str) {
        let Some(preset) = onchain_chain_preset(chain) else {
            return;
        };
        self.chain.set(preset.id.to_owned());
        self.provider.set(preset.provider.to_owned());
        self.ensure_peer_provider();
        self.pool_or_route.set(preset.route.to_owned());
        self.base_token.set(preset.base_token.to_owned());
        self.quote_token.set(preset.quote_token.to_owned());
        self.base_identity_resolved.set(true);
        self.quote_identity_resolved.set(true);
        self.base_mint.set(preset.base_address.to_owned());
        self.quote_mint.set(preset.quote_address.to_owned());
        self.base_decimals.set(preset.base_decimals.to_string());
        self.quote_decimals.set(preset.quote_decimals.to_string());
        self.base_amount.set(preset.base_amount_raw.to_owned());
        self.quote_amount.set(decimal_units(
            preset.quote_amount_raw,
            preset.quote_decimals,
        ));
        self.wallet_address.set(String::new());
        if preset.chain_id.is_none() {
            self.rpc_mode.set(OnchainRpcMode::ProviderManaged);
            self.custom_rpc_url.set(String::new());
        }
    }

    pub(super) fn apply_provider(self, provider: &str) {
        let Some(provider) = onchain_quote_provider(provider) else {
            return;
        };
        self.provider.set(provider.id.to_owned());
        self.pool_or_route.set(provider.default_route.to_owned());
        self.ensure_peer_provider();
    }

    pub(super) fn apply_peer_provider(self, provider: &str) {
        if onchain_quote_provider_supported(provider, &self.chain.get_untracked())
            && onchain_quote_providers_independent(&self.provider.get_untracked(), provider)
        {
            self.peer_provider.set(provider.to_owned());
        }
    }

    pub(super) fn set_dex_compare_enabled(self, enabled: bool) {
        self.dex_compare_enabled.set(enabled);
        if enabled {
            self.ensure_peer_provider();
            if self.peer_provider.get_untracked().is_empty() {
                self.dex_compare_enabled.set(false);
            }
        }
    }

    pub(super) fn set_cross_chain_enabled(self, enabled: bool) {
        self.cross_chain_enabled.set(enabled);
        if !enabled {
            return;
        }
        if self
            .cross_chain_peer_item_id
            .get_untracked()
            .trim()
            .is_empty()
        {
            self.cross_chain_enabled.set(false);
        }
    }

    pub(super) fn apply_cross_chain_peer(self, item_id: &str) {
        self.cross_chain_peer_item_id.set(item_id.trim().to_owned());
        if item_id.trim().is_empty() {
            self.cross_chain_enabled.set(false);
        }
    }

    pub(super) fn apply_venue(self, venue: &str) {
        if self.venue.get_untracked().eq_ignore_ascii_case(venue) {
            return;
        }
        self.venue.set(venue.to_owned());
        self.symbol.set(String::new());
    }

    pub(super) fn apply_cex_symbol(self, symbol: &str) {
        let symbol = symbol.to_ascii_uppercase();
        if !self.base_identity_resolved.get_untracked() {
            if let Some((base, _)) = explicit_pair_assets(&symbol) {
                self.base_token.set(base);
            }
        }
        self.symbol.set(symbol);
    }

    pub(super) fn normalize_tokens(self) {
        let base = self.base_token.get_untracked().trim().to_ascii_uppercase();
        let quote = self.quote_token.get_untracked().trim().to_ascii_uppercase();
        self.base_token.set(base);
        self.quote_token.set(quote);
    }

    pub(super) fn apply_base_identity(self, identity: &OnchainTokenIdentity) {
        self.base_mint.set(identity.address.clone());
        self.base_token.set(identity.symbol.clone());
        self.base_identity_resolved.set(identity.verified);
        self.base_decimals.set(identity.decimals.to_string());
        self.base_amount.set(raw_units(identity.decimals, 1));
        self.normalize_tokens();
    }

    pub(super) fn apply_quote_identity(self, identity: &OnchainTokenIdentity) {
        self.quote_mint.set(identity.address.clone());
        self.quote_token.set(identity.symbol.clone());
        self.quote_identity_resolved.set(identity.verified);
        self.quote_decimals.set(identity.decimals.to_string());
        if self.quote_amount.get_untracked().trim().is_empty() {
            self.quote_amount.set("100".to_owned());
        }
        self.normalize_tokens();
    }

    pub(super) fn apply_base_precision(self, address: &str, decimals: u8) {
        self.base_token.set(abbreviated_contract(address));
        self.base_identity_resolved.set(false);
        self.base_decimals.set(decimals.to_string());
        self.base_amount.set(raw_units(decimals, 1));
    }

    pub(super) fn apply_quote_precision(self, address: &str, decimals: u8) {
        self.quote_token.set(abbreviated_contract(address));
        self.quote_identity_resolved.set(false);
        self.quote_decimals.set(decimals.to_string());
        if self.quote_amount.get_untracked().trim().is_empty() {
            self.quote_amount.set("100".to_owned());
        }
    }

    pub(super) fn clear_base_identity(self) {
        self.base_mint.set(String::new());
        self.base_token.set(String::new());
        self.base_identity_resolved.set(false);
        self.base_decimals.set(String::new());
        self.base_amount.set(String::new());
        self.normalize_tokens();
    }

    pub(super) fn invalidate_base_identity(self) {
        self.base_token.set(String::new());
        self.base_identity_resolved.set(false);
        self.base_decimals.set(String::new());
        self.base_amount.set(String::new());
    }

    pub(super) fn clear_quote_identity(self) {
        self.quote_mint.set(String::new());
        self.quote_token.set(String::new());
        self.quote_identity_resolved.set(false);
        self.quote_decimals.set(String::new());
        self.quote_amount.set(String::new());
        self.normalize_tokens();
    }

    pub(super) fn invalidate_quote_identity(self) {
        self.quote_token.set(String::new());
        self.quote_identity_resolved.set(false);
        self.quote_decimals.set(String::new());
        self.quote_amount.set(String::new());
    }

    pub(super) fn patch(self) -> OnchainComparisonConfigPatch {
        let custom_rpc_url = self.custom_rpc_url.get_untracked();
        OnchainComparisonConfigPatch {
            chain: Some(self.chain.get_untracked()),
            pool_or_route: Some(self.pool_or_route.get_untracked()),
            base_token: Some(self.base_token.get_untracked()),
            quote_token: Some(self.quote_token.get_untracked()),
            base_identity_resolved: Some(self.base_identity_resolved.get_untracked()),
            quote_identity_resolved: Some(self.quote_identity_resolved.get_untracked()),
            cex_venue: Some(self.venue.get_untracked()),
            cex_symbol: Some(self.symbol.get_untracked()),
            base_mint: Some(self.base_mint.get_untracked()),
            quote_mint: Some(self.quote_mint.get_untracked()),
            base_decimals: self.base_decimals.get_untracked().parse().ok(),
            quote_decimals: self.quote_decimals.get_untracked().parse().ok(),
            base_amount_raw: Some(self.base_amount.get_untracked()),
            quote_amount_raw: self.quote_amount_raw(),
            wallet_address: Some(self.wallet_address.get_untracked()),
            cex_taker_fee_bps: percent_to_bps(&self.cex_fee.get_untracked()),
            gas_usd: self.gas_usd.get_untracked().parse().ok(),
            slippage_bps: percent_to_bps(&self.slippage.get_untracked()),
            min_liquidity_usd: self.min_liquidity.get_untracked().parse().ok(),
            max_age_ms: self.max_age.get_untracked().parse().ok(),
            source: Some(OnchainSourceConfigPatch {
                provider: Some(self.provider.get_untracked()),
                rpc_mode: Some(self.rpc_mode.get_untracked()),
                custom_rpc_url: (!custom_rpc_url.trim().is_empty()).then_some(custom_rpc_url),
            }),
            spread_alert: Some(OnchainSpreadAlertConfigPatch {
                enabled: Some(self.alert_enabled.get_untracked()),
                mode: Some(self.alert_mode.get_untracked()),
                min_net_spread_bps: percent_to_bps(&self.alert_threshold.get_untracked()),
                min_raw_spread_bps: percent_to_bps(&self.alert_raw_threshold.get_untracked()),
                cooldown_ms: self
                    .alert_cooldown
                    .get_untracked()
                    .parse::<i64>()
                    .ok()
                    .and_then(|seconds| seconds.checked_mul(1_000)),
            }),
            dex_comparison: Some(OnchainDexComparisonConfigPatch {
                enabled: Some(self.dex_compare_enabled.get_untracked()),
                peer_provider: Some(self.peer_provider.get_untracked()),
            }),
            cross_chain: Some(OnchainCrossChainConfigPatch {
                enabled: Some(self.cross_chain_enabled.get_untracked()),
                peer_item_id: Some(self.cross_chain_peer_item_id.get_untracked()),
                provider: Some("lifi".to_owned()),
                stablecoin_risk_bps: None,
            }),
            ..OnchainComparisonConfigPatch::default()
        }
    }

    pub(super) fn matches_applied_config(self, config: &OnchainComparisonConfig) -> bool {
        self.chain.get().eq_ignore_ascii_case(&config.chain)
            && self.provider.get().eq_ignore_ascii_case(&config.provider)
            && self.pool_or_route.get().trim() == config.pool_or_route
            && self
                .base_token
                .get()
                .eq_ignore_ascii_case(&config.base_token)
            && self
                .quote_token
                .get()
                .eq_ignore_ascii_case(&config.quote_token)
            && self.base_identity_resolved.get() == config.base_identity_resolved
            && self.quote_identity_resolved.get() == config.quote_identity_resolved
            && self.base_mint.get().trim() == config.base_mint
            && self.quote_mint.get().trim() == config.quote_mint
            && self.base_decimals.get().parse().ok() == Some(config.base_decimals)
            && self.quote_decimals.get().parse().ok() == Some(config.quote_decimals)
            && self.base_amount.get().trim() == config.base_amount_raw
            && self.quote_amount_raw().as_deref() == Some(config.quote_amount_raw.as_str())
            && self.wallet_address.get().trim() == config.wallet_address
            && self.venue.get().eq_ignore_ascii_case(&config.cex_venue)
            && self.symbol.get().eq_ignore_ascii_case(&config.cex_symbol)
            && numeric_input_matches(&self.cex_fee.get(), config.cex_taker_fee_bps / 100.0)
            && numeric_input_matches(&self.gas_usd.get(), config.gas_usd)
            && numeric_input_matches(&self.slippage.get(), config.slippage_bps / 100.0)
            && numeric_input_matches(&self.min_liquidity.get(), config.min_liquidity_usd)
            && self.max_age.get().parse().ok() == Some(config.max_age_ms)
            && self.rpc_mode.get() == config.rpc.mode
            && self.dex_comparison_matches(config)
            && self.cross_chain_matches(config)
            && self.alert_matches_applied_config(config)
    }

    pub(super) fn matches_batch_config(self, config: &OnchainComparisonConfig) -> bool {
        self.chain.get().eq_ignore_ascii_case(&config.chain)
            && self.provider.get().eq_ignore_ascii_case(&config.provider)
            && self.pool_or_route.get().trim() == config.pool_or_route
            && self
                .base_token
                .get()
                .eq_ignore_ascii_case(&config.base_token)
            && self
                .quote_token
                .get()
                .eq_ignore_ascii_case(&config.quote_token)
            && self.base_identity_resolved.get() == config.base_identity_resolved
            && self.quote_identity_resolved.get() == config.quote_identity_resolved
            && self.base_mint.get().trim() == config.base_mint
            && self.quote_mint.get().trim() == config.quote_mint
            && self.base_decimals.get().parse().ok() == Some(config.base_decimals)
            && self.quote_decimals.get().parse().ok() == Some(config.quote_decimals)
            && self.base_amount.get().trim() == config.base_amount_raw
            && self.quote_amount_raw().as_deref() == Some(config.quote_amount_raw.as_str())
            && self.venue.get().eq_ignore_ascii_case(&config.cex_venue)
            && self.symbol.get().eq_ignore_ascii_case(&config.cex_symbol)
            && numeric_input_matches(&self.cex_fee.get(), config.cex_taker_fee_bps / 100.0)
            && numeric_input_matches(&self.gas_usd.get(), config.gas_usd)
            && numeric_input_matches(&self.slippage.get(), config.slippage_bps / 100.0)
            && numeric_input_matches(&self.min_liquidity.get(), config.min_liquidity_usd)
            && self.max_age.get().parse().ok() == Some(config.max_age_ms)
            && self.dex_comparison_matches(config)
            && self.cross_chain_matches(config)
            && self.alert_matches_applied_config(config)
    }

    pub(super) fn alert_matches_applied_config(self, config: &OnchainComparisonConfig) -> bool {
        self.alert_enabled.get() == config.spread_alert.enabled
            && self.alert_mode.get() == config.spread_alert.mode
            && numeric_input_matches(
                &self.alert_threshold.get(),
                config.spread_alert.min_net_spread_bps / 100.0,
            )
            && numeric_input_matches(
                &self.alert_raw_threshold.get(),
                config.spread_alert.min_raw_spread_bps / 100.0,
            )
            && self
                .alert_cooldown
                .get()
                .parse::<i64>()
                .ok()
                .and_then(|seconds| seconds.checked_mul(1_000))
                == Some(config.spread_alert.cooldown_ms)
    }

    fn dex_comparison_matches(self, config: &OnchainComparisonConfig) -> bool {
        self.dex_compare_enabled.get() == config.dex_comparison.enabled
            && self
                .peer_provider
                .get()
                .eq_ignore_ascii_case(&config.dex_comparison.peer_provider)
    }

    fn cross_chain_matches(self, config: &OnchainComparisonConfig) -> bool {
        self.cross_chain_enabled.get() == config.cross_chain.enabled
            && self.cross_chain_peer_item_id.get().trim() == config.cross_chain.peer_item_id
    }

    fn hydrate(self, snapshot: &OnchainComparisonSnapshot) {
        self.load_config(&snapshot.config);
    }

    pub(super) fn load_config(self, config: &OnchainComparisonConfig) {
        self.chain.set(config.chain.clone());
        self.provider.set(config.provider.clone());
        self.dex_compare_enabled.set(config.dex_comparison.enabled);
        self.peer_provider
            .set(config.dex_comparison.peer_provider.clone());
        self.cross_chain_enabled.set(config.cross_chain.enabled);
        self.cross_chain_peer_item_id
            .set(config.cross_chain.peer_item_id.clone());
        self.pool_or_route.set(config.pool_or_route.clone());
        self.base_token.set(config.base_token.clone());
        self.quote_token.set(config.quote_token.clone());
        self.base_identity_resolved
            .set(config.base_identity_resolved);
        self.quote_identity_resolved
            .set(config.quote_identity_resolved);
        self.venue.set(config.cex_venue.clone());
        self.symbol.set(config.cex_symbol.clone());
        self.base_mint.set(config.base_mint.clone());
        self.quote_mint.set(config.quote_mint.clone());
        self.base_decimals.set(config.base_decimals.to_string());
        self.quote_decimals.set(config.quote_decimals.to_string());
        self.base_amount.set(config.base_amount_raw.clone());
        self.quote_amount.set(decimal_units(
            &config.quote_amount_raw,
            config.quote_decimals,
        ));
        self.wallet_address.set(config.wallet_address.clone());
        self.cex_fee.set(percent_input(config.cex_taker_fee_bps));
        self.gas_usd.set(config.gas_usd.to_string());
        self.slippage.set(percent_input(config.slippage_bps));
        self.min_liquidity.set(config.min_liquidity_usd.to_string());
        self.max_age.set(config.max_age_ms.to_string());
        self.rpc_mode.set(config.rpc.mode);
        self.custom_rpc_url.set(String::new());
        self.alert_enabled.set(config.spread_alert.enabled);
        self.alert_mode.set(config.spread_alert.mode);
        self.alert_threshold
            .set(percent_input(config.spread_alert.min_net_spread_bps));
        self.alert_raw_threshold
            .set(percent_input(config.spread_alert.min_raw_spread_bps));
        self.alert_cooldown
            .set((config.spread_alert.cooldown_ms / 1_000).to_string());
    }

    fn sync_runtime_identity(self, config: &OnchainComparisonConfig) {
        set_string_if_changed(self.symbol, &config.cex_symbol);
        set_string_if_changed(self.base_mint, &config.base_mint);
        set_string_if_changed(self.quote_mint, &config.quote_mint);
        set_string_if_changed(self.base_decimals, &config.base_decimals.to_string());
        set_string_if_changed(self.quote_decimals, &config.quote_decimals.to_string());
        set_string_if_changed(self.base_amount, &config.base_amount_raw);
        set_string_if_changed(
            self.quote_amount,
            &decimal_units(&config.quote_amount_raw, config.quote_decimals),
        );
    }

    pub(super) fn quote_amount_raw(self) -> Option<String> {
        let decimals = self.quote_decimals.get_untracked().parse::<u8>().ok()?;
        decimal_to_raw_units(&self.quote_amount.get_untracked(), decimals)
    }

    fn ensure_peer_provider(self) {
        let chain = self.chain.get_untracked();
        let primary = self.provider.get_untracked();
        let current = self.peer_provider.get_untracked();
        if onchain_quote_provider_supported(&current, &chain)
            && onchain_quote_providers_independent(&primary, &current)
        {
            return;
        }
        let next = ONCHAIN_QUOTE_PROVIDERS
            .iter()
            .find(|provider| {
                onchain_quote_provider_supported(provider.id, &chain)
                    && onchain_quote_providers_independent(&primary, provider.id)
            })
            .map_or_else(String::new, |provider| provider.id.to_owned());
        self.peer_provider.set(next);
    }
}

fn set_string_if_changed(signal: RwSignal<String>, next: &str) {
    if signal.get_untracked() != next {
        signal.set(next.to_owned());
    }
}

fn abbreviated_contract(address: &str) -> String {
    let address = address.trim();
    if address.len() <= 16 {
        return address.to_owned();
    }
    format!("{}..{}", &address[..8], &address[address.len() - 6..])
}

fn numeric_input_matches(input: &str, expected: f64) -> bool {
    input
        .trim()
        .parse::<f64>()
        .is_ok_and(|value| (value - expected).abs() <= f64::EPSILON * 16.0)
}

#[cfg(test)]
mod tests;
