mod components;
mod data;
mod draft;
mod format;
mod view;

pub(in crate::panels) use data::{create_onchain_runtime, OnchainRuntime};
pub(in crate::panels) use view::onchain_module;
