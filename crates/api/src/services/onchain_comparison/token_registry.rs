use super::jupiter_quota;
use super::provider_runtime::env_key;
use super::quote::{decode_jupiter_general_response, quote_client};
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const JUPITER_TOKEN_SEARCH_ENDPOINT: &str = "https://api.jup.ag/tokens/v2/search";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterTokenMetadata {
    id: String,
    name: String,
    symbol: String,
    decimals: u8,
    #[serde(default)]
    is_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedTokenIdentity {
    pub(super) symbol: String,
    pub(super) address: String,
    pub(super) decimals: u8,
    pub(super) name: Option<String>,
    pub(super) verified: bool,
}

pub(super) async fn resolve_jupiter_token_identities(
    symbols: &[&str],
) -> Result<Vec<ResolvedTokenIdentity>, String> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }
    let cache = token_identity_cache();
    let missing = symbols
        .iter()
        .map(|symbol| symbol.trim().to_ascii_uppercase())
        .filter(|symbol| !cache.contains_key(symbol))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let query = missing.join(",");
        let rows = fetch_token_rows(&query).await?;
        for symbol in &missing {
            let exact = rows
                .iter()
                .filter(|row| row.is_verified && row.symbol.eq_ignore_ascii_case(symbol))
                .collect::<Vec<_>>();
            if exact.len() != 1 {
                return Err(format!(
                    "Jupiter 官方 token registry 无法唯一解析 {symbol}；请填写 mint 与精度"
                ));
            }
            let row = exact[0];
            cache.insert(
                symbol.clone(),
                ResolvedTokenIdentity {
                    symbol: row.symbol.to_ascii_uppercase(),
                    address: row.id.clone(),
                    decimals: row.decimals,
                    name: Some(row.name.clone()),
                    verified: row.is_verified,
                },
            );
        }
    }
    symbols
        .iter()
        .map(|symbol| {
            cache
                .get(&symbol.trim().to_ascii_uppercase())
                .map(|entry| entry.value().clone())
                .ok_or_else(|| format!("Jupiter token identity missing for {symbol}"))
        })
        .collect()
}

pub(super) async fn resolve_jupiter_token_identity(
    address: &str,
) -> Result<ResolvedTokenIdentity, String> {
    let cache_key = format!("mint:{address}");
    if let Some(identity) = token_identity_cache().get(&cache_key) {
        return Ok(identity.value().clone());
    }
    let rows = fetch_token_rows(address).await?;
    let exact = rows
        .iter()
        .filter(|row| row.id == address)
        .collect::<Vec<_>>();
    if exact.len() != 1 {
        return Err("Jupiter token registry 无法唯一解析该 mint".to_owned());
    }
    let row = exact[0];
    let identity = ResolvedTokenIdentity {
        symbol: row.symbol.to_ascii_uppercase(),
        address: row.id.clone(),
        decimals: row.decimals,
        name: Some(row.name.clone()),
        verified: row.is_verified,
    };
    token_identity_cache().insert(cache_key, identity.clone());
    Ok(identity)
}

async fn fetch_token_rows(query: &str) -> Result<Vec<JupiterTokenMetadata>, String> {
    let api_key = env_key("JUPITER_API_KEY");
    jupiter_quota::wait_for_general_request(api_key.is_some()).await;
    let mut request = quote_client()
        .get(JUPITER_TOKEN_SEARCH_ENDPOINT)
        .query(&[("query", query)]);
    if let Some(api_key) = api_key.as_deref() {
        request = request.header("x-api-key", api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Jupiter token search failed: {error}"))?;
    let body = decode_jupiter_general_response(response, api_key.is_some(), "Jupiter token search")
        .await?;
    serde_json::from_str(&body)
        .map_err(|error| format!("Jupiter token search decode failed: {error}"))
}

fn token_identity_cache() -> &'static dashmap::DashMap<String, ResolvedTokenIdentity> {
    static CACHE: OnceLock<dashmap::DashMap<String, ResolvedTokenIdentity>> = OnceLock::new();
    CACHE.get_or_init(dashmap::DashMap::new)
}
