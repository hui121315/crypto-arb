use arc_swap::{ArcSwap, ArcSwapOption};
use shared_types::{
    onchain_chain_preset, onchain_quote_provider, onchain_quote_provider_supported,
    onchain_quote_providers_independent, OnchainComparisonConfig, OnchainComparisonConfigPatch,
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainRpcMode, OnchainRpcStatus,
    ONCHAIN_CEX_VENUES, ONCHAIN_QUOTE_PROVIDERS,
};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use url::Url;

use crate::{
    OnchainBatchMonitor, OnchainCrossChainQuoteSet, OnchainDexCrossQuoteSet, OnchainQuotePair,
    OnchainWalletInventory,
};

#[derive(Debug)]
pub struct OnchainMonitor {
    snapshot: ArcSwap<OnchainComparisonSnapshot>,
    quotes: ArcSwapOption<OnchainQuotePair>,
    dex_cross_quotes: ArcSwapOption<OnchainDexCrossQuoteSet>,
    dex_cross_problem: ArcSwapOption<String>,
    dex_cross_in_flight: AtomicBool,
    cross_chain_quotes: ArcSwapOption<OnchainCrossChainQuoteSet>,
    cross_chain_problem: ArcSwapOption<String>,
    cross_chain_in_flight: AtomicBool,
    cross_chain_attempt_started_at_ms: AtomicI64,
    quote_attempt_started_at_ms: AtomicI64,
    wallet_inventory: ArcSwapOption<OnchainWalletInventory>,
    wallet_attempt_started_at_ms: AtomicI64,
    custom_rpc_url: ArcSwapOption<String>,
    rpc_status: ArcSwap<OnchainRpcStatus>,
    batch: OnchainBatchMonitor,
}

impl Default for OnchainMonitor {
    fn default() -> Self {
        Self::from_validated_config(OnchainComparisonConfig::default(), None, 0)
    }
}

impl OnchainMonitor {
    fn from_validated_config(
        config: OnchainComparisonConfig,
        custom_rpc_url: Option<String>,
        now_ms: i64,
    ) -> Self {
        let rpc_status = pending_rpc_status(&config, custom_rpc_url.as_deref());
        let quality = if config.enabled {
            OnchainComparisonQuality::Pending
        } else {
            OnchainComparisonQuality::Disabled
        };
        Self {
            snapshot: ArcSwap::from_pointee(OnchainComparisonSnapshot {
                config,
                quality,
                rpc_status: rpc_status.clone(),
                observed_at_ms: now_ms,
                ..OnchainComparisonSnapshot::default()
            }),
            quotes: ArcSwapOption::empty(),
            dex_cross_quotes: ArcSwapOption::empty(),
            dex_cross_problem: ArcSwapOption::empty(),
            dex_cross_in_flight: AtomicBool::new(false),
            cross_chain_quotes: ArcSwapOption::empty(),
            cross_chain_problem: ArcSwapOption::empty(),
            cross_chain_in_flight: AtomicBool::new(false),
            cross_chain_attempt_started_at_ms: AtomicI64::new(0),
            quote_attempt_started_at_ms: AtomicI64::new(0),
            wallet_inventory: ArcSwapOption::empty(),
            wallet_attempt_started_at_ms: AtomicI64::new(0),
            custom_rpc_url: ArcSwapOption::from(custom_rpc_url.map(Arc::new)),
            rpc_status: ArcSwap::from_pointee(rpc_status),
            batch: OnchainBatchMonitor::default(),
        }
    }

    pub fn from_config(
        config: OnchainComparisonConfig,
        now_ms: i64,
    ) -> Result<Self, OnchainMonitorError> {
        validate(&config, None)?;
        Ok(Self::from_validated_config(config, None, now_ms))
    }

    pub fn from_config_with_custom_rpc(
        config: OnchainComparisonConfig,
        custom_rpc_url: Option<String>,
        now_ms: i64,
    ) -> Result<Self, OnchainMonitorError> {
        validate(&config, custom_rpc_url.as_deref())?;
        Ok(Self::from_validated_config(config, custom_rpc_url, now_ms))
    }

    pub fn snapshot(&self) -> Arc<OnchainComparisonSnapshot> {
        self.snapshot.load_full()
    }

    pub fn preview_config(
        &self,
        patch: &OnchainComparisonConfigPatch,
    ) -> Result<OnchainComparisonConfig, OnchainMonitorError> {
        self.preview_update(patch).map(|(config, _)| config)
    }

    pub fn update_config(
        &self,
        patch: &OnchainComparisonConfigPatch,
        now_ms: i64,
    ) -> Result<Arc<OnchainComparisonSnapshot>, OnchainMonitorError> {
        let previous = self.snapshot();
        let previous_rpc_url = self.custom_rpc_url();
        let (config, custom_rpc_url) = self.preview_update(patch)?;
        let retain_quotes = previous.config.enabled
            && config.enabled
            && same_quote_scope(&previous.config, &config);
        let retain_dex_cross = retain_quotes
            && previous.config.dex_comparison == config.dex_comparison
            && config.dex_comparison.enabled;
        let retain_cross_chain = retain_quotes
            && previous.config.cross_chain == config.cross_chain
            && config.cross_chain.enabled;
        let retain_wallet = self
            .wallet_inventory()
            .is_some_and(|inventory| inventory.matches_config(&config));
        let retain_rpc = same_rpc_scope(
            &previous.config,
            &config,
            previous_rpc_url.as_deref().map(String::as_str),
            custom_rpc_url.as_deref(),
        );
        let rpc_status = if retain_rpc {
            (*self.rpc_status()).clone()
        } else {
            pending_rpc_status(&config, custom_rpc_url.as_deref())
        };
        let quality = if config.enabled {
            OnchainComparisonQuality::Pending
        } else {
            OnchainComparisonQuality::Disabled
        };
        let next = OnchainComparisonSnapshot {
            config,
            quality,
            rpc_status: rpc_status.clone(),
            observed_at_ms: now_ms,
            ..OnchainComparisonSnapshot::default()
        };
        if !retain_quotes {
            self.quotes.store(None);
            self.quote_attempt_started_at_ms.store(0, Ordering::Release);
            self.batch.reset_active_attempt();
        }
        if !retain_dex_cross {
            self.dex_cross_quotes.store(None);
            self.dex_cross_problem.store(None);
        }
        if !retain_cross_chain {
            self.cross_chain_quotes.store(None);
            self.cross_chain_problem.store(None);
            self.cross_chain_attempt_started_at_ms
                .store(0, Ordering::Release);
        }
        if !retain_wallet {
            self.wallet_inventory.store(None);
            self.wallet_attempt_started_at_ms
                .store(0, Ordering::Release);
        }
        self.custom_rpc_url.store(custom_rpc_url.map(Arc::new));
        self.rpc_status.store(Arc::new(rpc_status));
        self.snapshot.store(Arc::new(next));
        Ok(self.snapshot())
    }

    fn preview_update(
        &self,
        patch: &OnchainComparisonConfigPatch,
    ) -> Result<(OnchainComparisonConfig, Option<String>), OnchainMonitorError> {
        let current = self.snapshot();
        let mut config = current.config.clone();
        apply_patch(&mut config, patch);
        let custom_rpc_url = next_custom_rpc_url(&config, patch, self.custom_rpc_url())?;
        validate(&config, custom_rpc_url.as_deref())?;
        Ok((config, custom_rpc_url))
    }

    pub fn publish(&self, snapshot: OnchainComparisonSnapshot) {
        self.snapshot.store(Arc::new(snapshot));
    }

    pub fn quote_pair(&self) -> Option<Arc<OnchainQuotePair>> {
        self.quotes.load_full()
    }

    pub fn publish_quote_pair(&self, quotes: OnchainQuotePair) {
        self.quotes.store(Some(Arc::new(quotes)));
    }

    pub fn dex_cross_quotes(&self) -> Option<Arc<OnchainDexCrossQuoteSet>> {
        self.dex_cross_quotes.load_full()
    }

    pub fn publish_dex_cross_quotes(&self, quotes: OnchainDexCrossQuoteSet) {
        self.dex_cross_quotes.store(Some(Arc::new(quotes)));
        self.dex_cross_problem.store(None);
    }

    pub fn dex_cross_problem(&self) -> Option<Arc<String>> {
        self.dex_cross_problem.load_full()
    }

    pub fn publish_dex_cross_problem(&self, problem: String) {
        self.dex_cross_problem.store(Some(Arc::new(problem)));
    }

    pub fn try_begin_dex_cross_refresh(&self) -> bool {
        self.dex_cross_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn finish_dex_cross_refresh(&self) {
        self.dex_cross_in_flight.store(false, Ordering::Release);
    }

    pub fn cross_chain_quotes(&self) -> Option<Arc<OnchainCrossChainQuoteSet>> {
        self.cross_chain_quotes.load_full()
    }

    pub fn publish_cross_chain_quotes(&self, quotes: OnchainCrossChainQuoteSet) {
        self.cross_chain_quotes.store(Some(Arc::new(quotes)));
        self.cross_chain_problem.store(None);
    }

    pub fn cross_chain_problem(&self) -> Option<Arc<String>> {
        self.cross_chain_problem.load_full()
    }

    pub fn publish_cross_chain_problem(&self, problem: String) {
        self.cross_chain_problem.store(Some(Arc::new(problem)));
    }

    pub fn try_begin_cross_chain_refresh(&self, now_ms: i64, min_interval_ms: i64) -> bool {
        if self
            .cross_chain_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        if !try_begin_attempt(
            &self.cross_chain_attempt_started_at_ms,
            now_ms,
            min_interval_ms,
        ) {
            self.cross_chain_in_flight.store(false, Ordering::Release);
            return false;
        }
        true
    }

    pub fn finish_cross_chain_refresh(&self) {
        self.cross_chain_in_flight.store(false, Ordering::Release);
    }

    pub fn wallet_inventory(&self) -> Option<Arc<OnchainWalletInventory>> {
        self.wallet_inventory.load_full()
    }

    pub fn publish_wallet_inventory(&self, inventory: OnchainWalletInventory) {
        self.wallet_inventory.store(Some(Arc::new(inventory)));
    }

    pub fn try_begin_wallet_attempt(&self, now_ms: i64, min_interval_ms: i64) -> bool {
        try_begin_attempt(&self.wallet_attempt_started_at_ms, now_ms, min_interval_ms)
    }

    pub fn try_begin_quote_attempt(&self, now_ms: i64, min_interval_ms: i64) -> bool {
        try_begin_attempt(&self.quote_attempt_started_at_ms, now_ms, min_interval_ms)
    }

    pub fn custom_rpc_url(&self) -> Option<Arc<String>> {
        self.custom_rpc_url.load_full()
    }

    pub fn rpc_status(&self) -> Arc<OnchainRpcStatus> {
        self.rpc_status.load_full()
    }

    pub fn publish_rpc_status(&self, status: OnchainRpcStatus) {
        self.rpc_status.store(Arc::new(status));
    }

    pub fn batch(&self) -> &OnchainBatchMonitor {
        &self.batch
    }
}

fn same_quote_scope(previous: &OnchainComparisonConfig, next: &OnchainComparisonConfig) -> bool {
    previous.chain.eq_ignore_ascii_case(&next.chain)
        && previous.provider == next.provider
        && previous.pool_or_route == next.pool_or_route
        && previous.base_mint.eq_ignore_ascii_case(&next.base_mint)
        && previous.quote_mint.eq_ignore_ascii_case(&next.quote_mint)
        && previous.base_decimals == next.base_decimals
        && previous.quote_decimals == next.quote_decimals
        && previous.base_amount_raw == next.base_amount_raw
        && previous.quote_amount_raw == next.quote_amount_raw
}

fn same_rpc_scope(
    previous: &OnchainComparisonConfig,
    next: &OnchainComparisonConfig,
    previous_url: Option<&str>,
    next_url: Option<&str>,
) -> bool {
    previous.chain.eq_ignore_ascii_case(&next.chain)
        && previous.rpc == next.rpc
        && previous_url == next_url
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainMonitorError(pub String);

impl std::fmt::Display for OnchainMonitorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for OnchainMonitorError {}

fn try_begin_attempt(attempt: &AtomicI64, now_ms: i64, min_interval_ms: i64) -> bool {
    let mut previous = attempt.load(Ordering::Acquire);
    loop {
        if previous > 0 && now_ms.saturating_sub(previous) < min_interval_ms {
            return false;
        }
        match attempt.compare_exchange_weak(previous, now_ms, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(current) => previous = current,
        }
    }
}

fn apply_patch(config: &mut OnchainComparisonConfig, patch: &OnchainComparisonConfigPatch) {
    if let Some(chain) = patch.chain.as_deref() {
        apply_chain_preset(config, chain);
    }
    apply_identity_patch(config, patch);
    apply_source_patch(config, patch);
    apply_numeric_patch(config, patch);
    apply_spread_alert_patch(config, patch);
    apply_dex_comparison_patch(config, patch);
    apply_cross_chain_patch(config, patch);
}

fn apply_chain_preset(config: &mut OnchainComparisonConfig, chain: &str) {
    let Some(preset) = onchain_chain_preset(chain) else {
        return;
    };
    if !config.chain.eq_ignore_ascii_case(preset.id) {
        config.wallet_address.clear();
    }
    config.chain = preset.id.to_owned();
    config.provider = preset.provider.to_owned();
    config.pool_or_route = preset.route.to_owned();
    config.base_token = preset.base_token.to_owned();
    config.quote_token = preset.quote_token.to_owned();
    config.base_identity_resolved = true;
    config.quote_identity_resolved = true;
    config.base_mint = preset.base_address.to_owned();
    config.quote_mint = preset.quote_address.to_owned();
    config.base_decimals = preset.base_decimals;
    config.quote_decimals = preset.quote_decimals;
    config.base_amount_raw = preset.base_amount_raw.to_owned();
    config.quote_amount_raw = preset.quote_amount_raw.to_owned();
    config.cex_symbol = format!("{}/{}", preset.base_token, preset.quote_token);
    if !onchain_quote_provider_supported(&config.dex_comparison.peer_provider, preset.id)
        || !onchain_quote_providers_independent(
            preset.provider,
            &config.dex_comparison.peer_provider,
        )
    {
        config.dex_comparison.peer_provider = default_peer_provider(preset.provider, preset.id);
    }
    if preset.chain_id.is_none() {
        config.rpc.mode = OnchainRpcMode::ProviderManaged;
    }
}

fn apply_source_patch(config: &mut OnchainComparisonConfig, patch: &OnchainComparisonConfigPatch) {
    let Some(source) = patch.source.as_ref() else {
        return;
    };
    if let Some(provider) = source.provider.as_deref() {
        config.provider = provider.trim().to_ascii_lowercase();
        if let Some(option) = onchain_quote_provider(provider) {
            config.pool_or_route = option.default_route.to_owned();
        }
        if !onchain_quote_providers_independent(
            &config.provider,
            &config.dex_comparison.peer_provider,
        ) || !onchain_quote_provider_supported(
            &config.dex_comparison.peer_provider,
            &config.chain,
        ) {
            config.dex_comparison.peer_provider =
                default_peer_provider(&config.provider, &config.chain);
        }
    }
    if let Some(mode) = source.rpc_mode {
        config.rpc.mode = mode;
    }
}

fn apply_identity_patch(
    config: &mut OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
) {
    macro_rules! apply {
        ($field:ident) => {
            if let Some(value) = patch.$field.as_ref() {
                config.$field.clone_from(value);
            }
        };
    }
    apply!(chain);
    apply!(pool_or_route);
    apply!(base_token);
    apply!(quote_token);
    apply!(base_mint);
    apply!(quote_mint);
    apply!(base_amount_raw);
    apply!(quote_amount_raw);
    apply!(wallet_address);
    apply!(cex_venue);
    apply!(cex_symbol);
}

fn apply_numeric_patch(config: &mut OnchainComparisonConfig, patch: &OnchainComparisonConfigPatch) {
    macro_rules! apply {
        ($field:ident) => {
            if let Some(value) = patch.$field {
                config.$field = value;
            }
        };
    }
    apply!(enabled);
    apply!(base_identity_resolved);
    apply!(quote_identity_resolved);
    apply!(base_decimals);
    apply!(quote_decimals);
    apply!(cex_taker_fee_bps);
    apply!(gas_usd);
    apply!(slippage_bps);
    apply!(min_liquidity_usd);
    apply!(max_age_ms);
}

fn apply_spread_alert_patch(
    config: &mut OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
) {
    let Some(alert) = patch.spread_alert.as_ref() else {
        return;
    };
    if let Some(enabled) = alert.enabled {
        config.spread_alert.enabled = enabled;
    }
    if let Some(mode) = alert.mode {
        config.spread_alert.mode = mode;
    }
    if let Some(threshold) = alert.min_net_spread_bps {
        config.spread_alert.min_net_spread_bps = threshold;
    }
    if let Some(threshold) = alert.min_raw_spread_bps {
        config.spread_alert.min_raw_spread_bps = threshold;
    }
    if let Some(cooldown_ms) = alert.cooldown_ms {
        config.spread_alert.cooldown_ms = cooldown_ms;
    }
}

fn apply_dex_comparison_patch(
    config: &mut OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
) {
    let Some(dex) = patch.dex_comparison.as_ref() else {
        return;
    };
    if let Some(enabled) = dex.enabled {
        config.dex_comparison.enabled = enabled;
    }
    if let Some(provider) = dex.peer_provider.as_deref() {
        config.dex_comparison.peer_provider = provider.trim().to_ascii_lowercase();
    }
}

fn apply_cross_chain_patch(
    config: &mut OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
) {
    let Some(cross_chain) = patch.cross_chain.as_ref() else {
        return;
    };
    if let Some(enabled) = cross_chain.enabled {
        config.cross_chain.enabled = enabled;
    }
    if let Some(peer_item_id) = cross_chain.peer_item_id.as_deref() {
        config.cross_chain.peer_item_id = peer_item_id.trim().to_owned();
    }
    if let Some(provider) = cross_chain.provider.as_deref() {
        config.cross_chain.provider = provider.trim().to_ascii_lowercase();
    }
    if let Some(risk_bps) = cross_chain.stablecoin_risk_bps {
        config.cross_chain.stablecoin_risk_bps = risk_bps;
    }
}

fn default_peer_provider(primary: &str, chain: &str) -> String {
    ONCHAIN_QUOTE_PROVIDERS
        .iter()
        .find(|provider| {
            onchain_quote_provider_supported(provider.id, chain)
                && onchain_quote_providers_independent(primary, provider.id)
        })
        .map_or_else(String::new, |provider| provider.id.to_owned())
}

fn validate(
    config: &OnchainComparisonConfig,
    custom_rpc_url: Option<&str>,
) -> Result<(), OnchainMonitorError> {
    if onchain_chain_preset(&config.chain).is_none() {
        return Err(OnchainMonitorError(
            "unsupported on-chain network".to_owned(),
        ));
    }
    if !onchain_quote_provider_supported(&config.provider, &config.chain) {
        return Err(OnchainMonitorError(format!(
            "provider {} does not support chain {}",
            config.provider, config.chain
        )));
    }
    if config.dex_comparison.enabled
        && (!onchain_quote_provider_supported(&config.dex_comparison.peer_provider, &config.chain)
            || !onchain_quote_providers_independent(
                &config.provider,
                &config.dex_comparison.peer_provider,
            ))
    {
        return Err(OnchainMonitorError(
            "DEX peer provider must be supported on this chain and independent from the primary provider"
                .to_owned(),
        ));
    }
    if config.cross_chain.enabled
        && (config.cross_chain.peer_item_id.trim().is_empty()
            || config.cross_chain.provider != "lifi"
            || config.cross_chain.stablecoin_risk_bps >= 10_000)
    {
        return Err(OnchainMonitorError(
            "cross-chain monitoring requires a peer market, LI.FI, and stablecoin risk below 100%"
                .to_owned(),
        ));
    }
    if config.rpc.mode == OnchainRpcMode::Custom && custom_rpc_url.is_none() {
        return Err(OnchainMonitorError(
            "custom RPC requires a trusted HTTPS endpoint".to_owned(),
        ));
    }
    if !ONCHAIN_CEX_VENUES
        .iter()
        .any(|venue| venue.id.eq_ignore_ascii_case(&config.cex_venue))
    {
        return Err(OnchainMonitorError(
            "unsupported CEX venue for on-chain comparison".to_owned(),
        ));
    }
    if [
        config.base_token.as_str(),
        config.quote_token.as_str(),
        config.base_mint.as_str(),
        config.quote_mint.as_str(),
        config.cex_venue.as_str(),
        config.cex_symbol.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(OnchainMonitorError(
            "token and CEX mappings are required".to_owned(),
        ));
    }
    if !config.wallet_address.trim().is_empty() && !valid_wallet_address(config) {
        return Err(OnchainMonitorError(format!(
            "wallet address does not match chain {}",
            config.chain
        )));
    }
    let normalized_base = normalized_pair_symbol(&config.base_token);
    let normalized_quote = normalized_pair_symbol(&config.quote_token);
    let valid_cex_pair = configured_cex_pair(&config.base_token, &config.cex_symbol)
        .is_some_and(|(base, quote)| base != quote);
    if !valid_cex_pair || normalized_base == normalized_quote {
        return Err(OnchainMonitorError(
            "CEX symbol must contain distinct Base and Quote assets".to_owned(),
        ));
    }
    if !valid_raw_amount(&config.base_amount_raw)
        || !valid_raw_amount(&config.quote_amount_raw)
        || config.base_decimals > 18
        || config.quote_decimals > 18
        || !(1_000..=60_000).contains(&config.max_age_ms)
        || !valid_cost(config.cex_taker_fee_bps)
        || !valid_cost(config.gas_usd)
        || !valid_cost(config.slippage_bps)
        || !config.min_liquidity_usd.is_finite()
        || config.min_liquidity_usd <= 0.0
        || !config.spread_alert.min_net_spread_bps.is_finite()
        || !(0.0..=10_000.0).contains(&config.spread_alert.min_net_spread_bps)
        || !config.spread_alert.min_raw_spread_bps.is_finite()
        || !(0.0..=10_000.0).contains(&config.spread_alert.min_raw_spread_bps)
        || !(30_000..=86_400_000).contains(&config.spread_alert.cooldown_ms)
    {
        return Err(OnchainMonitorError(
            "numeric comparison settings are invalid".to_owned(),
        ));
    }
    Ok(())
}

fn configured_cex_pair(base_token: &str, symbol: &str) -> Option<(String, String)> {
    let pair = symbol
        .split_once('/')
        .or_else(|| symbol.split_once(':'))
        .or_else(|| symbol.split_once('-'))
        .or_else(|| symbol.split_once('_'));
    if let Some((base, quote)) = pair {
        let base = normalized_pair_symbol(base);
        let quote = normalized_pair_symbol(quote);
        return (!base.is_empty() && !quote.is_empty()).then_some((base, quote));
    }
    let base = normalized_pair_symbol(base_token);
    let compact = normalized_pair_symbol(symbol);
    let quote = compact.strip_prefix(&base)?.to_owned();
    (!base.is_empty() && !quote.is_empty()).then_some((base, quote))
}

fn next_custom_rpc_url(
    config: &OnchainComparisonConfig,
    patch: &OnchainComparisonConfigPatch,
    current: Option<Arc<String>>,
) -> Result<Option<String>, OnchainMonitorError> {
    if config.rpc.mode == OnchainRpcMode::ProviderManaged {
        return Ok(None);
    }
    let raw = patch
        .source
        .as_ref()
        .and_then(|source| source.custom_rpc_url.as_deref());
    match raw {
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => validate_rpc_url(value).map(Some),
        None => Ok(current.map(|value| (*value).clone())),
    }
}

fn validate_rpc_url(raw: &str) -> Result<String, OnchainMonitorError> {
    let trimmed = raw.trim();
    if trimmed.len() > 2_048 {
        return Err(OnchainMonitorError(
            "custom RPC URL exceeds 2048 bytes".to_owned(),
        ));
    }
    let url = Url::parse(trimmed)
        .map_err(|_| OnchainMonitorError("custom RPC URL is invalid".to_owned()))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(OnchainMonitorError(
            "custom RPC URL must use HTTPS".to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(OnchainMonitorError(
            "custom RPC URL must not contain userinfo or a fragment".to_owned(),
        ));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.parse::<IpAddr>().is_ok_and(private_or_local_ip)
    {
        return Err(OnchainMonitorError(
            "custom RPC URL must resolve through a public trusted provider".to_owned(),
        ));
    }
    Ok(url.to_string())
}

fn private_or_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
    }
}

fn pending_rpc_status(config: &OnchainComparisonConfig, rpc_url: Option<&str>) -> OnchainRpcStatus {
    let expected_chain_id = onchain_chain_preset(&config.chain).and_then(|preset| preset.chain_id);
    if config.rpc.mode == OnchainRpcMode::ProviderManaged {
        return OnchainRpcStatus {
            mode: OnchainRpcMode::ProviderManaged,
            expected_chain_id,
            official_docs_url: rpc_docs(expected_chain_id).to_owned(),
            ..OnchainRpcStatus::default()
        };
    }
    OnchainRpcStatus {
        mode: OnchainRpcMode::Custom,
        configured: rpc_url.is_some(),
        endpoint_label: rpc_url.and_then(rpc_endpoint_label),
        expected_chain_id,
        ready: false,
        problem: Some("waiting for custom RPC chain and head verification".to_owned()),
        official_docs_url: rpc_docs(expected_chain_id).to_owned(),
        ..OnchainRpcStatus::default()
    }
}

const fn rpc_docs(expected_chain_id: Option<u64>) -> &'static str {
    if expected_chain_id.is_some() {
        "https://ethereum.org/developers/docs/apis/json-rpc/"
    } else {
        "https://solana.com/docs/rpc/http/getgenesishash"
    }
}

fn rpc_endpoint_label(value: &str) -> Option<String> {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

pub fn normalized_pair_symbol(symbol: &str) -> String {
    symbol
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn valid_raw_amount(value: &str) -> bool {
    value.parse::<u128>().is_ok_and(|amount| amount > 0)
}

fn valid_cost(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn valid_wallet_address(config: &OnchainComparisonConfig) -> bool {
    let address = config.wallet_address.trim();
    if onchain_chain_preset(&config.chain).is_some_and(|preset| preset.chain_id.is_some()) {
        return address.len() == 42
            && address.starts_with("0x")
            && address[2..].bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    // Solana RPC requires a base-58 encoded account public key.
    // https://solana.com/docs/rpc/http/getbalance
    (32..=44).contains(&address.len())
        && address
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() && !matches!(byte, b'0' | b'O' | b'I' | b'l'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{OnchainSourceConfigPatch, OnchainSpreadAlertConfigPatch};

    fn quote_pair(config: &OnchainComparisonConfig) -> OnchainQuotePair {
        OnchainQuotePair {
            chain: config.chain.clone(),
            provider: config.provider.clone(),
            endpoint: "quote".to_owned(),
            official_docs_url: "https://example.test/docs".to_owned(),
            base_address: config.base_mint.clone(),
            quote_address: config.quote_mint.clone(),
            forward: crate::ProviderQuote {
                input_address: config.base_mint.clone(),
                output_address: config.quote_mint.clone(),
                input_amount_raw: config.base_amount_raw.clone(),
                output_amount_raw: config.quote_amount_raw.clone(),
                router: None,
            },
            reverse: crate::ProviderQuote {
                input_address: config.quote_mint.clone(),
                output_address: config.base_mint.clone(),
                input_amount_raw: config.quote_amount_raw.clone(),
                output_amount_raw: config.base_amount_raw.clone(),
                router: None,
            },
            observed_at_ms: 1,
            request_latency_ms: 10,
            quote_interval_ms: 1_000,
        }
    }

    #[test]
    fn invalid_mapping_numbers_fail_closed() {
        let monitor = OnchainMonitor::default();
        let result = monitor.update_config(
            &OnchainComparisonConfigPatch {
                base_amount_raw: Some("0".to_owned()),
                gas_usd: Some(-1.0),
                max_age_ms: Some(999),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cex_only_update_keeps_matching_dex_quote() -> Result<(), OnchainMonitorError> {
        let config = OnchainComparisonConfig {
            enabled: true,
            ..OnchainComparisonConfig::default()
        };
        let monitor = OnchainMonitor::from_config(config.clone(), 0)?;
        monitor.publish_quote_pair(quote_pair(&config));

        let snapshot = monitor.update_config(
            &OnchainComparisonConfigPatch {
                cex_venue: Some("okx".to_owned()),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        )?;

        assert_eq!(snapshot.quality, OnchainComparisonQuality::Pending);
        assert!(monitor.quote_pair().is_some());

        monitor.update_config(
            &OnchainComparisonConfigPatch {
                base_amount_raw: Some("2000000000".to_owned()),
                ..OnchainComparisonConfigPatch::default()
            },
            2,
        )?;
        assert!(monitor.quote_pair().is_none());
        Ok(())
    }

    #[test]
    fn restored_and_previewed_configs_do_not_require_an_early_commit(
    ) -> Result<(), OnchainMonitorError> {
        let mut restored = OnchainComparisonConfig::default();
        restored.enabled = true;
        restored.provider = "jupiter_swap_v2_keyed".to_owned();
        restored.pool_or_route = "jupiter-api-key".to_owned();
        let monitor = OnchainMonitor::from_config(restored.clone(), 50)?;

        assert_eq!(monitor.snapshot().config, restored);
        assert_eq!(
            monitor.snapshot().quality,
            OnchainComparisonQuality::Pending
        );
        assert_eq!(
            monitor.snapshot().rpc_status.official_docs_url,
            "https://solana.com/docs/rpc/http/getgenesishash"
        );
        let preview = monitor.preview_config(&OnchainComparisonConfigPatch {
            cex_symbol: Some("SOL/USDT".to_owned()),
            ..OnchainComparisonConfigPatch::default()
        })?;

        assert_eq!(preview.cex_symbol, "SOL/USDT");
        assert_eq!(monitor.snapshot().config.cex_symbol, "SOL/USDC");
        Ok(())
    }

    #[test]
    fn spread_alert_limits_are_applied_and_fail_closed() -> Result<(), OnchainMonitorError> {
        let monitor = OnchainMonitor::default();
        let snapshot = monitor.update_config(
            &OnchainComparisonConfigPatch {
                spread_alert: Some(OnchainSpreadAlertConfigPatch {
                    enabled: Some(true),
                    mode: Some(shared_types::OnchainSpreadAlertMode::RawObservation),
                    min_net_spread_bps: Some(25.0),
                    min_raw_spread_bps: Some(40.0),
                    cooldown_ms: Some(60_000),
                }),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        )?;
        assert!(snapshot.config.spread_alert.enabled);
        assert_eq!(
            snapshot.config.spread_alert.mode,
            shared_types::OnchainSpreadAlertMode::RawObservation
        );
        assert_eq!(snapshot.config.spread_alert.min_net_spread_bps, 25.0);
        assert_eq!(snapshot.config.spread_alert.min_raw_spread_bps, 40.0);
        assert_eq!(snapshot.config.spread_alert.cooldown_ms, 60_000);

        let invalid = monitor.update_config(
            &OnchainComparisonConfigPatch {
                spread_alert: Some(OnchainSpreadAlertConfigPatch {
                    cooldown_ms: Some(1_000),
                    ..OnchainSpreadAlertConfigPatch::default()
                }),
                ..OnchainComparisonConfigPatch::default()
            },
            2,
        );
        assert!(invalid.is_err());
        Ok(())
    }

    #[test]
    fn cross_quote_pair_is_saved_for_explicit_conversion_gating() {
        let monitor = OnchainMonitor::default();
        let result = monitor.update_config(
            &OnchainComparisonConfigPatch {
                cex_symbol: Some("SOL/USDT".to_owned()),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        );

        assert_eq!(
            result
                .ok()
                .map(|snapshot| snapshot.config.cex_symbol.clone()),
            Some("SOL/USDT".to_owned())
        );
    }

    #[test]
    fn cex_pair_with_another_base_is_saved_for_raw_observation() {
        let monitor = OnchainMonitor::default();
        let result = monitor.update_config(
            &OnchainComparisonConfigPatch {
                cex_symbol: Some("BTC/USDT".to_owned()),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        );

        assert_eq!(
            result
                .ok()
                .map(|snapshot| snapshot.config.cex_symbol.clone()),
            Some("BTC/USDT".to_owned())
        );
    }

    #[test]
    fn cex_pair_rejects_identical_base_and_quote() {
        let monitor = OnchainMonitor::default();
        let result = monitor.update_config(
            &OnchainComparisonConfigPatch {
                cex_symbol: Some("SOL/SOL".to_owned()),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        );

        assert!(result
            .map_err(|error| error.to_string())
            .is_err_and(|error| error.contains("distinct Base and Quote")));
    }

    #[test]
    fn equivalent_pair_separators_are_accepted() {
        assert_eq!(normalized_pair_symbol("sol-usdc"), "SOLUSDC");
        assert_eq!(normalized_pair_symbol("SOL/USDC"), "SOLUSDC");
    }

    #[test]
    fn quote_attempts_are_paced_even_when_an_attempt_does_not_publish() {
        let monitor = OnchainMonitor::default();

        assert!(monitor.try_begin_quote_attempt(10_000, 4_500));
        assert!(!monitor.try_begin_quote_attempt(14_499, 4_500));
        assert!(monitor.try_begin_quote_attempt(14_500, 4_500));
        assert!(!monitor.try_begin_quote_attempt(14_500, 4_500));
    }

    #[test]
    fn chain_change_applies_documented_provider_and_default_identity(
    ) -> Result<(), OnchainMonitorError> {
        let monitor = OnchainMonitor::default();
        let snapshot = monitor.update_config(
            &OnchainComparisonConfigPatch {
                chain: Some("base".to_owned()),
                cex_symbol: Some("ETH/USDC".to_owned()),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        )?;

        assert_eq!(snapshot.config.provider, "zeroex_swap_v2");
        assert_eq!(snapshot.config.base_token, "ETH");
        assert_eq!(snapshot.config.quote_token, "USDC");
        assert_eq!(snapshot.config.quote_decimals, 6);
        Ok(())
    }

    #[test]
    fn evm_provider_can_switch_to_okx_without_changing_chain_identity(
    ) -> Result<(), OnchainMonitorError> {
        let monitor = OnchainMonitor::default();
        let snapshot = monitor.update_config(
            &OnchainComparisonConfigPatch {
                chain: Some("base".to_owned()),
                cex_symbol: Some("ETH/USDC".to_owned()),
                source: Some(OnchainSourceConfigPatch {
                    provider: Some("okx_dex_v6".to_owned()),
                    ..OnchainSourceConfigPatch::default()
                }),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        )?;

        assert_eq!(snapshot.config.provider, "okx_dex_v6");
        assert_eq!(snapshot.config.pool_or_route, "okx-dex-aggregator");
        Ok(())
    }

    #[test]
    fn custom_rpc_is_kept_out_of_public_snapshot_and_private_hosts_are_rejected(
    ) -> Result<(), OnchainMonitorError> {
        let monitor = OnchainMonitor::default();
        let snapshot = monitor.update_config(
            &OnchainComparisonConfigPatch {
                chain: Some("base".to_owned()),
                cex_symbol: Some("ETH/USDC".to_owned()),
                source: Some(OnchainSourceConfigPatch {
                    rpc_mode: Some(OnchainRpcMode::Custom),
                    custom_rpc_url: Some(
                        "https://base-mainnet.example/v2/private-token".to_owned(),
                    ),
                    ..OnchainSourceConfigPatch::default()
                }),
                ..OnchainComparisonConfigPatch::default()
            },
            1,
        )?;

        let serialized = serde_json::to_string(&*snapshot)
            .map_err(|error| OnchainMonitorError(error.to_string()))?;
        assert!(!serialized.contains("private-token"));
        assert_eq!(
            snapshot.rpc_status.endpoint_label.as_deref(),
            Some("base-mainnet.example")
        );
        assert!(monitor.custom_rpc_url().is_some());

        let before_rejected_update = monitor.snapshot();
        let private = monitor.update_config(
            &OnchainComparisonConfigPatch {
                source: Some(OnchainSourceConfigPatch {
                    rpc_mode: Some(OnchainRpcMode::Custom),
                    custom_rpc_url: Some("https://127.0.0.1:8545".to_owned()),
                    ..OnchainSourceConfigPatch::default()
                }),
                ..OnchainComparisonConfigPatch::default()
            },
            2,
        );
        assert!(private.is_err());
        assert_eq!(monitor.snapshot().config, before_rejected_update.config);
        assert_eq!(
            monitor.snapshot().rpc_status,
            before_rejected_update.rpc_status
        );
        Ok(())
    }
}
