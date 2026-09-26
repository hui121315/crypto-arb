use serde::{Deserialize, Serialize};
use shared_types::TradingRiskStatus;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

const CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub(crate) struct TradingRuntimeConfigStore {
    path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct TradingRuntimeConfigReplay {
    pub(crate) store: TradingRuntimeConfigStore,
    pub(crate) snapshot: Option<TradingRuntimeConfigSnapshot>,
    pub(crate) problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TradingRuntimeConfigSnapshot {
    pub(crate) adapter_id: String,
    pub(crate) risk: TradingRiskStatus,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TradingRuntimeConfigCheckpoint {
    version: u32,
    runtime: TradingRuntimeConfigSnapshot,
}

#[derive(Debug, Error)]
pub(crate) enum TradingRuntimeConfigStoreError {
    #[error("trading runtime config storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("trading runtime config serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl TradingRuntimeConfigStore {
    pub(crate) fn load(path: Option<PathBuf>) -> TradingRuntimeConfigReplay {
        let store = Self { path };
        let Some(path) = store.path.as_deref() else {
            return replay(store, None, None);
        };
        match read_checkpoint(path) {
            Ok(Some(checkpoint)) if checkpoint.version == CHECKPOINT_VERSION => {
                replay(store, Some(checkpoint.runtime), None)
            }
            Ok(Some(checkpoint)) => replay(
                store,
                None,
                Some(format!(
                    "unsupported trading runtime config checkpoint version {}",
                    checkpoint.version
                )),
            ),
            Ok(None) => replay(store, None, None),
            Err(error) => replay(store, None, Some(error.to_string())),
        }
    }

    pub(crate) fn persist(
        &self,
        adapter_id: &str,
        risk: &TradingRiskStatus,
    ) -> Result<(), TradingRuntimeConfigStoreError> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let checkpoint = TradingRuntimeConfigCheckpoint {
            version: CHECKPOINT_VERSION,
            runtime: TradingRuntimeConfigSnapshot {
                adapter_id: adapter_id.to_owned(),
                risk: risk.clone(),
            },
        };
        atomic_write(path, &serde_json::to_vec_pretty(&checkpoint)?)
    }
}

fn replay(
    store: TradingRuntimeConfigStore,
    snapshot: Option<TradingRuntimeConfigSnapshot>,
    problem: Option<String>,
) -> TradingRuntimeConfigReplay {
    TradingRuntimeConfigReplay {
        store,
        snapshot,
        problem,
    }
}

fn read_checkpoint(
    path: &Path,
) -> Result<Option<TradingRuntimeConfigCheckpoint>, TradingRuntimeConfigStoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), TradingRuntimeConfigStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod browser_server;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_replays_adapter_and_risk_snapshot() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "crossline-trading-runtime-{}-{}.json",
            std::process::id(),
            common::time::now_ms()
        ));
        let store = TradingRuntimeConfigStore::load(Some(path.clone())).store;
        let mut risk = crate::services::risk_config::snapshot(&trading::RiskConfig::default());
        risk.max_order_notional = 20.0;
        risk.allowed_exchanges = vec!["bitget".to_owned(), "hyperliquid".to_owned()];
        store.persist("live", &risk)?;

        let restored = TradingRuntimeConfigStore::load(Some(path.clone()));

        assert!(restored.problem.is_none());
        assert_eq!(
            restored.snapshot,
            Some(TradingRuntimeConfigSnapshot {
                adapter_id: "live".to_owned(),
                risk,
            })
        );
        let _ = fs::remove_file(path);
        Ok(())
    }
}
