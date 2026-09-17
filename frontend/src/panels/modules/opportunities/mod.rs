pub(crate) mod components;
pub(crate) mod data;
pub(crate) mod view;

pub(in crate::panels) use data::{create_opportunities_runtime, OpportunitiesRuntime};
pub(in crate::panels) use view::opportunities_module;
