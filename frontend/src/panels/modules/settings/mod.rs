pub(crate) mod data;
pub(crate) mod tabs;
pub(crate) mod view;

pub(in crate::panels) use view::{select_credentials_tab, select_risk_tab, settings_module};
mod runtime;
pub(in crate::panels) use runtime::{create_settings_runtime, SettingsRuntime};
