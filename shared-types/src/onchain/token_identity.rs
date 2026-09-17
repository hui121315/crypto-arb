use crate::{ApiProblem, MarketDataQuality, MarketDataSourceKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenIdentityRequest {
    pub chain: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_rpc_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenIdentity {
    pub chain: String,
    pub address: String,
    pub symbol: String,
    pub name: Option<String>,
    pub decimals: u8,
    pub source: String,
    pub evidence_url: String,
    pub verified: bool,
    pub native: bool,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainTokenResolution {
    pub chain: String,
    pub address: String,
    pub decimals: u8,
    pub precision_source: String,
    pub precision_evidence_url: String,
    pub identity: Option<OnchainTokenIdentity>,
    pub identity_problem: Option<String>,
    pub observed_at_ms: i64,
}

impl OnchainTokenResolution {
    pub fn complete(identity: OnchainTokenIdentity) -> Self {
        Self {
            chain: identity.chain.clone(),
            address: identity.address.clone(),
            decimals: identity.decimals,
            precision_source: identity.source.clone(),
            precision_evidence_url: identity.evidence_url.clone(),
            observed_at_ms: identity.observed_at_ms,
            identity: Some(identity),
            identity_problem: None,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.identity.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexPairQuery {
    pub venue: String,
    pub base_token: String,
    #[serde(default)]
    pub quote_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexPairOption {
    pub venue: String,
    pub base_token: String,
    pub quote_token: String,
    pub cex_symbol: String,
    pub native_symbol: String,
    pub quality: MarketDataQuality,
    pub source: MarketDataSourceKind,
    pub freshness_ms: Option<i64>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexPairCatalog {
    pub venue: String,
    pub base_token: String,
    #[serde(default)]
    pub pairs: Vec<OnchainCexPairOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}
