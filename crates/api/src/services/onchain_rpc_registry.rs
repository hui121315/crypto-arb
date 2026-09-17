use shared_types::onchain_chain_preset;

use super::venue_credentials;

const RPC_SECRET_KEYS: [(&str, &str); 8] = [
    ("solana", "ONCHAIN_RPC_SOLANA_URL"),
    ("ethereum", "ONCHAIN_RPC_ETHEREUM_URL"),
    ("arbitrum", "ONCHAIN_RPC_ARBITRUM_URL"),
    ("base", "ONCHAIN_RPC_BASE_URL"),
    ("optimism", "ONCHAIN_RPC_OPTIMISM_URL"),
    ("polygon", "ONCHAIN_RPC_POLYGON_URL"),
    ("bnb-smart-chain", "ONCHAIN_RPC_BNB_SMART_CHAIN_URL"),
    ("avalanche", "ONCHAIN_RPC_AVALANCHE_URL"),
];

pub(crate) fn configured_url(chain: &str) -> Option<String> {
    let key = secret_key(chain)?;
    venue_credentials::secret(key).and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

pub(crate) async fn persist(chain: &str, url: &str) -> Result<(), String> {
    let key =
        secret_key(chain).ok_or_else(|| format!("不支持为链 {} 保存自定义 RPC", chain.trim()))?;
    let url = url.trim();
    if url.is_empty() {
        return Err("自定义 RPC URL 不能为空".to_owned());
    }
    venue_credentials::persist_secrets(&[(key.to_owned(), url.to_owned())])
        .await
        .map_err(|error| format!("自定义 RPC 安全存储失败：{error}"))
}

fn secret_key(chain: &str) -> Option<&'static str> {
    let normalized = chain.trim().to_ascii_lowercase();
    onchain_chain_preset(&normalized)?;
    RPC_SECRET_KEYS
        .iter()
        .find_map(|(known, key)| (*known == normalized).then_some(*key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ONCHAIN_CHAIN_PRESETS;
    use std::collections::HashSet;

    #[test]
    fn every_supported_chain_has_one_unique_static_secret_key() {
        let keys = ONCHAIN_CHAIN_PRESETS
            .iter()
            .map(|preset| secret_key(preset.id).expect("supported chain RPC key"))
            .collect::<Vec<_>>();

        assert_eq!(keys.len(), ONCHAIN_CHAIN_PRESETS.len());
        assert_eq!(
            keys.iter().copied().collect::<HashSet<_>>().len(),
            keys.len()
        );
        assert!(secret_key("unknown-chain").is_none());
    }
}
