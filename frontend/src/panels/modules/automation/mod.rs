mod components;
mod data;
mod draft;
mod format;
mod health;
mod protection_calibration;
mod receipts;
mod view;

pub(in crate::panels) use data::{create_automation_runtime, AutomationRuntime};
pub(in crate::panels) use view::automation_module;
