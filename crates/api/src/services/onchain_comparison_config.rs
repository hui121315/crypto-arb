use serde::{Deserialize, Serialize};
use shared_types::OnchainComparisonConfig;
#[cfg(test)]
use shared_types::OnchainRpcMode;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub(crate) struct OnchainComparisonConfigStore {
    path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct OnchainComparisonConfigReplay {
    pub(crate) store: OnchainComparisonConfigStore,
    pub(crate) config: Option<OnchainComparisonConfig>,
    pub(crate) batch_configs: Vec<OnchainComparisonConfig>,
    pub(crate) problem: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint {
    version: u32,
    config: OnchainComparisonConfig,
    #[serde(default)]
    batch_configs: Vec<OnchainComparisonConfig>,
}

#[derive(Debug, Error)]
pub(crate) enum OnchainComparisonConfigStoreError {
    #[error("链上套利配置存储失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("链上套利配置序列化失败: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl OnchainComparisonConfigStore {
    pub(crate) fn load(path: Option<PathBuf>) -> OnchainComparisonConfigReplay {
        let store = Self { path };
        let Some(path) = store.path.as_deref() else {
            return replay(store, None, Vec::new(), None);
        };
        match read_checkpoint(path) {
            Ok(Some(checkpoint)) if checkpoint.version == CHECKPOINT_VERSION => replay(
                store,
                Some(safe_replay_config(checkpoint.config)),
                checkpoint
                    .batch_configs
                    .into_iter()
                    .map(safe_batch_replay_config)
                    .collect(),
                None,
            ),
            Ok(Some(checkpoint)) => replay(
                store,
                None,
                Vec::new(),
                Some(format!(
                    "unsupported on-chain comparison checkpoint version {}",
                    checkpoint.version
                )),
            ),
            Ok(None) => replay(store, None, Vec::new(), None),
            Err(error) => replay(store, None, Vec::new(), Some(error.to_string())),
        }
    }

    pub(crate) fn persist(
        &self,
        config: &OnchainComparisonConfig,
        batch_configs: &[OnchainComparisonConfig],
    ) -> Result<(), OnchainComparisonConfigStoreError> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let checkpoint = Checkpoint {
            version: CHECKPOINT_VERSION,
            config: safe_replay_config(config.clone()),
            batch_configs: batch_configs
                .iter()
                .cloned()
                .map(safe_batch_replay_config)
                .collect(),
        };
        atomic_write(path, &serde_json::to_vec_pretty(&checkpoint)?)
    }
}

fn safe_replay_config(mut config: OnchainComparisonConfig) -> OnchainComparisonConfig {
    // The URL is stored by the secret backend; the checkpoint only keeps the mode.
    migrate_legacy_identity_alias(&mut config.base_token, &mut config.base_identity_resolved);
    migrate_legacy_identity_alias(&mut config.quote_token, &mut config.quote_identity_resolved);
    config
}

fn safe_batch_replay_config(mut config: OnchainComparisonConfig) -> OnchainComparisonConfig {
    migrate_legacy_identity_alias(&mut config.base_token, &mut config.base_identity_resolved);
    migrate_legacy_identity_alias(&mut config.quote_token, &mut config.quote_identity_resolved);
    config.enabled = true;
    config
}

fn migrate_legacy_identity_alias(token: &mut String, identity_resolved: &mut bool) {
    let Some(alias) = token.strip_prefix("UNVERIFIED:") else {
        return;
    };
    *token = alias.trim().to_owned();
    *identity_resolved = false;
}

fn replay(
    store: OnchainComparisonConfigStore,
    config: Option<OnchainComparisonConfig>,
    batch_configs: Vec<OnchainComparisonConfig>,
    problem: Option<String>,
) -> OnchainComparisonConfigReplay {
    OnchainComparisonConfigReplay {
        store,
        config,
        batch_configs,
        problem,
    }
}

fn read_checkpoint(path: &Path) -> Result<Option<Checkpoint>, OnchainComparisonConfigStoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), OnchainComparisonConfigStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, bytes)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_custom_rpc_mode_without_a_custom_rpc_secret() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "crossline-onchain-config-{}-{}.json",
            std::process::id(),
            common::time::now_ms()
        ));
        let store = OnchainComparisonConfigStore::load(Some(path.clone())).store;
        let mut config = OnchainComparisonConfig::default();
        config.enabled = true;
        config.provider = "jupiter_swap_v2_keyed".to_owned();
        config.pool_or_route = "jupiter-api-key".to_owned();
        config.rpc.mode = OnchainRpcMode::Custom;

        let mut watched = config.clone();
        watched.base_token = "WIF".to_owned();
        watched.base_mint = "wif-mint".to_owned();
        watched.cex_symbol = "WIF/USDC".to_owned();
        store.persist(&config, &[watched])?;
        let restored = OnchainComparisonConfigStore::load(Some(path.clone()));

        assert!(restored.problem.is_none());
        let restored = restored.config.expect("restored config");
        assert!(restored.enabled);
        assert_eq!(restored.provider, "jupiter_swap_v2_keyed");
        assert_eq!(restored.rpc.mode, OnchainRpcMode::Custom);
        assert_eq!(
            OnchainComparisonConfigStore::load(Some(path.clone())).batch_configs[0]
                .rpc
                .mode,
            OnchainRpcMode::Custom
        );
        let contents = fs::read_to_string(&path)?;
        assert!(!contents.contains("http"));
        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn migrates_legacy_unverified_alias_into_structured_identity_state() {
        let mut config = OnchainComparisonConfig::default();
        config.base_token = "UNVERIFIED:PUPS".to_owned();

        let migrated = safe_replay_config(config);

        assert_eq!(migrated.base_token, "PUPS");
        assert!(!migrated.base_identity_resolved);
    }

    #[test]
    fn loads_a_legacy_checkpoint_without_a_batch_field() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "crossline-onchain-config-legacy-{}-{}.json",
            std::process::id(),
            common::time::now_ms()
        ));
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": CHECKPOINT_VERSION,
                "config": OnchainComparisonConfig::default(),
            }))?,
        )?;

        let restored = OnchainComparisonConfigStore::load(Some(path.clone()));

        assert!(restored.problem.is_none());
        assert!(restored.config.is_some());
        assert!(restored.batch_configs.is_empty());
        let _ = fs::remove_file(path);
        Ok(())
    }
}
