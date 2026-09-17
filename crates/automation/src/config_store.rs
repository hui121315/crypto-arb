use serde::{Deserialize, Serialize};
use shared_types::AutomatedArbitrageConfig;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum AutomationConfigStoreError {
    #[error("automation config storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("automation config serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AutomationConfigStore {
    path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct AutomationConfigReplay {
    pub store: AutomationConfigStore,
    pub config: AutomatedArbitrageConfig,
    pub problem: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutomationConfigCheckpoint {
    version: u32,
    config: AutomatedArbitrageConfig,
}

impl AutomationConfigStore {
    #[must_use]
    pub fn load(path: Option<PathBuf>) -> AutomationConfigReplay {
        let store = Self { path };
        let Some(path) = store.path.as_deref() else {
            return AutomationConfigReplay {
                store,
                config: AutomatedArbitrageConfig::default(),
                problem: None,
            };
        };
        match read_checkpoint(path) {
            Ok(Some(checkpoint)) if checkpoint.version == CHECKPOINT_VERSION => {
                AutomationConfigReplay {
                    store,
                    config: checkpoint.config,
                    problem: None,
                }
            }
            Ok(Some(checkpoint)) => AutomationConfigReplay {
                store,
                config: AutomatedArbitrageConfig::default(),
                problem: Some(format!(
                    "unsupported automation config checkpoint version {}",
                    checkpoint.version
                )),
            },
            Ok(None) => AutomationConfigReplay {
                store,
                config: AutomatedArbitrageConfig::default(),
                problem: None,
            },
            Err(error) => AutomationConfigReplay {
                store,
                config: AutomatedArbitrageConfig::default(),
                problem: Some(error.to_string()),
            },
        }
    }

    pub fn persist(
        &self,
        config: &AutomatedArbitrageConfig,
    ) -> Result<(), AutomationConfigStoreError> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let checkpoint = AutomationConfigCheckpoint {
            version: CHECKPOINT_VERSION,
            config: config.clone(),
        };
        atomic_write(path, &serde_json::to_vec_pretty(&checkpoint)?)
    }
}

fn read_checkpoint(
    path: &Path,
) -> Result<Option<AutomationConfigCheckpoint>, AutomationConfigStoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AutomationConfigStoreError> {
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
    use shared_types::{
        ExecutionEnvironment, DEFAULT_AUTOMATION_CAPITAL_USD, DEFAULT_AUTOMATION_MIN_DEPTH_USD,
    };

    #[test]
    fn missing_checkpoint_uses_small_calibration_defaults() {
        let replay = AutomationConfigStore::load(None);

        assert_eq!(replay.config.capital_usd, DEFAULT_AUTOMATION_CAPITAL_USD);
        assert_eq!(
            replay.config.min_depth_usd,
            DEFAULT_AUTOMATION_MIN_DEPTH_USD
        );
        assert!(replay.problem.is_none());
    }

    #[test]
    fn persists_and_replays_full_config() -> Result<(), AutomationConfigStoreError> {
        let path = std::env::temp_dir().join(format!(
            "crossline-automation-config-{}-{}.json",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let replay = AutomationConfigStore::load(Some(path.clone()));
        let mut config = replay.config;
        config.enabled = true;
        config.paused = true;
        config.environment = ExecutionEnvironment::Live;
        config.capital_usd = 10.0;
        config.min_depth_usd = 10.0;
        replay.store.persist(&config)?;

        let restored = AutomationConfigStore::load(Some(path.clone()));

        assert_eq!(restored.config, config);
        assert!(restored.problem.is_none());
        let _ = fs::remove_file(path);
        Ok(())
    }
}
