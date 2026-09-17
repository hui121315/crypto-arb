pub mod config_store;
pub mod controller;
pub mod selector;

pub use config_store::{AutomationConfigReplay, AutomationConfigStore, AutomationConfigStoreError};
pub use controller::{AutomationController, AutomationError};
pub use selector::{
    select_candidate, select_candidates, select_monitor_candidates, CandidateSelection,
};
