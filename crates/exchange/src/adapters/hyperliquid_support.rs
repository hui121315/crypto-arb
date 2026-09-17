//! Hyperliquid adapter construction and private support helpers.

use super::{
    Hyperliquid, HyperliquidAccountAbstraction, HyperliquidConfig, HyperliquidCredentialRelation,
    HyperliquidCredentials, NAME,
};
use crate::adapter::ExchangeAdapter;
use crate::adapters::hyperliquid_config::{PROD_BASE, PROD_WS_TRADE};
use crate::adapters::hyperliquid_instruments::{
    builder_dex_index, HyperliquidInstrumentMetadata, HyperliquidMeta, HyperliquidPerpCategories,
    HyperliquidPerpDex,
};
use crate::adapters::hyperliquid_private_data::{parse_spot_balances, SpotClearinghouseState};
use crate::adapters::hyperliquid_public_rest as public_rest;
use crate::adapters::hyperliquid_trade_data::EXCHANGE_PATH;
use crate::adapters::hyperliquid_ws_trade::{self, WsInfoConfig, WsTradeConfig};
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use crate::signing::hyperliquid::{self, HyperliquidNetwork};
use crate::venue_spec::VenueId;
use moka::future::Cache;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::BalanceInfo;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

// Official WebSocket limit is 2,000 outbound messages/minute across all
// connections. Keep headroom for subscriptions, heartbeats, and trade actions.
const WS_INFO_REQUESTS_PER_SECOND: u32 = 25;
const BUILDER_DIRECTORY_TTL: Duration = Duration::from_secs(30);
const WS_INFO_ATTEMPT_TIMEOUT_SECS: u64 = 2;
const WS_INFO_RETRY_BACKOFF_MS: i64 = 30_000;

type BuilderDirectoryResult = ExchangeResult<Arc<HyperliquidBuilderDirectory>>;

static BUILDER_DIRECTORIES: OnceLock<Cache<String, Arc<BuilderDirectoryResult>>> = OnceLock::new();
static WS_INFO_RETRY_AFTER_MS: AtomicI64 = AtomicI64::new(0);

#[derive(Debug)]
struct HyperliquidBuilderDirectory {
    dexes: Vec<Option<HyperliquidPerpDex>>,
    categories: HyperliquidPerpCategories,
}

impl Hyperliquid {
    pub fn new(config: HyperliquidConfig) -> ExchangeResult<Self> {
        let base_url = config
            .base_url_override
            .clone()
            .unwrap_or_else(|| PROD_BASE.to_owned());
        let adapter_name = config.market.venue();
        let rate_limiter = Arc::new(RateLimiter::with_shared_budget(
            adapter_name,
            config.qps,
            NAME,
            VenueId::Hyperliquid.defaults().qps,
        ));
        let http = HttpClient::builder(adapter_name)
            .timeout_secs(config.timeout_secs)
            .rate_limiter(Arc::clone(&rate_limiter))
            .build()?;
        Ok(Self {
            config,
            base_url,
            http,
            _rate_limiter: rate_limiter,
            market_stream: OnceLock::new(),
            active_ctx_stream: OnceLock::new(),
            all_mids_stream: OnceLock::new(),
        })
    }

    pub(super) fn adapter_name(&self) -> &'static str {
        self.config.market.venue()
    }

    pub(super) fn api_coin(&self, symbol: &str) -> String {
        let base = self.normalize_symbol(symbol);
        self.config
            .market
            .dex()
            .map(|dex| format!("{dex}:{base}"))
            .unwrap_or(base)
    }

    pub(super) fn meta_body(&self) -> Value {
        self.config.market.dex().map_or_else(
            || json!({"type": "metaAndAssetCtxs"}),
            |dex| json!({"type": "metaAndAssetCtxs", "dex": dex}),
        )
    }

    pub(super) fn private_state_body(&self, request_type: &str, user: &str) -> Value {
        self.config.market.dex().map_or_else(
            || json!({"type": request_type, "user": user}),
            |dex| json!({"type": request_type, "user": user, "dex": dex}),
        )
    }

    pub(super) fn require_credentials(&self) -> ExchangeResult<&HyperliquidCredentials> {
        self.config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("hyperliquid: missing user_address".into()))
    }

    pub(super) fn require_user(&self) -> ExchangeResult<String> {
        Ok(self.require_credentials()?.user_address.clone())
    }

    pub(super) fn require_private_key(&self) -> ExchangeResult<&str> {
        self.require_credentials()?
            .private_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| ExchangeError::Auth("hyperliquid: missing private_key".into()))
    }

    pub async fn validate_account_role_status(&self) -> ExchangeResult<()> {
        let user = self.require_user()?;
        let role = self.user_role(&user).await?;
        match role.role.as_str() {
            "user" | "vault" | "subAccount" => Ok(()),
            "missing" => Err(ExchangeError::Auth(format!(
                "hyperliquid account address is missing: {user}"
            ))),
            "agent" => Err(ExchangeError::Auth(
                "hyperliquid account address is an agent wallet; use the master/sub-account address for reads".into(),
            )),
            other => Err(ExchangeError::Parse(format!(
                "hyperliquid unsupported userRole for account: {other}"
            ))),
        }
    }

    pub async fn validate_agent_approval_status(&self) -> ExchangeResult<bool> {
        let account = self.require_user()?;
        let agent = hyperliquid::address_from_private_key(self.require_private_key()?)
            .map_err(|error| ExchangeError::Auth(format!("hyperliquid private_key: {error}")))?;
        let role = self.user_role(&agent).await?;
        match role.role.as_str() {
            "agent" => Ok(role
                .data
                .as_ref()
                .and_then(|data| data.user.as_deref())
                .is_some_and(|owner| addresses_equal(owner, &account))),
            "missing" | "user" | "vault" | "subAccount" => Ok(false),
            other => Err(ExchangeError::Parse(format!(
                "hyperliquid unsupported userRole for agent signer: {other}"
            ))),
        }
    }

    /// Read the official account/signer/vault relationship without sending a
    /// write action. The API layer applies the fail-closed role policy to these
    /// raw facts and records the resulting credential probe.
    pub async fn credential_relation(&self) -> ExchangeResult<HyperliquidCredentialRelation> {
        let main_account = self.require_user()?;
        let signer = hyperliquid::address_from_private_key(self.require_private_key()?)
            .map_err(|error| ExchangeError::Auth(format!("hyperliquid private_key: {error}")))?;
        let main_account_role = self.user_role(&main_account).await?;
        let main_account_owner = main_account_role
            .data
            .as_ref()
            .and_then(UserRoleData::owner)
            .map(str::to_owned);
        let (signer_role, signer_owner) = if addresses_equal(&signer, &main_account) {
            (main_account_role.role.clone(), None)
        } else {
            let signer_role = self.user_role(&signer).await?;
            (
                signer_role.role,
                signer_role.data.and_then(|data| data.user),
            )
        };
        let vault_address = self
            .require_credentials()?
            .vault_address
            .as_deref()
            .map(str::trim)
            .filter(|address| !address.is_empty())
            .map(str::to_owned);
        let (vault_role, vault_leader) = match vault_address.as_deref() {
            None => (None, None),
            Some(vault_address) => {
                let (vault_role, vault_details) = tokio::try_join!(
                    self.user_role(vault_address),
                    self.vault_details(vault_address, &main_account),
                )?;
                if !addresses_equal(&vault_details.vault_address, vault_address) {
                    return Err(ExchangeError::Parse(format!(
                        "hyperliquid vaultDetails address mismatch: requested={vault_address} returned={}",
                        vault_details.vault_address
                    )));
                }
                (Some(vault_role.role), Some(vault_details.leader))
            }
        };
        Ok(HyperliquidCredentialRelation {
            main_account,
            main_account_role: main_account_role.role,
            main_account_owner,
            signer,
            signer_role,
            signer_owner,
            vault_address,
            vault_role,
            vault_leader,
        })
    }

    /// Read the official account and builder-dex abstraction states for the
    /// configured master/sub-account address.
    pub async fn account_abstraction_state(&self) -> ExchangeResult<HyperliquidAccountAbstraction> {
        let account_address = self.require_user()?;
        let (user_abstraction, user_dex_abstraction) = tokio::try_join!(
            self.post_info::<String>(json!({
                "type": "userAbstraction",
                "user": &account_address,
            })),
            self.post_info::<Option<bool>>(json!({
                "type": "userDexAbstraction",
                "user": &account_address,
            })),
        )?;
        if user_abstraction.trim().is_empty() {
            return Err(ExchangeError::Parse(
                "hyperliquid userAbstraction returned an empty state".into(),
            ));
        }
        Ok(HyperliquidAccountAbstraction {
            account_address,
            user_abstraction,
            user_dex_abstraction: user_dex_abstraction.map(|enabled| {
                if enabled {
                    "enabled".to_owned()
                } else {
                    "disabled".to_owned()
                }
            }),
        })
    }

    pub async fn spot_balance_truth(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        self.get_spot_balance(currency).await
    }

    pub async fn validate_safe_noop_permission(&self) -> ExchangeResult<()> {
        let cfg = self.exchange_action_config()?;
        let signer_key = hyperliquid_ws_trade::signer_nonce_key(cfg.network, cfg.private_key);
        let nonce = hyperliquid_ws_trade::next_nonce(&signer_key);
        let expires_after = cfg
            .action_expires_after_ms
            .map(|window| nonce.saturating_add(window));
        let action = json!({"type": "noop"});
        let signature = hyperliquid::sign_l1_action(
            cfg.private_key,
            &action,
            nonce,
            cfg.vault_address,
            expires_after,
            cfg.network,
        )
        .map_err(|error| {
            ExchangeError::Auth(format!("hyperliquid noop signing failed: {error}"))
        })?;
        let mut body = json!({
            "action": action,
            "nonce": nonce,
            "signature": {
                "r": signature.r,
                "s": signature.s,
                "v": signature.v,
            },
        });
        if let Some(vault_address) = cfg.vault_address {
            body["vaultAddress"] = json!(vault_address);
        }
        if let Some(expires_after) = expires_after {
            body["expiresAfter"] = json!(expires_after);
        }
        let result = self.post_exchange_noop(body).await;
        hyperliquid_ws_trade::record_signer_session_result(cfg, nonce, result.as_ref().err());
        result
    }

    pub(super) fn ensure_write_adapter(&self) -> ExchangeResult<()> {
        if self.config.allow_live_writes {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "hyperliquid live trading disabled; enable live writes explicitly".into(),
            ))
        }
    }

    pub(super) fn ws_trade_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        self.exchange_action_config()
    }

    fn ws_info_config(&self) -> WsInfoConfig<'_> {
        WsInfoConfig {
            url: PROD_WS_TRADE,
            // Preserve time inside the account-read deadline for HTTP fallback.
            timeout_secs: self.config.timeout_secs.min(WS_INFO_ATTEMPT_TIMEOUT_SECS),
        }
    }

    fn exchange_action_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        let vault_address = self
            .require_credentials()?
            .vault_address
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        Ok(WsTradeConfig {
            url: PROD_WS_TRADE,
            account_address: &self.require_credentials()?.user_address,
            private_key: self.require_private_key()?,
            timeout_secs: self.config.timeout_secs,
            network: HyperliquidNetwork::Mainnet,
            vault_address,
            action_expires_after_ms: self.config.action_expires_after_ms,
        })
    }

    async fn post_exchange_noop(&self, body: Value) -> ExchangeResult<()> {
        let url = format!("{}{}", self.base_url, EXCHANGE_PATH);
        let resp = self
            .http
            .execute_with_retry(|| {
                self.http
                    .request(reqwest::Method::POST, &url)
                    .header("Content-Type", "application/json")
                    .json(&body)
            })
            .await?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|error| ExchangeError::Network(format!("hyperliquid noop body: {error}")))?;
        if !(200..300).contains(&status) {
            return Err(ExchangeError::Http { status, body: text });
        }
        let value: Value = serde_json::from_str(&text).map_err(|error| {
            ExchangeError::Parse(format!("hyperliquid noop json: {error}: {text}"))
        })?;
        if value.get("status").and_then(Value::as_str) == Some("ok") {
            Ok(())
        } else {
            Err(ExchangeError::Api {
                exchange: NAME.into(),
                code: "noop".into(),
                message: value.to_string(),
            })
        }
    }

    pub(super) async fn get_spot_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        let user = self.require_user()?;
        let body = json!({"type": "spotClearinghouseState", "user": user});
        let state: SpotClearinghouseState = self.post_info(body).await?;
        parse_spot_balances(state, currency)
    }

    pub(super) async fn post_info<T: serde::de::DeserializeOwned>(
        &self,
        body: Value,
    ) -> ExchangeResult<T> {
        let operation = body
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_owned();
        if self.config.base_url_override.is_none() && claim_ws_info_probe() {
            ws_info_rate_limiter().wait().await;
            match hyperliquid_ws_trade::post_info(self.ws_info_config(), body.clone()).await {
                Ok(response) => {
                    WS_INFO_RETRY_AFTER_MS.store(0, Ordering::Release);
                    return Ok(response);
                }
                Err(error) => {
                    WS_INFO_RETRY_AFTER_MS.store(
                        common::time::now_ms().saturating_add(WS_INFO_RETRY_BACKOFF_MS),
                        Ordering::Release,
                    );
                    tracing::warn!(
                        %error,
                        operation = %operation,
                        retry_ms = WS_INFO_RETRY_BACKOFF_MS,
                        "hyperliquid ws info request failed; temporarily using HTTP"
                    );
                }
            }
        }
        public_rest::post_info(&self.http, &self.base_url, body).await
    }

    pub(super) async fn instrument_metadata(
        &self,
    ) -> ExchangeResult<HyperliquidInstrumentMetadata> {
        let Some(dex) = self.config.market.dex() else {
            let meta = self
                .post_info::<HyperliquidMeta>(json!({"type": "meta"}))
                .await?;
            return Ok(HyperliquidInstrumentMetadata {
                meta,
                builder_dex_index: None,
                categories: Vec::new(),
            });
        };
        let directory = self.builder_directory().await?;
        let meta = self
            .post_info::<HyperliquidMeta>(json!({"type": "meta", "dex": dex}))
            .await?;
        Ok(HyperliquidInstrumentMetadata {
            meta,
            builder_dex_index: Some(builder_dex_index(&directory.dexes, dex)?),
            categories: directory.categories.clone(),
        })
    }

    async fn builder_directory(&self) -> BuilderDirectoryResult {
        let key = self.base_url.clone();
        let snapshot = builder_directories()
            .get_with(key, async {
                let dexes = self
                    .post_info::<Vec<Option<HyperliquidPerpDex>>>(json!({"type": "perpDexs"}));
                let categories = self
                    .post_info::<HyperliquidPerpCategories>(json!({"type": "perpCategories"}));
                let (dexes, categories) = tokio::join!(dexes, categories);
                let categories = match categories {
                    Ok(categories) => categories,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "hyperliquid perp categories unavailable; builder identities remain unknown"
                        );
                        Vec::new()
                    }
                };
                Arc::new(dexes.map(|dexes| {
                    Arc::new(HyperliquidBuilderDirectory { dexes, categories })
                }))
            })
            .await;
        snapshot.as_ref().clone()
    }

    async fn user_role(&self, user: &str) -> ExchangeResult<UserRoleResponse> {
        self.post_info(json!({"type": "userRole", "user": user}))
            .await
    }

    async fn vault_details(
        &self,
        vault_address: &str,
        user: &str,
    ) -> ExchangeResult<VaultDetailsResponse> {
        self.post_info(json!({
            "type": "vaultDetails",
            "vaultAddress": vault_address,
            "user": user,
        }))
        .await
    }
}

fn claim_ws_info_probe() -> bool {
    let now_ms = common::time::now_ms();
    let lease_until_ms = now_ms.saturating_add(WS_INFO_ATTEMPT_TIMEOUT_SECS as i64 * 1_000);
    let mut retry_after_ms = WS_INFO_RETRY_AFTER_MS.load(Ordering::Acquire);
    loop {
        if retry_after_ms > now_ms {
            return false;
        }
        match WS_INFO_RETRY_AFTER_MS.compare_exchange_weak(
            retry_after_ms,
            lease_until_ms,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(current) => retry_after_ms = current,
        }
    }
}

fn ws_info_rate_limiter() -> &'static RateLimiter {
    static LIMITER: OnceLock<RateLimiter> = OnceLock::new();
    LIMITER
        .get_or_init(|| RateLimiter::per_second("hyperliquid_ws_info", WS_INFO_REQUESTS_PER_SECOND))
}

fn builder_directories() -> &'static Cache<String, Arc<BuilderDirectoryResult>> {
    BUILDER_DIRECTORIES.get_or_init(|| {
        Cache::builder()
            .max_capacity(8)
            .time_to_live(BUILDER_DIRECTORY_TTL)
            .build()
    })
}

#[derive(Debug, Deserialize)]
struct UserRoleResponse {
    role: String,
    #[serde(default)]
    data: Option<UserRoleData>,
}

#[derive(Debug, Deserialize)]
struct UserRoleData {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    master: Option<String>,
}

impl UserRoleData {
    fn owner(&self) -> Option<&str> {
        self.user.as_deref().or(self.master.as_deref())
    }
}

#[derive(Debug, Deserialize)]
struct VaultDetailsResponse {
    #[serde(rename = "vaultAddress")]
    vault_address: String,
    leader: String,
}

fn addresses_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}
